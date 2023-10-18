//! Distributed Energy Resources Function Set
//!
//! This module is primarily an implementation of a Schedule for DERControl events.
//!
//!

use std::{sync::Arc, time::Duration};

use sep2_common::packages::{
    der::DERControl,
    identification::ResponseStatus,
    objects::EventStatusType as EventStatus,
    types::{MRIDType, PrimacyType},
};
use tokio::sync::{broadcast::Receiver, RwLock};

use crate::{
    client::Client,
    edev::SEDevice,
    event::{EIStatus, EventHandler, EventInstance, Events, Schedule, Scheduler},
    time::current_time,
};

impl EventInstance<DERControl> {
    // Check if two DERControls have the same base
    fn has_same_target(&self, other: &Self) -> bool {
        self.event()
            .der_control_base
            .same_target(&other.event().der_control_base)
    }
}

/// Given two DERControls, determine which is superseded,
/// mark the superseded event as superseded_by,
/// and return the superseded event
pub(crate) fn der_mark_supersede<'a>(
    a: (&'a mut EventInstance<DERControl>, &'a MRIDType),
    b: (&'a mut EventInstance<DERControl>, &'a MRIDType),
) -> Option<&'a mut EventInstance<DERControl>> {
    // TODO: Check DeviceCategory
    let same_target = a.0.has_same_target(b.0);
    let out = if a.0.does_supersede(b.0) && same_target {
        Some((a, b))
    } else if b.0.does_supersede(a.0) && same_target {
        Some((b, a))
    } else {
        None
    };

    out.map(|(superseding, superseded)| {
        log::debug!(
            "Schedule: Determined that {} superseded {}",
            superseding.1,
            superseded.1
        );
        superseded.0.superseded_by(superseding.1);
        superseded.0
    })
}

impl<H: EventHandler<DERControl>> Schedule<DERControl, H> {
    async fn der_start_task(self, mut rx: Receiver<()>) {
        loop {
            // Intermittently sleep until next event start time
            tokio::select! {
                _ = tokio::time::sleep(self.tickrate) => (),
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
            let events = events.downgrade();
            let target = events.get(&mrid).unwrap();
            let resp = self.handler.event_update(target).await;
            self.client
                .auto_der_response(self.device.read().await.lfdi, target.event(), resp)
                .await;
        }
    }

    async fn der_end_task(self, mut rx: Receiver<()>) {
        loop {
            // Intermittently sleep until next event end time
            tokio::select! {
                _ = tokio::time::sleep(self.tickrate) => (),
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
            let events = events.downgrade();
            let target = events.get(&mrid).unwrap();
            let resp = self.handler.event_update(target).await;
            self.client
                .auto_der_response(self.device.read().await.lfdi, target.event(), resp)
                .await;
        }
    }

    /// Cancel an [`EventInstance<DerControl>`] that has been previously added to the schedule
    ///
    /// Update the internal [`EventInstance<DerControl>`] state.
    /// If the event is responsible for superseding other events,
    /// and those events have not started, they will be marked as scheduled internally - the client will not be informed.
    ///
    /// `cancel_reason` must/will be one of [`EIStatus::Cancelled`] | [`EIStatus::CancelledRandom`] | [`EIStatus::Superseded`]
    async fn cancel_dercontrol(
        &mut self,
        target_mrid: &MRIDType,
        current_status: EIStatus,
        cancel_reason: EIStatus,
    ) {
        let mut events = self.events.write().await;
        events.cancel_event(target_mrid, cancel_reason);
        let events = events.downgrade();
        let ei = events.get(target_mrid).unwrap();
        let resp = if current_status == EIStatus::Active {
            // If the event was active, let the client know it is over
            (self.handler).event_update(ei).await
        } else {
            // If it's not active, the client doesn't even know about this event
            ResponseStatus::EventCancelled
        };
        self.client
            .auto_der_response(self.device.read().await.lfdi, ei.event(), resp)
            .await;
    }
}

/// DER is a function set where events exhibit 'direct control', according to the specification.
/// Thus, this Schedule will determine when overlapping or superseded events should be superseded based off their target, their primacy, and their creation time and when they are no longer superseded, if their superseding event is cancelled.
#[async_trait::async_trait]
impl<H: EventHandler<DERControl>> Scheduler<DERControl, H> for Schedule<DERControl, H> {
    /// Create a schedule for the given [`Client`] & it's [`EndDevice`] representation.
    ///
    /// Any instance of [`Client`] can be used, as responses are made in accordance to the hostnames within the provided events.
    fn new(
        client: Client,
        device: Arc<RwLock<SEDevice>>,
        handler: Arc<H>,
        tickrate: Duration,
    ) -> Self {
        let (tx, rx) = tokio::sync::broadcast::channel::<()>(1);
        let out = Schedule {
            client,
            device,
            events: Arc::new(RwLock::new(Events::new())),
            handler,
            bc_sd: tx.clone(),
            tickrate,
        };
        tokio::spawn(out.clone().clean_events(rx));
        tokio::spawn(out.clone().der_start_task(tx.subscribe()));
        tokio::spawn(out.clone().der_end_task(tx.subscribe()));
        out
    }
    /// Add a [`DERControl`] Event to the schedule.
    /// Subsequent retrievals/notifications of any and all [`DERControl`] resources should call this function.
    async fn add_event(&mut self, event: DERControl, primacy: PrimacyType) {
        let mrid = event.mrid;
        let incoming_status = event.event_status.current_status;

        // If the event already exists in the schedule
        if self.events.read().await.contains(&mrid) {
            // "Editing events shall NOT be allowed, except for updating status"
            let current_status = self.events.read().await.get(&mrid).unwrap().status();
            match (current_status, incoming_status) {
                // Active -> (Cancelled || CancelledRandom || Superseded)
                (EIStatus::Active, EventStatus::Cancelled | EventStatus::CancelledRandom | EventStatus::Superseded) => {
                    log::warn!("DERControlSchedule: DERControl ({mrid}) has been marked as superseded by the server, yet it is active locally. The event will be cancelled");
                    self.cancel_dercontrol(&mrid, current_status, incoming_status.into()).await;
                },
                // Scheduled -> (Cancelled || CancelledRandom)
                (EIStatus::Scheduled, EventStatus::Cancelled | EventStatus::CancelledRandom) => {
                    log::info!("DERControlSchedule: DERControl ({mrid} has been marked as cancelled by the server. It will not be started");
                    self.cancel_dercontrol(&mrid, current_status, incoming_status.into()).await;
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
                .auto_der_response(
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
                    .auto_der_response(
                        self.device.read().await.lfdi,
                        &event,
                        incoming_status.into(),
                    )
                    .await;
                return;
            }

            // Calculate start & end times
            // TODO: Clamp the duration and start time to remove gaps between successive events
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
                // For function sets with direct control ... Do this response
                self.client
                    .auto_der_response(
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
            for (o_mrid, other) in events.iter_mut() {
                if let Some(superseded) = der_mark_supersede((&mut target, &mrid), (other, o_mrid))
                {
                    // Tell the server we've been informed this event is superseded
                    let prev_status = superseded.status();
                    superseded.update_status(EIStatus::Superseded);
                    if prev_status == EIStatus::Active {
                        // Since the newly superseded event is over, tell the client it's finished
                        (self.handler).event_update(superseded).await;
                    }
                    self.client
                        .auto_der_response(
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
}
