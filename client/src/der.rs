use std::sync::Arc;

use sep2_common::{
    packages::{
        der::DERControl,
        identification::ResponseStatus,
        objects::EventStatusType as EventStatus,
        types::{MRIDType, PrimacyType}, edev::EndDevice,
    },
    traits::SEEvent,
};
use tokio::sync::RwLock;

use crate::{
    event::{EIStatus, EventHandler, EventInstance, Schedule, Events},
    time::{current_time, SLEEP_TICKRATE}, client::Client,
};

impl EventInstance<DERControl> {
    /// Determine whether one DERControl supersedes another
    pub(crate) fn der_supersedes(&self, other: &Self) -> bool {
        // If there is any overlap
        if self.start_time() <= other.end_time() && self.end_time() >= other.start_time() {
            if self.primacy() == other.primacy()
                && self.event().creation_time() > other.event().creation_time()
                || self.primacy() < other.primacy()
            {
                return self.event()
                    .der_control_base
                    .same_target(&other.event().der_control_base);
            }
        }
        false
    }
}

impl<H: EventHandler<DERControl>> Schedule<DERControl, H> {
    /// Create a schedule for the given [`Client`] & it's [`EndDevice`] representation.
    ///
    /// Any instance of [`Client`] can be used, as responses are made in accordance to the hostnames within the provided events.
    pub fn new(client: Client, device: Arc<RwLock<EndDevice>>, handler: H) -> Self {
        let out = Schedule {
            client,
            device,
            events: Arc::new(RwLock::new(Events::new())),
            handler: Arc::new(handler),
        };
        // TODO: Add a way to kill tasks
        tokio::spawn(out.clone().clean_events());
        tokio::spawn(out.clone().der_start_task());
        tokio::spawn(out.clone().der_end_task());
        out
    }

    /// Add a [`DERControl`] Event to the schedule.
    /// Subsequent retrievals/notifications of any and all [`DERControl`] resources should call this function.
    pub async fn add_dercontrol(&mut self, event: DERControl, primacy: PrimacyType) {
        let mrid = event.mrid;
        let incoming_status = event.event_status.current_status;

        // If the event already exists in the schedule
        if self.events.read().await.contains(&mrid) {
            // "Editing events shall NOT be allowed, except for updating status"
            let current_status = {
                let events = self.events.read().await;
                let ei = events.get(&mrid).unwrap();
                ei.status()
            };
            match (current_status, incoming_status) {
                // Active -> (Cancelled || CancelledRandom || Superseded)
                (EIStatus::Active, EventStatus::Cancelled | EventStatus::CancelledRandom | EventStatus::Superseded) => {
                    log::warn!("DERControlSchedule: DERControl ({mrid}) has been marked as superseded by the server, yet it is active locally. The event will be cancelled");
                    self.cancel_dercontrol(&mrid, incoming_status.into()).await;
                },
                // Scheduled -> (Cancelled || CancelledRandom)
                (EIStatus::Scheduled, EventStatus::Cancelled | EventStatus::CancelledRandom) => {
                    log::info!("DERControlSchedule: DERControl ({mrid} has been marked as cancelled by the server. It will not be started");
                    self.cancel_dercontrol(&mrid, incoming_status.into()).await;
                },
                // Scheduled -> Active
                (EIStatus::Scheduled, EventStatus::Active) => {
                    log::info!("DERControlSchedule: DERControl ({mrid}) has entered it's earliest effective start time.")
                }
                // Scheduled -> Superseded
                (EIStatus::Scheduled, EventStatus::Superseded) => 
                    log::warn!("DERControlSchedule: DERControl ({mrid}) has been marked as superseded by the server, yet it is not locally."),
                // Active -> Scheduled
                (EIStatus::Active, EventStatus::Scheduled) => 
                    log::warn!("DERControlSchedule: DERControl ({mrid}) is active locally, and scheduled on the server. Is the client clock ahead?"),
                // Active -> Active
                (EIStatus::Active, EventStatus::Active) => (),
                // Complete -> Any
                (EIStatus::Complete, _) => (),
                // (Cancelled | CancelledRandom | Superseded) -> Any
                (EIStatus::Cancelled | EIStatus::CancelledRandom | EIStatus::Superseded, _) => (),
                // Scheduled -> Scheduled
                (EIStatus::Scheduled, EventStatus::Scheduled) => (),
            }
        } else {
            // We intentionally hold this lock for this entire scope
            let mut events = self.events.write().await;
            // Inform server event was received
            self.client
                .send_der_response(
                    self.device.read().await.lfdi,
                    &event,
                    ResponseStatus::EventReceived,
                )
                .await;

            // Event arrives cancelled or superseded
            if matches!(incoming_status, EventStatus::Cancelled | EventStatus::CancelledRandom | EventStatus::Superseded) {
                log::warn!("DERControlSchedule: Told to schedule DERControl ({mrid}) which is already {:?}, sending server response and not scheduling.", incoming_status);
                    self.client
                    .send_der_response(
                        self.device.read().await.lfdi,
                        &event,
                        der_status_response(incoming_status),
                    )
                    .await;
                return;
            }

            // Calculate start & end times
            let ei = EventInstance::new_rand(
                primacy,
                event.randomize_duration,
                event.randomize_start,
                event,
            );

            // The event may have expired already
            if ei.end_time() <= current_time().get() {
                log::warn!("DERControlSchedule: Told to schedule DERControl ({mrid}) which has already ended, sending server response and not scheduling.");
                // Do not add event to schedule
                self.client
                    .send_der_response(
                        self.device.read().await.lfdi,
                        ei.event(),
                        ResponseStatus::EventExpired,
                    )
                    .await;
                return;
            }

            // For each event that would be superseded by this event starting:
            // - inform the client
            // - inform the server
            // - mark as superseded

            // Determine what events this supersedes
            let target = ei;
            let mut superseded = vec![];
            for (o_mrid, other) in events.iter() {
                if target.der_supersedes(other) {
                    if other.status() == EIStatus::Active {
                        // Since the newly superseded event is over, tell the client it's finished
                        (self.handler)
                            .event_update(&other, EIStatus::Superseded)
                            .await;
                    }
                    superseded.push(*o_mrid);
                    // Tell the server we've been informed this event is superseded
                    self.client
                        .send_der_response(
                            self.device.read().await.lfdi,
                            other.event(),
                            ResponseStatus::EventSuperseded,
                        )
                        .await;
                }
            }

            // Update internal status
            events.update_events(&superseded, EIStatus::Superseded);

            // Add it to our schedule
            events.insert(&mrid, target);
        };
    }

    async fn der_start_task(self) {
        loop {
            // Intermittently sleep until next event start time
            tokio::time::sleep(SLEEP_TICKRATE).await;
            let mut events = self.events.write().await;
            let mrid = match events.next_start() {
                Some((time, mrid)) if time < current_time().get() => mrid,
                // If no next, or not time yet
                _ => continue,
            };

            // Mark event as complete
            events.update_event(&mrid, EIStatus::Active);

            // Notify client and server
            let target = events.get(&mrid).unwrap();
            let resp = self.handler.event_update(target, EIStatus::Active).await;
            self.client.send_der_response(self.device.read().await.lfdi, target.event(), resp).await;
        }
    }

    async fn der_end_task(self) {
        loop {
            // Intermittently sleep until next event end time
            tokio::time::sleep(SLEEP_TICKRATE).await;
            let mut events = self.events.write().await;
            let mrid = match events.next_end() {
                Some((time, mrid)) if time < current_time().get() => mrid,
                // If no next, or not time yet
                _ => continue,
            };

            // Mark event as complete
            events.update_event(&mrid,EIStatus::Complete);

            // Notify client and server
            let target = events.get(&mrid).unwrap();
            let resp = self.handler.event_update(target, EIStatus::Complete).await;
            self.client.send_der_response(self.device.read().await.lfdi, target.event(), resp).await;
        }
    }


    /// Cancel an [`EventInstance<DerControl>`] that has been previously added to the schedule
    ///
    /// Update the internal [`EventInstance<DerControl>`]
    ///
    /// `cancel_reason` must/will be one of [`EIStatus::Cancelled`] | [`EIStatus::CancelledRandom`] | [`EIStatus::Superseded`]
    async fn cancel_dercontrol(&mut self, target_mrid: &MRIDType, cancel_reason: EIStatus) {
        let mut events = self.events.write().await;
        events.update_event(target_mrid, cancel_reason);

        let ei = events.get(&target_mrid).unwrap();
        let resp = (self.handler).event_update(&ei, cancel_reason).await;
        self.client
            .send_der_response(self.device.read().await.lfdi, ei.event(), resp)
            .await;
    }
}

/// Given the status of an Event, convert it to a ResponseStatus, as per the DER FS
pub fn der_status_response(e: EventStatus) -> ResponseStatus {
    match e {
        EventStatus::Scheduled => ResponseStatus::EventReceived,
        EventStatus::Active => ResponseStatus::EventStarted,
        EventStatus::Cancelled => ResponseStatus::EventCancelled,
        EventStatus::CancelledRandom => ResponseStatus::EventCancelled,
        EventStatus::Superseded => ResponseStatus::EventSuperseded,
    }
}

/// Given the status of an EventInstance, convert it to a ResponseStatus, as per the DER FS
pub fn der_ei_status_response(e: EIStatus) -> ResponseStatus {
    match e {
        EIStatus::Scheduled => ResponseStatus::EventReceived,
        EIStatus::Active => ResponseStatus::EventStarted,
        EIStatus::Cancelled => ResponseStatus::EventCancelled,
        EIStatus::Complete => ResponseStatus::EventCompleted,
        EIStatus::CancelledRandom => ResponseStatus::EventCancelled,
        EIStatus::Superseded => ResponseStatus::EventSuperseded,
    }
}
