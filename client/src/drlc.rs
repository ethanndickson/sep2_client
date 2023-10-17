use sep2_common::packages::drlc::EndDeviceControl;

use crate::event::{EventHandler, Schedule};

use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use sep2_common::packages::types::PrimacyType;
use tokio::sync::RwLock;

use crate::{
    client::Client,
    edev::SEDevice,
    event::{Events, Scheduler},
};

// Demand Response Load Control Function Set
impl<H: EventHandler<EndDeviceControl>> Schedule<EndDeviceControl, H> {}

#[async_trait]
impl<H: EventHandler<EndDeviceControl>> Scheduler<EndDeviceControl, H>
    for Schedule<EndDeviceControl, H>
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
    async fn add_event(&mut self, event: EndDeviceControl, primacy: PrimacyType) {}
}
