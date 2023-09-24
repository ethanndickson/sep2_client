use std::sync::Arc;

use async_trait::async_trait;
use sep2_client::{
    client::Client,
    event::{EIStatus, EventHandler, EventInstance, Schedule},
};
use sep2_common::packages::{der::DERControl, edev::EndDevice, identification::ResponseStatus};
use tokio::sync::RwLock;

fn test_setup() -> Schedule<DERControl, DERControlHandler> {
    let client = Client::new(
        "https://127.0.0.1:1337",
        "../certs/client_cert.pem",
        "../certs/client_private_key.pem",
        None,
    )
    .unwrap();
    let device = EndDevice::default();
    let handler = DERControlHandler {};
    Schedule::new(client, Arc::new(RwLock::new(device)), handler)
}

struct DERControlHandler {}

#[async_trait]
impl EventHandler<DERControl> for DERControlHandler {
    async fn event_update(
        &self,
        event: Arc<RwLock<EventInstance<DERControl>>>,
        status: EIStatus,
    ) -> ResponseStatus {
        match status {
            EIStatus::Scheduled => ResponseStatus::EventAcknowledge,
            EIStatus::Active => todo!(),
            EIStatus::Cancelled => todo!(),
            EIStatus::Complete => todo!(),
            EIStatus::CancelledRandom => todo!(),
            EIStatus::Superseded => todo!(),
        }
    }
}

#[tokio::test]
async fn der_scheduler() {
    let schedule = test_setup();
}
