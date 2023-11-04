//! Distributed Energy Resources Function Set
//!
//! This module is primarily an implementation of a Schedule for DERControl events.
//!
//! The interface for a DERControl Schedule can be found in [`Schedule`]
//!
//!

use std::{
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use sep2_common::packages::{
    der::{DERControl, DERProgram},
    identification::ResponseStatus,
    objects::EventStatusType as EventStatus,
    types::MRIDType,
};
use tokio::sync::{broadcast::Receiver, RwLock};

use crate::{
    client::{Client, SEPResponse},
    device::SEDevice,
    event::{EIPair, EIStatus, EventHandler, EventInstance, Events, Schedule, Scheduler},
};

impl EventInstance<DERControl> {
    // Check if two DERControls have the same base
    fn has_same_target(&self, other: &Self) -> bool {
        self.event()
            .der_control_base
            .same_target(&other.event().der_control_base)
    }
}

/// Given two DERControls, determine which is superseded, and which is superseding, or None if neither supersede one another
fn der_supersedes<'a>(
    a: EIPair<'a, DERControl>,
    b: EIPair<'a, DERControl>,
) -> Option<(EIPair<'a, DERControl>, EIPair<'a, DERControl>)> {
    let same_target = a.0.has_same_target(b.0);
    if a.0.does_supersede(b.0) && same_target {
        Some((b, a))
    } else if b.0.does_supersede(a.0) && same_target {
        Some((a, b))
    } else {
        None
    }
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
                Some((time, mrid)) if time < self.schedule_time().into() => mrid,
                // If no next, or not time yet
                _ => continue,
            };

            // Mark event as complete
            events.update_event(&mrid, EIStatus::Active);

            // Notify client and server
            let target = events.get(&mrid).unwrap();
            let resp = self.handler.event_update(target).await;
            self.auto_der_response(target.event(), resp).await;
            // If the device opts-out or if the event cannot be active, we update it's internal status.
            match resp {
                ResponseStatus::EventOptOut
                | ResponseStatus::EventNotApplicable
                | ResponseStatus::EventInvalid => events.update_event(&mrid, EIStatus::Cancelled),
                _ => (),
            }
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
                Some((time, mrid)) if time < self.schedule_time().into() => mrid,
                // If no next, or not time yet
                _ => continue,
            };

            // Mark event as complete
            events.update_event(&mrid, EIStatus::Complete);

            // Notify client and server
            let events = events.downgrade();
            let target = events.get(&mrid).unwrap();
            let resp = self.handler.event_update(target).await;
            self.auto_der_response(target.event(), resp).await;
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
        events.cancel_event(target_mrid, cancel_reason, self.schedule_time().into());
        let events = events.downgrade();
        let ei = events.get(target_mrid).unwrap();
        let resp = if current_status == EIStatus::Active {
            // If the event was active, let the client know it is over
            (self.handler).event_update(ei).await
        } else {
            // If it's not active, the client doesn't even know about this event
            ResponseStatus::EventCancelled
        };
        self.auto_der_response(ei.event(), resp).await;
    }

    async fn auto_der_response(&self, event: &DERControl, status: ResponseStatus) {
        match self
            .client
            .send_der_response(
                self.device.read().await.lfdi,
                event,
                status,
                self.schedule_time(),
            )
            .await
        {
            Ok(
                e @ (SEPResponse::BadRequest(_)
                | SEPResponse::NotFound
                | SEPResponse::MethodNotAllowed(_)),
            ) => {
                log::warn!(
                    "DERControlSchedule: DERControlResponse POST attempt failed with HTTP status code: {}",
                    e
                );
            }
            Err(e) => log::warn!(
                "DERControlSchedule: DERControlResponse POST attempt failed with reason: {}",
                e
            ),
            Ok(r @ (SEPResponse::Created(_) | SEPResponse::NoContent)) => {
                log::info!(
                    "DERControlSchedule: DERControlResponse POST attempt succeeded with reason: {}",
                    r
                )
            }
        }
    }
}

/// DER is a function set where events exhibit 'direct control', according to the specification.
/// Thus, this Schedule will determine when overlapping or superseded events should be superseded based off their target, their primacy, and their creation time and when they are no longer superseded, if their superseding event is cancelled.
#[async_trait::async_trait]
impl<H: EventHandler<DERControl>> Scheduler<DERControl, H> for Schedule<DERControl, H> {
    type Program = DERProgram;
    /// Create a schedule for the given [`Client`] & it's [`SEDevice`] representation.
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
            time_offset: Arc::new(AtomicI64::new(0)),
        };
        tokio::spawn(out.clone().clean_events(rx));
        tokio::spawn(out.clone().der_start_task(tx.subscribe()));
        tokio::spawn(out.clone().der_end_task(tx.subscribe()));
        out
    }

    async fn add_event(&mut self, event: DERControl, program: &Self::Program, server_id: u8) {
        let mrid = event.mrid;
        let incoming_status = event.event_status.current_status;
        // Devices SHOULD ignore events that do not indicate their device category.
        // If not present, all devices SHOULD respond
        if let Some(category) = event.device_category {
            if !category.intersects(self.device.read().await.device_categories) {
                log::warn!("DERControlSchedule: DERControl ({mrid}) does not target this category of device. Not scheduling event.");
                return;
            }
        }

        // If the event already exists in the schedule
        // "Editing events shall NOT be allowed, except for updating status"
        let cur = { self.events.read().await.get(&mrid).map(|e| e.status()) };
        if let Some(current_status) = cur {
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
            self.auto_der_response(&event, ResponseStatus::EventReceived)
                .await;

            // Event arrives cancelled or superseded
            if matches!(
                incoming_status,
                EventStatus::Cancelled | EventStatus::CancelledRandom | EventStatus::Superseded
            ) {
                log::warn!("DERControlSchedule: Told to schedule DERControl ({mrid}) which is already {:?}, sending server response and not scheduling.", incoming_status);
                self.auto_der_response(&event, incoming_status.into()).await;
                return;
            }

            // Calculate start & end times
            // TODO: Clamp the duration and start time to remove gaps between successive events
            let ei = EventInstance::new_rand(
                program.primacy,
                event.randomize_duration,
                event.randomize_start,
                event,
                program.mrid,
                server_id,
            );

            // The event may have expired already
            if ei.end_time() <= self.schedule_time().into() {
                log::warn!("DERControlSchedule: Told to schedule DERControl ({mrid}) which has already ended, sending server response and not scheduling.");
                // Do not add event to schedule
                // For function sets with direct control ... Do this response
                self.auto_der_response(ei.event(), ResponseStatus::EventExpired)
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
                if let Some(((superseded, _), (superseding, superseding_mrid))) =
                    der_supersedes((&mut target, &mrid), (other, o_mrid))
                {
                    // Mark as superseded
                    let prev_status = superseded.status();
                    superseded.update_status(EIStatus::Superseded);
                    superseded.superseded_by(superseding_mrid);

                    // Determine appropriate status
                    let status = if prev_status == EIStatus::Active {
                        // Since the newly superseded event is over, tell the client it's finished
                        // We override whatever response the client provides to the more correct one
                        (self.handler).event_update(superseded).await;
                        if superseded.program_mrid() != superseding.program_mrid() {
                            // If the two events come from different programs
                            ResponseStatus::EventAbortedProgram
                        } else if superseded.server_id() != superseding.server_id() {
                            // If the two events come from different servers
                            ResponseStatus::EventAbortedServer
                        } else {
                            ResponseStatus::EventSuperseded
                        }
                    } else {
                        ResponseStatus::EventSuperseded
                    };
                    self.auto_der_response(superseded.event(), status).await;
                }
            }

            // Update `next_start` and `next_end`
            events.update_nexts();

            // Add it to our schedule
            events.insert(&mrid, target);
        };
    }
}
