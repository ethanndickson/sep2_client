use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use sep2_client::{
    client::Client,
    event::{EIStatus, EventHandler, EventInstance, Schedule},
    time::current_time,
};
use sep2_common::{
    packages::{
        der::DERControl,
        edev::EndDevice,
        identification::ResponseStatus,
        objects::EventStatusType,
        primitives::{HexBinary128, Int64, Uint32},
        types::PrimacyType,
    },
    traits::SEIdentifiedObject,
};
use tokio::sync::RwLock;

fn test_setup() -> (
    Schedule<DERControl, DERControlHandler>,
    Arc<DERControlHandler>,
) {
    let client = Client::new(
        "https://127.0.0.1:1337",
        "../certs/client_cert.pem",
        "../certs/client_private_key.pem",
        None,
    )
    .unwrap();
    let device = EndDevice::default();
    let handler = Arc::new(DERControlHandler {
        logs: RwLock::new(vec![]),
    });
    (
        Schedule::new(
            client,
            Arc::new(RwLock::new(device)),
            handler.clone(),
            Duration::from_secs(1),
        ),
        handler,
    )
}

struct DERControlHandler {
    logs: RwLock<Vec<String>>,
}

#[async_trait]
impl EventHandler<DERControl> for DERControlHandler {
    async fn event_update(
        &self,
        event: &EventInstance<DERControl>,
        status: EIStatus,
    ) -> ResponseStatus {
        let log = match status {
            EIStatus::Scheduled => {
                format!("Received DERControl: {}", event.event().mrid())
            }
            EIStatus::Active => {
                format!("DERControl Started: {}", event.event().mrid())
            }
            EIStatus::Cancelled => {
                format!("DERControl Cancelled: {}", event.event().mrid())
            }
            EIStatus::Complete => {
                format!("DERControl Complete: {}", event.event().mrid())
            }
            EIStatus::CancelledRandom => {
                format!("DERControl Cancelled: {}", event.event().mrid())
            }
            EIStatus::Superseded => {
                format!("DERControl Started: {}", event.event().mrid())
            }
        };
        self.logs.write().await.push(log);
        status.into_der_response()
    }
}

// Create an event, as would be acquired from the server
fn create_event(status: EventStatusType, count: i64, start: i64, duration: u32) -> DERControl {
    let mut out = DERControl::default();
    out.mrid = HexBinary128(count.try_into().unwrap());
    out.creation_time = Int64(count);
    out.event_status.current_status = status;
    out.interval.start = Int64(start);
    out.interval.duration = Uint32(duration);
    out
}

/// Test the scheduler with non-overlapping events
#[tokio::test]
async fn basic_der_scheduler() {
    // T0
    let (mut schedule, logs) = test_setup();
    // T3 -> T5
    let first = create_event(EventStatusType::Scheduled, 1, current_time().get() + 3, 2);
    // T7 -> T9
    let second = create_event(EventStatusType::Scheduled, 2, current_time().get() + 7, 2);
    // T11 -> T13
    let third = create_event(EventStatusType::Scheduled, 3, current_time().get() + 11, 2);
    schedule
        .add_dercontrol(first, PrimacyType::InHomeEnergyManagementSystem)
        .await;
    schedule
        .add_dercontrol(second, PrimacyType::InHomeEnergyManagementSystem)
        .await;
    schedule
        .add_dercontrol(third, PrimacyType::InHomeEnergyManagementSystem)
        .await;
    tokio::time::sleep(Duration::from_secs(15)).await;
    println!("{:?}", logs.logs.read().await);
}
