use std::{
    future::Future,
    sync::Arc,
    time::{Duration, SystemTime},
};

use common::{
    packages::{
        identification::ResponseStatus,
        objects::{DERControl, EventStatusType as EventStatus},
        types::{MRIDType, PrimacyType},
    },
    traits::SEEvent,
};
use tokio::sync::RwLock;

use crate::{
    event::{EIStatus, EventInstance, Schedule},
    time::current_time,
};

impl EventInstance<DERControl> {
    /// Determine whether one DERControl supersedes another
    pub(crate) fn der_supersedes(&self, other: &Self) -> bool {
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

impl<F, Res> Schedule<DERControl, F, Res>
where
    F: FnMut(Arc<RwLock<EventInstance<DERControl>>>, EIStatus) -> Res
        + Send
        + Sync
        + Clone
        + 'static,
    Res: Future<Output = ResponseStatus> + Send,
{
    /// Add a [`DERControl`] Event to the schedule.
    /// Subsequent retrievals/notifications of any and all [`DERControl`] resources should call this function.
    pub async fn add_dercontrol(&mut self, event: DERControl, primacy: PrimacyType) {
        let mrid = event.mrid;
        let ev = self.events.get_mut(&mrid);
        let incoming_status = event.event_status.current_status;

        // If the event already exists in the schedule
        let ei = if let Some(ei) = ev {
            // "Editing events shall NOT be allowed, except for updating status"
            ei.clone()
        } else {
            // Inform server event was scheduled
            self.client
                .send_der_response(
                    self.device.read().await.lfdi,
                    &event,
                    ResponseStatus::EventReceived,
                )
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
                log::warn!("DERControlSchedule: Told to schedule DERControl ({mrid}) which has already ended, sending server response and not scheduling.");
                // Do not add event to schedule
                self.client
                    .send_der_response(
                        self.device.read().await.lfdi,
                        &ei.event,
                        ResponseStatus::EventExpired,
                    )
                    .await;
                return;
            }
            // Add it to our schedule
            let ei = Arc::new(RwLock::new(ei));
            self.events.insert(mrid, ei.clone());
            ei
        };

        let current_status = ei.read().await.status;
        // Handle status transitions
        // TODO: Determine when the currently superseded events need to be reevaluated
        match (current_status, incoming_status) {
            // Active -> Active - Do nothing
            (EIStatus::Active, EventStatus::Active) => (),
            // Complete -> Any - Do nothing
            (EIStatus::Complete, _) => (),
            // (Cancelled | CancelledRandom | Superseded) -> Any - Do nothing
            (EIStatus::Cancelled | EIStatus::CancelledRandom | EIStatus::Superseded, _) => (),
            // Scheduled -> Scheduled - Start wait task
            (EIStatus::Scheduled, EventStatus::Scheduled) => self.schedule_dercontrol(&mrid).await,
            // Scheduled -> (Cancelled || CancelledRandom) - Respond EventCancelled
            (EIStatus::Scheduled, EventStatus::Cancelled | EventStatus::CancelledRandom) => {
                self.client
                    .send_der_response(
                        self.device.read().await.lfdi,
                        &ei.read().await.event,
                        incoming_status.into(),
                    )
                    .await;
            }
            // Scheduled -> Active
            (EIStatus::Scheduled, EventStatus::Active) => self.start_dercontrol(&mrid).await,
            // (Active | Scheduled) -> Superseded
            (EIStatus::Active | EIStatus::Scheduled, EventStatus::Superseded) => {
                self.cancel_dercontrol(&mrid, incoming_status.into()).await
            }
            // Active -> (Cancelled || CancelledRandom)
            (EIStatus::Active, EventStatus::Cancelled | EventStatus::CancelledRandom) => {
                self.cancel_dercontrol(&mrid, incoming_status.into()).await
            }
            // Active -> Scheduled
            (EIStatus::Active, EventStatus::Scheduled) => todo!("Is this transition possible?"),
        }
    }

    /// Start an [`EventInstance<DerControl>`] that is not [`EIStatus::Active`].
    ///
    /// Mark all events that this event supersedes as superseded.
    /// Inform both the server, and the client itself of these state changes.
    /// Create a task that ends the event at the correct time, can be overriden.
    ///
    /// `mrid` must be the key to a previously scheduled [`EventInstance<DerControl>`].
    async fn start_dercontrol(&mut self, mrid: &MRIDType) {
        // Guaranteed to exist, avoid double mut borrow
        let target_ei = self.events.remove(mrid).unwrap();
        let mut superseded: Vec<MRIDType> = vec![];
        // Mark required events as superseded
        for (mrid, ei) in &mut self.events {
            let ei_w = &mut *ei.write().await;
            if (target_ei.read().await).der_supersedes(ei_w) {
                if ei_w.status == EIStatus::Active {
                    // Since the event is active, the client needs to be told the event is over
                    (self.callback)(ei.clone(), EIStatus::Superseded);
                }
                ei_w.status = EIStatus::Superseded;
                superseded.push(*mrid);
            }
        }
        // Notify server
        for mrid in superseded {
            let event = &self.events.get(&mrid).unwrap().read().await.event;
            self.client
                .send_der_response(
                    self.device.read().await.lfdi,
                    event,
                    ResponseStatus::EventSuperseded,
                )
                .await;
        }
        // Update event status
        target_ei.write().await.status = EIStatus::Active;

        // Setup wait task
        let wait_duration = (target_ei.read().await.end
            - (SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or(Duration::ZERO)
                .as_secs() as i64))
            .max(0) as u64;
        let mut callback = self.callback.clone();
        let client = self.client.clone();
        let lfdi = self.device.read().await.lfdi;
        let ei = target_ei.clone();

        // Start waiting for end of event
        let _ = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(wait_duration)).await;
            // If it's Active, then it hasn't been cancelled
            if ei.read().await.status == EIStatus::Active {
                ei.write().await.status = EIStatus::Complete;
                // Defer to client callback for ResponseStatus
                let resp = callback(ei.clone(), EIStatus::Complete).await;
                // Inform server
                client
                    .send_der_response(lfdi, &ei.read().await.event, resp)
                    .await;
            }
        });

        // Inform client of event start
        (self.callback)(target_ei.clone(), EIStatus::Active);
        // Store event
        self.events.insert(*mrid, target_ei);
    }

    /// Schedule an [`EventInstance<DerControl>`] that has been previously added to the schedule, such that it begins at it's scheduled start time
    async fn schedule_dercontrol(&mut self, mrid: &MRIDType) {
        todo!()
    }

    /// Cancel an [`EventInstance<DerControl>`] that has been previously added to the schedule
    ///
    /// Update the internal [`EventInstance<DerControl>`]
    ///
    /// `cancel_reason` must/will be one of [`EIStatus::Cancelled`] | [`EIStatus::CancelledRandom`] | [`EIStatus::Superseded`]
    async fn cancel_dercontrol(&mut self, mrid: &MRIDType, cancel_reason: EIStatus) {
        let target_ei = self.events.get(mrid).unwrap().clone();
        target_ei.write().await.status = cancel_reason;
        let resp = (self.callback)(target_ei.clone(), cancel_reason).await;
        self.client
            .send_der_response(
                self.device.read().await.lfdi,
                &target_ei.read().await.event,
                resp,
            )
            .await;
    }
}
