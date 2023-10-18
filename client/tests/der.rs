#![cfg(feature = "der")]

use std::{sync::Arc, time::Duration};

use sep2_client::{
    client::Client,
    edev::SEDevice,
    event::{EIStatus, EventHandler, EventInstance, Schedule, Scheduler},
    time::current_time,
};
use sep2_common::{
    packages::{
        der::DERControl,
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
        None,
    )
    .unwrap();
    let device = SEDevice::new_from_cert("../certs/client_cert.pem").unwrap();
    let handler = Arc::new(DERControlHandler {
        logs: RwLock::new(vec![]),
    });
    (
        Schedule::new(
            client,
            Arc::new(RwLock::new(device)),
            handler.clone(),
            Duration::from_secs(0),
        ),
        handler,
    )
}

struct DERControlHandler {
    logs: RwLock<Vec<String>>,
}

#[async_trait::async_trait]
impl EventHandler<DERControl> for DERControlHandler {
    async fn event_update(&self, event: &EventInstance<DERControl>) -> ResponseStatus {
        let log = match event.status() {
            EIStatus::Scheduled => {
                format!("Received DERControl: {}", event.event().mrid().0)
            }
            EIStatus::Active => {
                format!("DERControl Started: {}", event.event().mrid().0)
            }
            EIStatus::Cancelled => {
                format!("DERControl Cancelled: {}", event.event().mrid().0)
            }
            EIStatus::Complete => {
                format!("DERControl Complete: {}", event.event().mrid().0)
            }
            EIStatus::CancelledRandom => {
                format!("DERControl Cancelled: {}", event.event().mrid().0)
            }
            EIStatus::Superseded => {
                format!("DERControl Superseded: {}", event.event().mrid().0)
            }
        };
        log::debug!("{log}");
        self.logs.write().await.push(log);
        event.status().into_der_response()
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
    // T1 -> T3
    let first = create_event(EventStatusType::Scheduled, 1, current_time().get() + 1, 2);
    // T4 -> T6
    let second = create_event(EventStatusType::Scheduled, 2, current_time().get() + 4, 2);
    // T7 -> T9
    let third = create_event(EventStatusType::Scheduled, 3, current_time().get() + 7, 2);
    // Schedule in a different roder
    schedule
        .add_event(second, PrimacyType::InHomeEnergyManagementSystem)
        .await;
    schedule
        .add_event(third, PrimacyType::InHomeEnergyManagementSystem)
        .await;
    schedule
        .add_event(first, PrimacyType::InHomeEnergyManagementSystem)
        .await;
    // Wait until all events end
    tokio::time::sleep(Duration::from_secs(10)).await;
    assert_eq!(
        logs.logs.read().await.as_ref(),
        vec![
            "DERControl Started: 1",
            "DERControl Complete: 1",
            "DERControl Started: 2",
            "DERControl Complete: 2",
            "DERControl Started: 3",
            "DERControl Complete: 3"
        ]
    );
}

/// Test the scheduler with overlapping events that get superseded
#[tokio::test]
async fn superseded_der_scheduler() {
    // T0
    let (mut schedule, logs) = test_setup();
    // T2 -> T3 - Never gets run
    let superseded = create_event(EventStatusType::Scheduled, 0, current_time().get() + 2, 1);
    // T1 -> T5
    let first = create_event(EventStatusType::Scheduled, 1, current_time().get() + 1, 4);
    // T4 -> T6
    let second = create_event(EventStatusType::Scheduled, 2, current_time().get() + 4, 2);
    // T7 -> T9
    let third = create_event(EventStatusType::Scheduled, 3, current_time().get() + 7, 2);
    schedule
        .add_event(first, PrimacyType::InHomeEnergyManagementSystem)
        .await;
    schedule
        .add_event(superseded, PrimacyType::InHomeEnergyManagementSystem)
        .await;
    tokio::time::sleep(Duration::from_secs(3)).await;
    // T3
    schedule
        .add_event(second, PrimacyType::InHomeEnergyManagementSystem)
        .await;
    schedule
        .add_event(third, PrimacyType::InHomeEnergyManagementSystem)
        .await;
    tokio::time::sleep(Duration::from_secs(7)).await;
    // T11
    assert_eq!(
        logs.logs.read().await.as_ref(),
        vec![
            "DERControl Started: 1",
            "DERControl Superseded: 1",
            "DERControl Started: 2",
            "DERControl Complete: 2",
            "DERControl Started: 3",
            "DERControl Complete: 3"
        ]
    );
}

/// Test the scheduler with events that get cancelled while in progress
#[tokio::test]
async fn cancelling_der_scheduler() {
    // T0
    let (mut schedule, logs) = test_setup();
    // T1 -> T3
    let mut first = create_event(EventStatusType::Scheduled, 1, current_time().get() + 1, 2);
    // T4 -> T6
    let mut second = create_event(EventStatusType::Scheduled, 2, current_time().get() + 4, 2);
    // T7 -> T9
    let mut third = create_event(EventStatusType::Scheduled, 3, current_time().get() + 7, 2);
    // Schedule in a different order
    schedule
        .add_event(second.clone(), PrimacyType::InHomeEnergyManagementSystem)
        .await;
    schedule
        .add_event(third.clone(), PrimacyType::InHomeEnergyManagementSystem)
        .await;
    schedule
        .add_event(first.clone(), PrimacyType::InHomeEnergyManagementSystem)
        .await;
    // Cancel first event while it's running
    tokio::time::sleep(Duration::from_secs(3)).await;
    first.event_status.current_status = EventStatusType::Cancelled;
    schedule
        .add_event(first, PrimacyType::InHomeEnergyManagementSystem)
        .await;
    // Cancel second event while it's running
    tokio::time::sleep(Duration::from_secs(3)).await;
    second.event_status.current_status = EventStatusType::Cancelled;
    schedule
        .add_event(second, PrimacyType::InHomeEnergyManagementSystem)
        .await;
    // Cancel third event before it starts
    third.event_status.current_status = EventStatusType::Cancelled;
    schedule
        .add_event(third, PrimacyType::InHomeEnergyManagementSystem)
        .await;
    tokio::time::sleep(Duration::from_secs(4)).await;
    assert_eq!(
        logs.logs.read().await.as_ref(),
        vec![
            "DERControl Started: 1",
            "DERControl Cancelled: 1",
            "DERControl Started: 2",
            "DERControl Cancelled: 2",
            // Client never learns about third event
        ]
    );
}

/// Test the scheduler with events that superseded while scheduled, then unsuperseded
#[tokio::test]
async fn unsupersede_der_scheduler() {
    // T0
    let (mut schedule, logs) = test_setup();
    // T4 -> T6 (Tentatively superseded)
    let second = create_event(EventStatusType::Scheduled, 0, current_time().get() + 4, 2);
    // T1 -> T5
    let mut first = create_event(EventStatusType::Scheduled, 1, current_time().get() + 1, 4);
    // T7 -> T9
    let third = create_event(EventStatusType::Scheduled, 3, current_time().get() + 7, 2);
    schedule
        .add_event(first.clone(), PrimacyType::InHomeEnergyManagementSystem)
        .await;
    schedule
        .add_event(second, PrimacyType::InHomeEnergyManagementSystem)
        .await;
    schedule
        .add_event(third, PrimacyType::InHomeEnergyManagementSystem)
        .await;
    tokio::time::sleep(Duration::from_secs(3)).await;
    // Cancel the first event, allowing the second to run, since it was created first
    first.event_status.current_status = EventStatusType::Cancelled;
    schedule
        .add_event(first.clone(), PrimacyType::InHomeEnergyManagementSystem)
        .await;
    tokio::time::sleep(Duration::from_secs(7)).await;
    assert_eq!(
        logs.logs.read().await.as_ref(),
        vec![
            "DERControl Started: 1",
            "DERControl Cancelled: 1",
            "DERControl Started: 0",
            "DERControl Complete: 0",
            "DERControl Started: 3",
            "DERControl Complete: 3"
        ]
    );
}

#[tokio::test]
async fn schedule_der_differing_primacy() {
    let (mut schedule, logs) = test_setup();
    // T1 -> T4
    let first = create_event(EventStatusType::Scheduled, 1, current_time().get() + 1, 3);
    // T2 -> T4
    let second = create_event(EventStatusType::Scheduled, 2, current_time().get() + 2, 2);
    // T3 -> T5
    let third = create_event(EventStatusType::Scheduled, 3, current_time().get() + 3, 2);
    schedule
        .add_event(first, PrimacyType::NonContractualServiceProvider)
        .await;
    schedule
        .add_event(second, PrimacyType::InHomeEnergyManagementSystem)
        .await;
    schedule
        .add_event(third, PrimacyType::ContractedPremisesServiceProvider)
        .await;

    tokio::time::sleep(Duration::from_secs(5)).await;

    assert_eq!(
        logs.logs.read().await.as_ref(),
        vec!["DERControl Started: 2", "DERControl Complete: 2",]
    );
}
