//! Distributed Energy Resources Function Set
//!
//! This module is primarily an implementation of a Schedule for EndDeviceControl events.
//! DRLC uses very similar rules for it's events - the exception being the rule for when events are superseded
//!
//!

use sep2_common::packages::{
    drlc::{DemandResponseProgram, EndDeviceControl},
    identification::ResponseStatus,
    objects::EventStatusType as EventStatus,
    types::MRIDType,
};

use crate::{
    client::SEPResponse,
    event::{EIPair, EIStatus, EventHandler, EventInstance, Schedule},
};

use std::{
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use tokio::sync::{broadcast::Receiver, RwLock};

use crate::{
    client::Client,
    device::SEDevice,
    event::{Events, Scheduler},
};

/// Given two EndDeviceControls, determine which is superseded, and which is superseding, or None if neither supersede one another
fn drlc_mark_supersede<'a>(
    a: EIPair<'a, EndDeviceControl>,
    b: EIPair<'a, EndDeviceControl>,
) -> Option<(EIPair<'a, EndDeviceControl>, EIPair<'a, EndDeviceControl>)> {
    if a.0.does_supersede(b.0) {
        Some((b, a))
    } else if b.0.does_supersede(a.0) {
        Some((a, b))
    } else {
        None
    }
}

// Demand Response Load Control Function Set
impl<H: EventHandler<EndDeviceControl>> Schedule<EndDeviceControl, H> {
    async fn drlc_start_task(self, mut rx: Receiver<()>) {
        loop {
            tokio::select! {
                _ = tokio::time::sleep(self.tickrate) => (),
                _ = rx.recv() => {
                    log::info!("DRLCSchedule: Shutting down event start task...");
                    break
                },
            }
            let mut events = self.events.write().await;
            let mrid = match events.next_start() {
                Some((time, mrid)) if time < self.schedule_time().get() => mrid,
                _ => continue,
            };

            events.update_event(&mrid, EIStatus::Active);

            let events = events.downgrade();
            let target = events.get(&mrid).unwrap();
            let resp = self.handler.event_update(target).await;
            self.auto_drlc_response(target.event(), resp).await;
        }
    }

    async fn drlc_end_task(self, mut rx: Receiver<()>) {
        loop {
            tokio::select! {
                _ = tokio::time::sleep(self.tickrate) => (),
                _ = rx.recv() => {
                    log::info!("DRLCSchedule: Shutting down event end task...");
                    break
                },
            }
            let mut events = self.events.write().await;
            let mrid = match events.next_end() {
                Some((time, mrid)) if time < self.schedule_time().get() => mrid,
                _ => continue,
            };

            // Mark event as complete
            events.update_event(&mrid, EIStatus::Complete);

            // Notify client and server
            let events = events.downgrade();
            let target = events.get(&mrid).unwrap();
            let resp = self.handler.event_update(target).await;
            self.auto_drlc_response(target.event(), resp).await;
        }
    }

    async fn cancel_enddevicecontrol(
        &mut self,
        target_mrid: &MRIDType,
        current_status: EIStatus,
        cancel_reason: EIStatus,
    ) {
        let mut events = self.events.write().await;
        events.cancel_event(target_mrid, cancel_reason, self.schedule_time().get());
        let events = events.downgrade();
        let ei = events.get(target_mrid).unwrap();
        let resp = if current_status == EIStatus::Active {
            (self.handler).event_update(ei).await
        } else {
            ResponseStatus::EventCancelled
        };
        self.auto_drlc_response(ei.event(), resp).await;
    }

    async fn auto_drlc_response(&self, event: &EndDeviceControl, status: ResponseStatus) {
        match self
            .client
            .send_drlc_response(
                &*self.device.read().await,
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
                    "Client: DRLC response POST attempt failed with HTTP status code: {}",
                    e
                );
            }
            Err(e) => log::warn!(
                "Client: DRLC response POST attempt failed with reason: {}",
                e
            ),
            Ok(r @ (SEPResponse::Created(_) | SEPResponse::NoContent)) => {
                log::info!(
                    "Client: DRLC response POST attempt succeeded with reason: {}",
                    r
                )
            }
        }
    }
}

#[async_trait::async_trait]
impl<H: EventHandler<EndDeviceControl>> Scheduler<EndDeviceControl, H>
    for Schedule<EndDeviceControl, H>
{
    type Program = DemandResponseProgram;
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
        tokio::spawn(out.clone().drlc_start_task(tx.subscribe()));
        tokio::spawn(out.clone().drlc_end_task(tx.subscribe()));
        out
    }

    async fn add_event(&mut self, event: EndDeviceControl, program: &Self::Program) {
        let mrid = event.mrid;
        let incoming_status = event.event_status.current_status;
        if !event
            .device_category
            .intersects(self.device.read().await.device_categories)
        {
            log::warn!("DRLCSchedule: EndDeviceControl ({mrid}) does not target this category of device. Not scheduling event.");
            return;
        }

        // If the event already exists in the schedule
        if self.events.read().await.contains(&mrid) {
            // "Editing events shall NOT be allowed, except for updating status"
            let current_status = self.events.read().await.get(&mrid).unwrap().status();
            match (current_status, incoming_status) {
                // Active -> (Cancelled || CancelledRandom || Superseded)
                (EIStatus::Active, EventStatus::Cancelled | EventStatus::CancelledRandom | EventStatus::Superseded) => {
                    log::warn!("DRLCSchedule: EndDeviceControl ({mrid}) has been marked as superseded by the server, yet it is active locally. The event will be cancelled");
                    self.cancel_enddevicecontrol(&mrid, current_status, incoming_status.into()).await;
                },
                // Scheduled -> (Cancelled || CancelledRandom)
                (EIStatus::Scheduled, EventStatus::Cancelled | EventStatus::CancelledRandom) => {
                    log::info!("DRLCSchedule: EndDeviceControl ({mrid} has been marked as cancelled by the server. It will not be started");
                    self.cancel_enddevicecontrol(&mrid, current_status, incoming_status.into()).await;
                },
                // Scheduled -> Active
                (EIStatus::Scheduled, EventStatus::Active) => {
                    log::info!("DRLCSchedule: EndDeviceControl ({mrid}) has entered it's earliest effective start time.")
                }
                // Scheduled -> Superseded
                (EIStatus::Scheduled, EventStatus::Superseded) =>
                    log::warn!("DRLCSchedule: EndDeviceControl ({mrid}) has been marked as superseded by the server, yet it is not locally."),
                // Active -> Scheduled
                (EIStatus::Active, EventStatus::Scheduled) =>
                    log::warn!("DRLCSchedule: EndDeviceControl ({mrid}) is active locally, and scheduled on the server. Is the client clock ahead?"),
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
            self.auto_drlc_response(&event, ResponseStatus::EventReceived)
                .await;

            // Event arrives cancelled or superseded
            if matches!(
                incoming_status,
                EventStatus::Cancelled | EventStatus::CancelledRandom | EventStatus::Superseded
            ) {
                log::warn!("DRLCSchedule: Told to schedule EndDeviceControl ({mrid}) which is already {:?}, sending server response and not scheduling.", incoming_status);
                self.auto_drlc_response(&event, incoming_status.into())
                    .await;
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
            );

            // The event may have expired already
            if ei.end_time() <= self.schedule_time().get() {
                log::warn!("DRLCSchedule: Told to schedule EndDeviceControl ({mrid}) which has already ended, sending server response and not scheduling.");
                // Do not add event to schedule
                // For function sets with direct control ... Do this response
                self.auto_drlc_response(ei.event(), ResponseStatus::EventExpired)
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
                    drlc_mark_supersede((&mut target, &mrid), (other, o_mrid))
                {
                    // Mark as superseded
                    let prev_status = superseded.status();
                    superseded.update_status(EIStatus::Superseded);
                    superseded.superseded_by(superseding_mrid);
                    // Determine appropriate response
                    let status = if prev_status == EIStatus::Active {
                        // Since the newly superseded event is over, tell the client it's finished
                        (self.handler).event_update(superseded).await;
                        // If the two events come from different programs
                        if superseded.program_mrid() != superseding.program_mrid() {
                            ResponseStatus::EventAbortedProgram
                        } else {
                            ResponseStatus::EventSuperseded
                        }
                    } else {
                        ResponseStatus::EventSuperseded
                    };
                    self.auto_drlc_response(superseded.event(), status).await;
                }
            }

            // Update `next_start` and `next_end`
            events.update_nexts();

            // Add it to our schedule
            events.insert(&mrid, target);
        };
    }
}
