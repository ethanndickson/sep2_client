//! Flow Reservation Function Set
//!
//! This module contains an unimplemented interface for a Flow Reversation schedule, that takes in `FlowReservationResponse`.
//!
//! Unfortunately, it's very unclear how this schedule should operate based off the specification alone, and as such a generic implementation for this client library would be difficult to build.
//! Specifically, it is unclear when `FlowReservationResponseResponses` should be sent to the server, and with what `ResponseStatus` they should contain, as for whatever reason Flow Reservation is not included in Table 27.
//!
//! Regardless, end users of this library could easily implement a version of this scheduler for their specific use case, using the implemented schedulers as a reference.

use sep2_common::packages::flow_reservation::FlowReservationResponse;

use crate::event::{EventHandler, Schedule};

use std::{
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use tokio::sync::RwLock;

use crate::{
    client::Client,
    device::SEDevice,
    event::{Events, Scheduler},
};

// Flow Reservation Schedule
#[async_trait::async_trait]
impl<H: EventHandler<FlowReservationResponse>> Scheduler<FlowReservationResponse, H>
    for Schedule<FlowReservationResponse, H>
{
    type Program = ();
    #[allow(unused_variables)]
    fn new(
        client: Client,
        device: Arc<RwLock<SEDevice>>,
        handler: Arc<H>,
        tickrate: Duration,
    ) -> Self {
        let (tx, rx) = tokio::sync::broadcast::channel::<()>(1);
        Schedule {
            client,
            device,
            events: Arc::new(RwLock::new(Events::new())),
            handler,
            bc_sd: tx.clone(),
            tickrate,
            time_offset: Arc::new(AtomicI64::new(0)),
        }
    }
    #[allow(unused_variables)]
    async fn add_event(
        &mut self,
        event: FlowReservationResponse,
        program: &Self::Program,
        server_id: u8,
    ) {
    }
}
