//! Messaging Function Set
//!
//! This module is primarily an implementation of a Schedule for TextMessage events.
//!

use std::{
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use sep2_common::packages::{
    identification::ResponseStatus,
    messaging::{MessagingProgram, TextMessage},
    objects::EventStatusType as EventStatus,
    types::MRIDType,
};
use tokio::sync::{broadcast::Receiver, RwLock};

use crate::{
    client::{Client, SEPResponse},
    device::SEDevice,
    event::{EIStatus, EventHandler, EventInstance, Events, Schedule, Scheduler},
};

// Messaging Function Set
impl<H: EventHandler<TextMessage>> Schedule<TextMessage, H> {
    async fn msg_start_task(self, mut rx: Receiver<()>) {
        loop {
            tokio::select! {
                _ = tokio::time::sleep(self.tickrate) => (),
                _ = rx.recv() => {
                    log::info!("TextMessageSchedule: Shutting down event start task...");
                    break
                },
            }
            let mut events = self.events.write().await;
            let mrid = match events.next_start() {
                Some((time, mrid)) if time < self.schedule_time().into() => mrid,
                _ => continue,
            };

            events.update_event(&mrid, EIStatus::Active);

            let events = events.downgrade();
            let target = events.get(&mrid).unwrap();
            let resp = self.handler.event_update(target).await;
            self.auto_msg_response(target.event(), resp).await;
        }
    }

    async fn msg_end_task(self, mut rx: Receiver<()>) {
        loop {
            tokio::select! {
                _ = tokio::time::sleep(self.tickrate) => (),
                _ = rx.recv() => {
                    log::info!("TextMessageSchedule: Shutting down event end task...");
                    break
                },
            }
            let mut events = self.events.write().await;
            let mrid = match events.next_end() {
                Some((time, mrid)) if time < self.schedule_time().into() => mrid,
                _ => continue,
            };

            events.update_event(&mrid, EIStatus::Complete);

            let events = events.downgrade();
            let target = events.get(&mrid).unwrap();
            let resp = self.handler.event_update(target).await;
            self.auto_msg_response(target.event(), resp).await;
        }
    }

    /// Cancel an [`EventInstance<TextMessage>`] that has been previously added to the schedule.
    ///
    /// Update the internal [`EventInstance<TextMessage`] state.
    async fn cancel_textmessage(
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
        self.auto_msg_response(ei.event(), resp).await;
    }

    async fn auto_msg_response(&self, event: &TextMessage, status: ResponseStatus) {
        match self
            .client
            .send_msg_response(
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
            ) => log::warn!(
                "Client: Messaging response POST attempt failed with HTTP status code: {}",
                e
            ),
            Err(e) => log::warn!(
                "Client: Messaging response POST attempted failed with reason: {}",
                e
            ),
            Ok(r @ (SEPResponse::Created(_) | SEPResponse::NoContent)) => {
                log::info!(
                    "Client: Messaging response POST attempt succeeded with reason: {}",
                    r
                )
            }
        }
    }
}

/// Messaging is NOT a function set where events exhibit 'direct control'.
/// Unlike direct control function sets, multiple text messages can be active at any given time.
///
/// This schedule simply calls the given handler when the start time arrives, if and when the event is cancelled,
/// The supplied handler is given the primacy, and the priority of the text message, and it is up to them how they are displayed.
///
///
/// TODO: This implementation currently does not support manual acknowledgement of text messages, as while the client's handler is called, a lock is acquired on the scheduler, meaning it cannot progress.
#[async_trait::async_trait]
impl<H: EventHandler<TextMessage>> Scheduler<TextMessage, H> for Schedule<TextMessage, H> {
    type Program = MessagingProgram;
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
        tokio::spawn(out.clone().msg_start_task(tx.subscribe()));
        tokio::spawn(out.clone().msg_end_task(tx.subscribe()));
        out
    }

    async fn add_event(&mut self, event: TextMessage, program: &Self::Program, server_id: u8) {
        let mrid = event.mrid;
        let incoming_status = event.event_status.current_status;

        if self.events.read().await.contains(&mrid) {
            let current_status = self.events.read().await.get(&mrid).unwrap().status();
            match (current_status, incoming_status) {
                // Active -> (Cancelled || CancelledRandom || Superseded)
                (EIStatus::Active, EventStatus::Cancelled | EventStatus::CancelledRandom | EventStatus::Superseded) => {
                    log::warn!("TextMessageSchedule: TextMessage ({mrid}) has been marked as superseded by the server, yet it is active locally. The event will be cancelled");
                    self.cancel_textmessage(&mrid, current_status, incoming_status.into()).await;
                },
                // Scheduled -> (Cancelled || CancelledRandom)
                (EIStatus::Scheduled, EventStatus::Cancelled | EventStatus::CancelledRandom) => {
                    log::info!("TextMessageSchedule: TextMessage ({mrid} has been marked as cancelled by the server. It will not be started");
                    self.cancel_textmessage(&mrid, current_status, incoming_status.into()).await;
                },
                // Scheduled -> Active
                (EIStatus::Scheduled, EventStatus::Active) => {
                    log::info!("TextMessageSchedule: TextMessage ({mrid}) has entered it's earliest effective start time.")
                }
                // Scheduled -> Superseded
                (EIStatus::Scheduled, EventStatus::Superseded) =>
                    log::warn!("TextMessageSchedule: TextMessage ({mrid}) has been marked as superseded by the server, which is not permissible. Ignoring."),
                // Active -> Scheduled
                (EIStatus::Active, EventStatus::Scheduled) =>
                    log::warn!("TextMessageSchedule: TextMessage ({mrid}) is active locally, and scheduled on the server. Is the client clock ahead?"),
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
            let mut events = self.events.write().await;

            self.auto_msg_response(&event, ResponseStatus::EventReceived)
                .await;

            // Event arrives cancelled or superseded
            if matches!(
                incoming_status,
                EventStatus::Cancelled | EventStatus::CancelledRandom | EventStatus::Superseded
            ) {
                log::warn!("TextMessageSchedule: Told to schedule DERControl ({mrid}) which is already {:?}, sending server response and not scheduling.", incoming_status);
                self.auto_msg_response(&event, incoming_status.into()).await;
                return;
            }

            let ei = EventInstance::new(program.primacy, event, program.mrid, server_id);
            // The event may have expired already
            if ei.end_time() <= self.schedule_time().into() {
                log::warn!("TextMessageSchedule: Told to schedule TextMessage ({mrid}) which has already ended, ignoring.");
                // We do NOT send a response, as required by the spec
                return;
            }

            // Update `next_start` and `next_end`
            events.update_nexts();

            // Add it to our schedule
            events.insert(&mrid, ei);
        }
    }
}
