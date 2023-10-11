use std::sync::Arc;

use sep2_common::packages::{
    der::DERControl,
    edev::EndDevice,
    identification::ResponseStatus,
    objects::EventStatusType as EventStatus,
    types::{MRIDType, PrimacyType},
};
use tokio::sync::{broadcast::Receiver, RwLock};

use crate::{
    client::Client,
    event::{EIStatus, EventHandler, EventInstance, Events, Schedule},
    time::{current_time, SLEEP_TICKRATE},
};

impl EventInstance<DERControl> {
    // Check if two DERControls have the same base
    fn has_same_target(&self, other: &Self) -> bool {
        self.event()
            .der_control_base
            .same_target(&other.event().der_control_base)
    }

    /// Return which of the given DERControl events would be superseded, if any
    pub(crate) fn der_superseded<'a>(&'a mut self, other: &'a mut Self) -> Option<&'a mut Self> {
        if self.does_supersede(other) && self.has_same_target(other) {
            return Some(other);
        }

        if other.does_supersede(self) && other.has_same_target(self) {
            return Some(self);
        }

        None
    }
}

impl<H: EventHandler<DERControl>> Schedule<DERControl, H> {
    /// Create a schedule for the given [`Client`] & it's [`EndDevice`] representation.
    ///
    /// Any instance of [`Client`] can be used, as responses are made in accordance to the hostnames within the provided events.
    pub fn new(client: Client, device: Arc<RwLock<EndDevice>>, handler: H) -> Self {
        let (tx, rx) = tokio::sync::broadcast::channel::<()>(1);
        let out = Schedule {
            client,
            device,
            events: Arc::new(RwLock::new(Events::new())),
            handler: Arc::new(handler),
            bc_sd: tx.clone(),
        };
        tokio::spawn(out.clone().clean_events(rx));
        tokio::spawn(out.clone().der_start_task(tx.subscribe()));
        tokio::spawn(out.clone().der_end_task(tx.subscribe()));
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
            if matches!(
                incoming_status,
                EventStatus::Cancelled | EventStatus::CancelledRandom | EventStatus::Superseded
            ) {
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
            let mut target = ei;
            for (_, other) in events.iter_mut() {
                // If an event is superseded
                if let Some(superseded) = target.der_superseded(other) {
                    if superseded.status() == EIStatus::Active {
                        // Since the newly superseded event is over, tell the client it's finished
                        (self.handler)
                            .event_update(&superseded, EIStatus::Superseded)
                            .await;
                    }
                    superseded.update_status(EIStatus::Superseded);
                    // Tell the server we've been informed this event is superseded
                    self.client
                        .send_der_response(
                            self.device.read().await.lfdi,
                            superseded.event(),
                            ResponseStatus::EventSuperseded,
                        )
                        .await;
                }
            }

            // Update `next_start` and `next_end`
            events.update_nexts();

            // Add it to our schedule
            events.insert(&mrid, target);
        };
    }

    async fn der_start_task(self, mut rx: Receiver<()>) {
        loop {
            // Intermittently sleep until next event start time
            tokio::select! {
                _ = tokio::time::sleep(SLEEP_TICKRATE) => (),
                _ = rx.recv() => {
                    log::info!("DERControlSchedule: Shutting down event start task...");
                    break
                },
            }
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
            self.client
                .send_der_response(self.device.read().await.lfdi, target.event(), resp)
                .await;
        }
    }

    async fn der_end_task(self, mut rx: Receiver<()>) {
        loop {
            // Intermittently sleep until next event end time
            tokio::select! {
                _ = tokio::time::sleep(SLEEP_TICKRATE) => (),
                _ = rx.recv() => {
                    log::info!("DERControlSchedule: Shutting down event end task...");
                    break
                },
            }
            let mut events = self.events.write().await;
            let mrid = match events.next_end() {
                Some((time, mrid)) if time < current_time().get() => mrid,
                // If no next, or not time yet
                _ => continue,
            };

            // Mark event as complete
            events.update_event(&mrid, EIStatus::Complete);

            // Notify client and server
            let target = events.get(&mrid).unwrap();
            let resp = self.handler.event_update(target, EIStatus::Complete).await;
            self.client
                .send_der_response(self.device.read().await.lfdi, target.event(), resp)
                .await;
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
