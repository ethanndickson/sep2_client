use std::{
    future::Future,
    sync::Arc,
    time::{Duration, SystemTime},
};

use common::{
    packages::{
        identification::ResponseStatus,
        objects::{DERControl, EventStatusType},
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
    /// Subsequent retrievals or notifications of any and all [`DERControl`] resources should call this function.
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
            (EIStatus::Active, EventStatusType::Active) => (),
            // Complete -> Any - Do nothing
            (EIStatus::Complete, _) => (),
            // Scheduled -> Scheduled - Start wait task
            (EIStatus::Scheduled, EventStatusType::Scheduled) => {
                self.schedule_dercontrol(&mrid).await
            }
            // (Cancelled | CancelledRandom | Superseded) -> Any
            (EIStatus::Cancelled | EIStatus::CancelledRandom | EIStatus::Superseded, _) => (),

            // Scheduled -> (Cancelled || CancelledRandom)
            (
                EIStatus::Scheduled,
                EventStatusType::Cancelled | EventStatusType::CancelledRandom,
            ) => {
                // Respond EventCancelled
                self.client
                    .send_der_response(
                        self.device.read().await.lfdi,
                        &ei.read().await.event,
                        incoming_status.into(),
                    )
                    .await;
            }

            // Scheduled -> Active
            (EIStatus::Scheduled, EventStatusType::Active) => self.start_dercontrol(&mrid).await,

            // (Active | Scheduled) -> Superseded
            (EIStatus::Active | EIStatus::Scheduled, EventStatusType::Superseded) => {
                self.cancel_dercontrol(&mrid, incoming_status.into()).await
            }

            // Active -> (Cancelled || CancelledRandom)
            (EIStatus::Active, EventStatusType::Cancelled | EventStatusType::CancelledRandom) => {
                self.cancel_dercontrol(&mrid, incoming_status.into()).await
            }

            // Active -> Scheduled
            (EIStatus::Active, EventStatusType::Scheduled) => todo!("Is this transition possible?"),
        }
    }

    /// Start an [`EventInstance<DerControl>`] that is not [`EIStatus::Active`].
    ///
    /// Mark all events that this event supersedes as superseded.
    /// Inform both the server, and the client itself of these state changes.
    /// Create a task that ends the event when instructed, or when the event is over.
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
                if matches!(ei_w.status, EIStatus::Active) {
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

        // Start waiting for end of event
        self.start_wait_end(target_ei.clone()).await;

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
    /// If the event is active, end the waiting task.
    /// Inform the server the event was cancelled with the given reason.
    /// Update the internal [`EventInstance<DerControl>`]
    ///
    /// `cancel_reason` must be one of [`EIStatus::Cancelled`] | [`EIStatus::CancelledRandom`] | [`EIStatus::Superseded`]
    async fn cancel_dercontrol(&mut self, mrid: &MRIDType, cancel_reason: EIStatus) {
        let target_ei = self.events.get(mrid).unwrap().clone();

        match target_ei.read().await.status {
            // Cancel waiting for end task
            EIStatus::Active => {
                let mut target_ei = target_ei.write().await;
                // Unwrap: EIStatus state transitions forbid this function from being called before [`start_dercontrol`]
                match target_ei.oneshot.0.take().unwrap().send(cancel_reason) {
                    Ok(_) => log::info!(
                        "DERControlSchedule: Cancelled in-progress DERControl event with mRID: {mrid}"
                    ),
                    Err(_) => {
                        log::error!(
                            "DERControlSchedule: Failed to cancel an in-progress DERControl event with mRID: {mrid}"
                        )
                    }
                }
            }
            // Cancel waiting for start task
            EIStatus::Scheduled => todo!(),
            // Else, we don't need to update any internal state
            _ => (),
        }

        // Inform server event was cancelled
        self.client
            .send_der_response(
                self.device.read().await.lfdi,
                &target_ei.read().await.event,
                cancel_reason.into(),
            )
            .await;

        target_ei.write().await.status = cancel_reason;
    }

    /// Create an async task that waits until the given event is finished, or until told to end the event early.
    ///
    /// When either condition is met, inform the server, and the client via the callback.
    async fn start_wait_end(&mut self, ei: Arc<RwLock<EventInstance<DERControl>>>) {
        // Setup task data
        let event_duration = (ei.read().await.end
            - (SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .expect("Time went backwards")
                .as_secs() as i64))
            .max(0) as u64;

        let mut callback = self.callback.clone();
        let target_event_c = ei.clone();
        let client = self.client.clone();
        let lfdi = self.device.read().await.lfdi;
        // Unwrap: EIStatus state forbids this function from being called multiple times
        let rx = ei.write().await.oneshot.1.take().unwrap();
        tokio::spawn(async move {
            let ei = target_event_c.clone();
            let resp = tokio::select! {
                // Wait until event end, then inform client.
                _ = tokio::time::sleep(Duration::from_secs(event_duration)) => {
                    ei.write().await.status = EIStatus::Complete;
                    callback(ei, EIStatus::Complete)
                },
                // Unless we're told to end early, with a given status
                status = rx => {
                    // Unwrap: EIStatus state transitions forbid this function from being called after [`cancel_dercontrol`]
                    let status = status.unwrap();
                    ei.write().await.status = status;
                    callback(ei, status)
                }
            }
            .await;
            // Return the user-defined response
            client
                .send_der_response(lfdi, &target_event_c.read().await.event, resp)
                .await;
        });
    }
}
