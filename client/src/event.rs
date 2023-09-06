use std::{collections::HashMap, sync::Arc};

use crate::{
    client::{Client, SepResponse},
    time::current_time,
};
use anyhow::{anyhow, Result};
use common::packages::{
    identification::{ResponseRequired, ResponseStatus},
    objects::{Dercontrol, EndDeviceControl, EventStatusType, TextMessage, TimeTariffInterval},
    primitives::HexBinary160,
    response::DercontrolResponse,
    traits::{SEEvent, SERespondableResource},
    types::{Mridtype, OneHourRangeType, PrimacyType},
    xsd::{EndDevice, FlowReservationResponse},
};
use log::error;
use rand::Rng;
use tokio::sync::RwLock;

struct EventInstance<E: SEEvent> {
    start: i64,
    end: i64,
    primacy: PrimacyType,
    status: EventStatusType,
    event: E,
}

impl EventInstance<Dercontrol> {
    pub fn der_supersedes(&self, other: &Self) -> bool {
        // TODO: Confirm this is correct
        if self.primacy == other.primacy && self.event.creation_time() > other.event.creation_time()
            || self.primacy < other.primacy
        {
            self.event
                .der_control_base
                .same_target(&other.event.der_control_base)
        } else {
            false
        }
    }
}

impl<E: SEEvent> EventInstance<E> {
    pub fn new(event: E, primacy: PrimacyType) -> Self {
        let start: i64 = event.interval().start.get();
        let end: i64 = start + i64::from(event.interval().duration.get());
        EventInstance {
            status: event.event_status().current_status,
            event,
            primacy,
            start,
            end,
        }
    }

    pub fn new_rand(
        primacy: PrimacyType,
        rand_duration: Option<OneHourRangeType>,
        rand_start: Option<OneHourRangeType>,
        event: E,
    ) -> Self {
        let start: i64 = event.interval().start.get() + randomize(rand_duration);
        let end: i64 = start + i64::from(event.interval().duration.get()) + randomize(rand_start);
        EventInstance {
            status: event.event_status().current_status,
            event,
            primacy,
            start,
            end,
        }
    }

    pub fn supersedes(&self, other: &Self) -> bool {
        self.primacy == other.primacy && self.event.creation_time() > other.event.creation_time()
            || self.primacy < other.primacy
    }

    pub fn update_primacy(&mut self, primacy: PrimacyType) {
        self.primacy = primacy;
    }
}

fn randomize(bound: Option<OneHourRangeType>) -> i64 {
    bound.map_or(0, |val| {
        let val = val.get();
        let mut rng = rand::thread_rng();
        let sign = val.signum() as i64;
        rng.gen_range(0..=val.abs().into()) * sign
    })
}

pub struct Schedule<E: SEEvent> {
    client: Client,
    // Send + Sync end device, as the EndDevice resource may be updated
    device: Arc<RwLock<EndDevice>>,
    // Lookup by MRID
    events: HashMap<Mridtype, Arc<RwLock<EventInstance<E>>>>,
}

impl<E: SEEvent> Schedule<E> {
    /// Create a schedule for the given client & it's EndDevice representation
    pub fn new(client: Client, device: Arc<RwLock<EndDevice>>) -> Self {
        Schedule {
            client,
            device,
            events: HashMap::new(),
        }
    }
}

// Distributed Energy Resources Function Set
impl Schedule<Dercontrol> {
    pub async fn schedule_event(&mut self, event: Dercontrol, primacy: PrimacyType) {
        let mrid = event.mrid;
        let ev = self.events.get_mut(&mrid);
        let incoming_status = event.event_status.current_status;

        // If the event already exists in the schedule
        let ei = if let Some(ei) = ev {
            // "Editing events shall NOT be allowed, except for updating status"
            // ev.primacy = primacy;
            ei.clone()
        } else {
            // Inform server event was scheduled
            self.send_der_resp(&event, ResponseStatus::EventReceived)
                .await;
            // Calculate start & end times
            let ei = EventInstance::new_rand(
                primacy,
                event.randomize_duration,
                event.randomize_start,
                event,
            );
            // The event may have expired already
            if ei.end <= current_time().get() {
                // Do not add event to schedule
                self.send_der_resp(&ei.event, ResponseStatus::EventExpired)
                    .await;
                return;
            }
            // Add it to our schedule
            let ei = Arc::new(RwLock::new(ei));
            self.events.insert(mrid, ei.clone());
            ei
        };

        // Handle status transitions
        let current_status = ei.read().await.status;
        match (current_status, incoming_status) {
            // (Cancelled | CancelledRandom | Superseded) -> Any
            (
                EventStatusType::Cancelled
                | EventStatusType::CancelledRandom
                | EventStatusType::Superseded,
                _,
            ) => (),
            // Scheduled -> (Cancelled || CancelledRandom)
            (
                EventStatusType::Scheduled,
                s @ (EventStatusType::Cancelled | EventStatusType::CancelledRandom),
            ) => {
                // Respond EventCancelled
                self.send_der_resp(&ei.read().await.event, s.into()).await;
                // TODO: What do we need to keep the cancelled event for?
                self.events.remove(&mrid);
            }
            // Scheduled -> Active
            (EventStatusType::Scheduled, EventStatusType::Active) => todo!(),
            // Active -> Superseded
            (EventStatusType::Active, EventStatusType::Superseded) => todo!(),
            // Scheduled -> Superseded
            (EventStatusType::Scheduled, EventStatusType::Superseded) => todo!(),
            // Active -> Scheduled
            (EventStatusType::Active, EventStatusType::Scheduled) => todo!(),
            // Active -> Cancelled
            (EventStatusType::Active, EventStatusType::Cancelled) => todo!(),
            // Active -> CancelledRandom
            (EventStatusType::Active, EventStatusType::CancelledRandom) => todo!(),
            // Active -> Active
            (EventStatusType::Active, EventStatusType::Active) => todo!(),
            // Scheduled -> Scheduled
            (EventStatusType::Scheduled, EventStatusType::Scheduled) => todo!(),
        }
    }

    // Schedule a new event, logging if it fails
    async fn send_der_resp(&self, event: &Dercontrol, status: ResponseStatus) {
        if let Some(lfdi) = { self.device.read().await.lfdi } {
            let _ = self
                .send_response(lfdi, event, status)
                .await
                .map_err(|e| error!("DER response POST attempt failed with reason: {}", e));
        } else {
            error!("Attempted to send DER response for EndDevice that does not have an LFDI")
        }
    }

    async fn send_response(
        &self,
        lfdi: HexBinary160,
        event: &Dercontrol,
        status: ResponseStatus,
    ) -> Result<SepResponse> {
        if matches!(status, ResponseStatus::EventReceived)
            && event
                .response_required
                .map(|rr| {
                    rr.contains(
                        ResponseRequired::MessageReceived | ResponseRequired::SpecificResponse,
                    )
                })
                .ok_or(anyhow!("Event does not contain a ResponseRequired field"))?
        {
            let resp = DercontrolResponse {
                created_date_time: Some(current_time()),
                end_device_lfdi: lfdi,
                status: Some(status),
                subject: event.mrid,
                href: None,
            };
            self.client
                .post(
                    event
                        .reply_to()
                        .ok_or(anyhow!("Event does not contain a ReplyTo field"))?,
                    &resp,
                )
                .await
        } else {
            Err(anyhow!(
                "Attempted to send a response for an event that did not require one."
            ))
        }
    }

    async fn start_event(&mut self, mrid: &Mridtype) {
        // Guaranteed to exist, remove to avoid double mut borrow
        let target_event = self.events.remove(mrid).unwrap();
        let mut superseded: Vec<Mridtype> = vec![];
        // Mark required events as superseded
        for (mrid, ei) in &mut self.events {
            let ei = &mut *ei.write().await;
            if (target_event.read().await).der_supersedes(ei) {
                // Notify client active event has ended
                if matches!(ei.status, EventStatusType::Active) {
                    // TODO: Callback w/ EVENT END
                }
                ei.status = EventStatusType::Superseded;
                superseded.push(*mrid);
            }
        }
        // Notify server
        for mrid in superseded {
            let event = &self.events.get(&mrid).unwrap().read().await.event;
            self.send_der_resp(event, ResponseStatus::EventSuperseded)
                .await;
        }
        target_event.write().await.status = EventStatusType::Active;
        // TODO: Callback W/ EVENT START
        self.events.insert(*mrid, target_event);
    }

    // Events may be cancelled before they complete.
    async fn cancel_event(&mut self, mrid: &Mridtype) {}
}

// Demand Response Load Control Function Set
impl Schedule<EndDeviceControl> {}

// Messaging Function Set
impl Schedule<TextMessage> {}

// Flow Reservation Function Set
impl Schedule<FlowReservationResponse> {}

// Pricing Function Set
impl Schedule<TimeTariffInterval> {}
