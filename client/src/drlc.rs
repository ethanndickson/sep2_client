//! Distributed Energy Resources Function Set
//!
//! This module is primarily an implementation of a Schedule for EndDeviceControl events.
//! DRLC uses very similar rules for it's events - the exception being the rule for when events are superseded
//!
//!

use sep2_common::packages::{
    drlc::EndDeviceControl, identification::ResponseStatus,
    objects::EventStatusType as EventStatus, types::MRIDType,
};

use crate::{
    event::{EIStatus, EventHandler, EventInstance, Schedule},
    time::current_time,
};

use std::{sync::Arc, time::Duration};

use sep2_common::packages::types::PrimacyType;
use tokio::sync::{broadcast::Receiver, RwLock};

use crate::{
    client::Client,
    device::SEDevice,
    event::{Events, Scheduler},
};

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
                Some((time, mrid)) if time < current_time().get() => mrid,
                _ => continue,
            };

            events.update_event(&mrid, EIStatus::Active);

            let events = events.downgrade();
            let target = events.get(&mrid).unwrap();
            let resp = self.handler.event_update(target).await;
            self.client
                .auto_drlc_response(&*self.device.read().await, target.event(), resp)
                .await;
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
                Some((time, mrid)) if time < current_time().get() => mrid,
                _ => continue,
            };

            // Mark event as complete
            events.update_event(&mrid, EIStatus::Complete);

            // Notify client and server
            let events = events.downgrade();
            let target = events.get(&mrid).unwrap();
            let resp = self.handler.event_update(target).await;
            self.client
                .auto_drlc_response(&*self.device.read().await, target.event(), resp)
                .await;
        }
    }

    async fn cancel_enddevicecontrol(
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
            (self.handler).event_update(ei).await
        } else {
            ResponseStatus::EventCancelled
        };
        self.client
            .auto_drlc_response(&*self.device.read().await, ei.event(), resp)
            .await;
    }
}

#[async_trait::async_trait]
impl<H: EventHandler<EndDeviceControl>> Scheduler<EndDeviceControl, H>
    for Schedule<EndDeviceControl, H>
{
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
        tokio::spawn(out.clone().drlc_start_task(tx.subscribe()));
        tokio::spawn(out.clone().drlc_end_task(tx.subscribe()));
        out
    }

    async fn add_event(&mut self, event: EndDeviceControl, primacy: PrimacyType) {
        let mrid = event.mrid;
        let incoming_status = event.event_status.current_status;

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
            self.client
                .auto_drlc_response(
                    &*self.device.read().await,
                    &event,
                    ResponseStatus::EventReceived,
                )
                .await;

            // Event arrives cancelled or superseded
            if matches!(
                incoming_status,
                EventStatus::Cancelled | EventStatus::CancelledRandom | EventStatus::Superseded
            ) {
                log::warn!("DRLCSchedule: Told to schedule EndDeviceControl ({mrid}) which is already {:?}, sending server response and not scheduling.", incoming_status);
                self.client
                    .auto_drlc_response(&*self.device.read().await, &event, incoming_status.into())
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
                log::warn!("DRLCSchedule: Told to schedule EndDeviceControl ({mrid}) which has already ended, sending server response and not scheduling.");
                // Do not add event to schedule
                // For function sets with direct control ... Do this response
                self.client
                    .auto_drlc_response(
                        &*self.device.read().await,
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
                if let Some(superseded) =
                    EventInstance::mark_supersede((&mut target, &mrid), (other, o_mrid))
                {
                    // Tell the server we've been informed this event is superseded
                    let prev_status = superseded.status();
                    superseded.update_status(EIStatus::Superseded);
                    if prev_status == EIStatus::Active {
                        // Since the newly superseded event is over, tell the client it's finished
                        (self.handler).event_update(superseded).await;
                    }
                    self.client
                        .auto_drlc_response(
                            &*self.device.read().await,
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
