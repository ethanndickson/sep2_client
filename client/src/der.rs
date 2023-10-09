use sep2_common::{
    packages::{
        der::DERControl,
        identification::ResponseStatus,
        objects::EventStatusType as EventStatus,
        types::{MRIDType, PrimacyType},
    },
    traits::SEEvent,
};

use crate::{
    event::{EIStatus, EventHandler, EventInstance, Schedule},
    time::{current_time, SLEEP_TICKRATE},
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

impl<H: EventHandler<DERControl>> Schedule<DERControl, H> {
    /// Add a [`DERControl`] Event to the schedule.
    /// Subsequent retrievals/notifications of any and all [`DERControl`] resources should call this function.
    pub async fn add_dercontrol(&mut self, event: DERControl, primacy: PrimacyType) {
        let mrid = event.mrid;
        let incoming_status = event.event_status.current_status;

        // If the event already exists in the schedule
        if let Some(_) = self.events.read().await.get(&mrid) {
            // "Editing events shall NOT be allowed, except for updating status"
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
            self.events.write().await.insert(mrid, ei);
            // TODO: Refactor this function
            self.clone().schedule_dercontrol(mrid).await;
        };
        let current_status = {
            let mut events = self.events.write().await;
            let ei = events.get_mut(&mrid).unwrap();
            ei.status()
        };

        // Handle status transitions
        // TODO: Determine when the currently superseded events need to be reevaluated
        match (current_status, incoming_status) {
            // Active -> Active
            (EIStatus::Active, EventStatus::Active) => (),
            // Complete -> Any
            (EIStatus::Complete, _) => (),
            // (Cancelled | CancelledRandom | Superseded) -> Any
            (EIStatus::Cancelled | EIStatus::CancelledRandom | EIStatus::Superseded, _) => (),
            // Scheduled -> Scheduled
            (EIStatus::Scheduled, EventStatus::Scheduled) => (),
            // Scheduled -> Active
            (EIStatus::Scheduled, EventStatus::Active) => {
                log::debug!("DERControlSchedule: DERControl ({mrid}) has entered it's earliest effective start time.")
            }
            // Active -> Superseded
            (EIStatus::Active, EventStatus::Superseded) => {
                self.cancel_dercontrol(&mrid, incoming_status.into()).await
            }
            // Active -> (Cancelled || CancelledRandom)
            (EIStatus::Active, EventStatus::Cancelled | EventStatus::CancelledRandom) => {
                self.cancel_dercontrol(&mrid, incoming_status.into()).await
            }
            // Scheduled -> (Cancelled || CancelledRandom) - Respond EventCancelled
            (EIStatus::Scheduled, EventStatus::Cancelled | EventStatus::CancelledRandom) => {
                let events = self.events.read().await;
                let ei = events.get(&mrid).unwrap();
                self.client
                    .send_der_response(
                        self.device.read().await.lfdi,
                        &ei.event,
                        der_status_response(incoming_status),
                    )
                    .await;
            }
            // Scheduled -> Superseded
            (EIStatus::Scheduled, EventStatus::Superseded) => todo!(),
            // Active -> Scheduled
            (EIStatus::Active, EventStatus::Scheduled) => todo!("Is this transition possible?"),
        }
    }

    /// Start an [`EventInstance<DerControl>`]
    ///
    /// Mark all events that this event supersedes as superseded.
    /// Inform both the server, and the client itself of these state changes.
    /// Create a task that ends the event at the correct time, can be overriden.
    ///
    /// `mrid` must be the key to a previously scheduled [`EventInstance<DerControl>`].
    async fn start_dercontrol(&mut self, mrid: &MRIDType) {
        // For each event that would be superseded by this event starting:
        // - inform the client
        // - inform the server
        // - mark as superseded
        {
            // We purposefully hold the RwLock for this entire block
            let mut events = self.events.write().await;
            let target_ei = events.get(mrid).unwrap();
            let mut superseded = vec![];

            // Mark required events as superseded
            for (mrid, ei) in events.iter() {
                if target_ei.der_supersedes(ei) {
                    if ei.status() == EIStatus::Active {
                        // Since the event is active, the client needs to be told the event is over
                        (self.handler).event_update(&ei, EIStatus::Superseded).await;
                    }
                    superseded.push(*mrid);
                    self.client
                        .send_der_response(
                            self.device.read().await.lfdi,
                            &ei.event,
                            ResponseStatus::EventSuperseded,
                        )
                        .await;
                }
            }

            // Update internal status
            for mrid in superseded {
                let ei = events.get_mut(&mrid).unwrap();
                ei.update_status(EIStatus::Superseded)
            }

            // Update event status
            let target_ei = events.get_mut(&mrid).unwrap();
            target_ei.update_status(EIStatus::Active);
        }

        // Setup task
        let end = self.events.read().await.get(mrid).unwrap().end;
        let handler = self.handler.clone();
        let client = self.client.clone();
        let lfdi = self.device.read().await.lfdi;
        let events = self.events.clone();
        let mrid = mrid.clone();

        // Start waiting for end of event
        tokio::spawn(async move {
            crate::time::sleep_until(end, SLEEP_TICKRATE).await;
            let mut events = events.write().await;
            let ei = events.get_mut(&mrid).unwrap();
            // If it's Active, then it hasn't been cancelled
            if ei.status() == EIStatus::Active {
                ei.update_status(EIStatus::Complete);
                // Defer to client callback for ResponseStatus
                let resp = handler.event_update(&ei, EIStatus::Complete).await;
                // Inform server
                client.send_der_response(lfdi, &ei.event, resp).await;
            }
        });

        // Inform client of event start
        (self.handler)
            .event_update(
                self.events.read().await.get(&mrid).unwrap(),
                EIStatus::Active,
            )
            .await;
    }

    /// Schedule an [`EventInstance<DerControl>`] such that it begins at it's scheduled start time.
    async fn schedule_dercontrol(&mut self, mrid: MRIDType) {
        let start = {
            let events = self.events.read().await;
            let ei = events.get(&mrid).unwrap().clone();
            ei.start
        };
        let this = self.clone();

        tokio::spawn(async move {
            crate::time::sleep_until(start, SLEEP_TICKRATE).await;
            // If the event is still scheduled
            let mut events = this.events.write().await;
            let ei = events.get_mut(&mrid).unwrap();
            if ei.status() == EIStatus::Scheduled {
                this.clone().start_dercontrol(&mrid).await;
            }
        });
    }

    /// Cancel an [`EventInstance<DerControl>`] that has been previously added to the schedule
    ///
    /// Update the internal [`EventInstance<DerControl>`]
    ///
    /// `cancel_reason` must/will be one of [`EIStatus::Cancelled`] | [`EIStatus::CancelledRandom`] | [`EIStatus::Superseded`]
    async fn cancel_dercontrol(&mut self, mrid: &MRIDType, cancel_reason: EIStatus) {
        let mut events = self.events.write().await;
        let ei = events.get_mut(mrid).unwrap();
        ei.update_status(cancel_reason);
        let resp = (self.handler).event_update(&ei, cancel_reason).await;
        self.client
            .send_der_response(self.device.read().await.lfdi, &ei.event, resp)
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
