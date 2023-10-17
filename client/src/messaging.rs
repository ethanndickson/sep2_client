use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use sep2_common::packages::{messaging::TextMessage, types::PrimacyType};
use tokio::sync::RwLock;

use crate::{
    client::Client,
    edev::SEDevice,
    event::{EventHandler, Events, Schedule, Scheduler},
};

// Messaging Function Set
impl<H: EventHandler<TextMessage>> Schedule<TextMessage, H> {}

#[async_trait]
impl<H: EventHandler<TextMessage>> Scheduler<TextMessage, H> for Schedule<TextMessage, H> {
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
    async fn add_event(&mut self, event: TextMessage, primacy: PrimacyType) {}
}
