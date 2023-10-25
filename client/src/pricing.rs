use std::{
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use sep2_common::packages::{
    identification::ResponseStatus,
    objects::EventStatusType as EventStatus,
    pricing::{RateComponent, TariffProfile, TimeTariffInterval},
    types::MRIDType,
};
use tokio::sync::{broadcast::Receiver, RwLock};

use crate::{
    client::{Client, SEPResponse},
    device::SEDevice,
    event::{EIPair, EIStatus, EventHandler, EventInstance, Events, Schedule, Scheduler},
};

/// Given two TimeTariffIntervals, determine which is superseded, and which is superseding, or None if neither supersede one another
fn pricing_supersedes<'a>(
    a: EIPair<'a, TimeTariffInterval>,
    b: EIPair<'a, TimeTariffInterval>,
) -> Option<(
    EIPair<'a, TimeTariffInterval>,
    EIPair<'a, TimeTariffInterval>,
)> {
    // Program MRID refers to the MRID of the rate
    let same_rate_component = a.0.program_mrid() == b.0.program_mrid();
    if a.0.does_supersede(b.0) && same_rate_component {
        Some((b, a))
    } else if b.0.does_supersede(a.0) && same_rate_component {
        Some((a, b))
    } else {
        None
    }
}

// Pricing Function Set
impl<H: EventHandler<TimeTariffInterval>> Schedule<TimeTariffInterval, H> {
    async fn pricing_start_task(self, mut rx: Receiver<()>) {
        loop {
            // Intermittently sleep until next event start time
            tokio::select! {
                _ = tokio::time::sleep(self.tickrate) => (),
                _ = rx.recv() => {
                    log::info!("PricingSchedule: Shutting down event start task...");
                    break
                },
            }
            let mut events = self.events.write().await;
            let mrid = match events.next_start() {
                Some((time, mrid)) if time < self.schedule_time().get() => mrid,
                // If no next, or not time yet
                _ => continue,
            };

            // Mark event as complete
            events.update_event(&mrid, EIStatus::Active);

            // Notify client and server
            let events = events.downgrade();
            let target = events.get(&mrid).unwrap();
            let resp = self.handler.event_update(target).await;
            self.auto_pricing_response(target.event(), resp).await;
        }
    }

    async fn pricing_end_task(self, mut rx: Receiver<()>) {
        loop {
            // Intermittently sleep until next event end time
            tokio::select! {
                _ = tokio::time::sleep(self.tickrate) => (),
                _ = rx.recv() => {
                    log::info!("PricingSchedule: Shutting down event end task...");
                    break
                },
            }
            let mut events = self.events.write().await;
            let mrid = match events.next_end() {
                Some((time, mrid)) if time < self.schedule_time().get() => mrid,
                // If no next, or not time yet
                _ => continue,
            };

            // Mark event as complete
            events.update_event(&mrid, EIStatus::Complete);

            // Notify client and server
            let events = events.downgrade();
            let target = events.get(&mrid).unwrap();
            let resp = self.handler.event_update(target).await;
            self.auto_pricing_response(target.event(), resp).await;
        }
    }

    async fn cancel_timetariffinterval(
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
        self.auto_pricing_response(ei.event(), resp).await;
    }

    async fn auto_pricing_response(&self, event: &TimeTariffInterval, status: ResponseStatus) {
        match self
            .client
            .send_pricing_response(
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
                "Client: Pricing response POST attempt failed with HTTP status code: {}",
                e
            ),
            Err(e) => log::warn!(
                "Client: Pricing response POST attempted failed with reason: {}",
                e
            ),
            Ok(r @ (SEPResponse::Created(_) | SEPResponse::NoContent)) => {
                log::info!(
                    "Client: Pricing response POST attempt succeeded with reason: {}",
                    r
                )
            }
        }
    }
}

#[async_trait::async_trait]
impl<H: EventHandler<TimeTariffInterval>> Scheduler<TimeTariffInterval, H>
    for Schedule<TimeTariffInterval, H>
{
    type Program = (TariffProfile, RateComponent);
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
        tokio::spawn(out.clone().pricing_start_task(tx.subscribe()));
        tokio::spawn(out.clone().pricing_end_task(tx.subscribe()));
        out
    }

    async fn add_event(
        &mut self,
        event: TimeTariffInterval,
        program: &Self::Program,
        server_id: u8,
    ) {
        let tariff_profile = &program.0;
        let rate_component = &program.1;
        let mrid = event.mrid;
        let incoming_status = event.event_status.current_status;

        // If the event already exists in the schedule
        if self.events.read().await.contains(&mrid) {
            // "Editing events shall NOT be allowed, except for updating status"
            let current_status = self.events.read().await.get(&mrid).unwrap().status();
            match (current_status, incoming_status) {
                // Active -> (Cancelled || CancelledRandom || Superseded)
                (EIStatus::Active, EventStatus::Cancelled | EventStatus::CancelledRandom | EventStatus::Superseded) => {
                    log::warn!("PricingSchedule: TimeTariffInterval ({mrid}) has been marked as superseded by the server, yet it is active locally. The event will be cancelled");
                    self.cancel_timetariffinterval(&mrid, current_status, incoming_status.into()).await;
                },
                // Scheduled -> (Cancelled || CancelledRandom)
                (EIStatus::Scheduled, EventStatus::Cancelled | EventStatus::CancelledRandom) => {
                    log::info!("PricingSchedule: TimeTariffInterval ({mrid} has been marked as cancelled by the server. It will not be started");
                    self.cancel_timetariffinterval(&mrid, current_status, incoming_status.into()).await;
                },
                // Scheduled -> Active
                (EIStatus::Scheduled, EventStatus::Active) => {
                    log::info!("PricingSchedule: TimeTariffInterval ({mrid}) has entered it's earliest effective start time.")
                }
                // Scheduled -> Superseded
                (EIStatus::Scheduled, EventStatus::Superseded) =>
                    log::warn!("PricingSchedule: TimeTariffInterval ({mrid}) has been marked as superseded by the server, yet it is not locally."),
                // Active -> Scheduled
                (EIStatus::Active, EventStatus::Scheduled) =>
                    log::warn!("PricingSchedule: TimeTariffInterval ({mrid}) is active locally, and scheduled on the server. Is the client clock ahead?"),
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
            self.auto_pricing_response(&event, ResponseStatus::EventReceived)
                .await;

            // Event arrives cancelled or superseded
            if matches!(
                incoming_status,
                EventStatus::Cancelled | EventStatus::CancelledRandom | EventStatus::Superseded
            ) {
                log::warn!("PricingSchedule: Told to schedule TimeTariffInterval ({mrid}) which is already {:?}, sending server response and not scheduling.", incoming_status);
                self.auto_pricing_response(&event, incoming_status.into())
                    .await;
                return;
            }

            // Calculate start & end times
            // TODO: Clamp the duration and start time to remove gaps between successive events
            let ei = EventInstance::new_rand(
                tariff_profile.primacy,
                event.randomize_duration,
                event.randomize_start,
                event,
                rate_component.mrid,
                server_id,
            );

            // The event may have expired already
            if ei.end_time() <= self.schedule_time().get() {
                log::warn!("PricingSchedule: Told to schedule TimeTariffInterval ({mrid}) which has already ended, sending server response and not scheduling.");
                // Do not add event to schedule
                // For function sets with direct control ... Do this response
                self.auto_pricing_response(ei.event(), ResponseStatus::EventExpired)
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
                    pricing_supersedes((&mut target, &mrid), (other, o_mrid))
                {
                    // Mark as superseded
                    let prev_status = superseded.status();
                    superseded.update_status(EIStatus::Superseded);
                    superseded.superseded_by(superseding_mrid);

                    // Determine appropriate status
                    let status = if prev_status == EIStatus::Active {
                        // Since the newly superseded event is over, tell the client it's finished
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
                    self.auto_pricing_response(superseded.event(), status).await;
                }
            }

            // Update `next_start` and `next_end`
            events.update_nexts();

            // Add it to our schedule
            events.insert(&mrid, target);
        };
    }
}
