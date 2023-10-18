use sep2_common::packages::flow_reservation::FlowReservationResponse;

use crate::event::{EventHandler, Schedule};

use std::{sync::Arc, time::Duration};

use sep2_common::packages::types::PrimacyType;
use tokio::sync::RwLock;

use crate::{
    client::Client,
    edev::SEDevice,
    event::{Events, Scheduler},
};

// Flow Reservation Function Set
impl<H: EventHandler<FlowReservationResponse>> Schedule<FlowReservationResponse, H> {}

#[async_trait::async_trait]
impl<H: EventHandler<FlowReservationResponse>> Scheduler<FlowReservationResponse, H>
    for Schedule<FlowReservationResponse, H>
{
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
        }
    }

    #[allow(unused_variables)]
    async fn add_event(&mut self, event: FlowReservationResponse, primacy: PrimacyType) {}
}
