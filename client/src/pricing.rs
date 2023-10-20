use std::{sync::Arc, time::Duration};

use sep2_common::packages::pricing::{TariffProfile, TimeTariffInterval};
use tokio::sync::RwLock;

use crate::{
    client::Client,
    device::SEDevice,
    event::{EventHandler, Events, Schedule, Scheduler},
};

// Pricing Function Set
impl<H: EventHandler<TimeTariffInterval>> Schedule<TimeTariffInterval, H> {}

#[async_trait::async_trait]
impl<H: EventHandler<TimeTariffInterval>> Scheduler<TimeTariffInterval, H>
    for Schedule<TimeTariffInterval, H>
{
    type Program = TariffProfile;
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
    async fn add_event(&mut self, event: TimeTariffInterval, program: &Self::Program) {}
}
