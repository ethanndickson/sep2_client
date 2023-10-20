#![cfg(feature = "drlc")]

use std::{sync::Arc, time::Duration};

use sep2_client::{
    client::Client,
    device::SEDevice,
    event::{EIStatus, EventHandler, EventInstance, Schedule, Scheduler},
    time::current_time,
};
use sep2_common::{
    packages::{
        drlc::{DemandResponseProgram, EndDeviceControl},
        identification::ResponseStatus,
        objects::EventStatusType,
        primitives::{HexBinary128, Int64, Uint32},
        types::PrimacyType,
    },
    traits::SEIdentifiedObject,
};
use tokio::sync::RwLock;

fn test_setup() -> (
    Schedule<EndDeviceControl, EndDeviceControlhandler>,
    Arc<EndDeviceControlhandler>,
) {
    let client = Client::new(
        "https://127.0.0.1:1337",
        "../certs/client_cert.pem",
        "../certs/client_private_key.pem",
        "../certs/rootCA.pem",
        None,
        None,
    )
    .unwrap();
    let device = SEDevice::new_from_cert("../certs/client_cert.pem").unwrap();
    let handler = Arc::new(EndDeviceControlhandler {
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

struct EndDeviceControlhandler {
    logs: RwLock<Vec<String>>,
}

#[async_trait::async_trait]
impl EventHandler<EndDeviceControl> for EndDeviceControlhandler {
    async fn event_update(&self, event: &EventInstance<EndDeviceControl>) -> ResponseStatus {
        let log = match event.status() {
            EIStatus::Scheduled => {
                format!("Received EndDeviceControl: {}", event.event().mrid().0)
            }
            EIStatus::Active => {
                format!("EndDeviceControl Started: {}", event.event().mrid().0)
            }
            EIStatus::Cancelled => {
                format!("EndDeviceControl Cancelled: {}", event.event().mrid().0)
            }
            EIStatus::Complete => {
                format!("EndDeviceControl Complete: {}", event.event().mrid().0)
            }
            EIStatus::CancelledRandom => {
                format!("EndDeviceControl Cancelled: {}", event.event().mrid().0)
            }
            EIStatus::Superseded => {
                format!("EndDeviceControl Superseded: {}", event.event().mrid().0)
            }
        };
        log::debug!("{log}");
        self.logs.write().await.push(log);
        event.status().into()
    }
}

// Create an event, as would be acquired from the server
fn create_event(
    status: EventStatusType,
    count: i64,
    start: i64,
    duration: u32,
) -> EndDeviceControl {
    let mut out = EndDeviceControl::default();
    out.mrid = HexBinary128(count.try_into().unwrap());
    out.creation_time = Int64(count);
    out.event_status.current_status = status;
    out.interval.start = Int64(start);
    out.interval.duration = Uint32(duration);
    out
}

/// Test the scheduler with non-overlapping events
#[tokio::test]
async fn basic_drlc_scheduler() {
    let program = DemandResponseProgram::default();
    // T0
    let (mut schedule, logs) = test_setup();
    // T1 -> T3
    let first = create_event(EventStatusType::Scheduled, 1, current_time().get() + 1, 2);
    // T4 -> T6
    let second = create_event(EventStatusType::Scheduled, 2, current_time().get() + 4, 2);
    // T7 -> T9
    let third = create_event(EventStatusType::Scheduled, 3, current_time().get() + 7, 2);
    // Schedule in a different order
    schedule.add_event(second, &program).await;
    schedule.add_event(third, &program).await;
    schedule.add_event(first, &program).await;
    // Wait until all events end
    tokio::time::sleep(Duration::from_secs(10)).await;
    assert_eq!(
        logs.logs.read().await.as_ref(),
        vec![
            "EndDeviceControl Started: 1",
            "EndDeviceControl Complete: 1",
            "EndDeviceControl Started: 2",
            "EndDeviceControl Complete: 2",
            "EndDeviceControl Started: 3",
            "EndDeviceControl Complete: 3"
        ]
    );
}

/// Test the scheduler with overlapping events that get superseded
#[tokio::test]
async fn superseded_drlc_scheduler() {
    let program = DemandResponseProgram::default();
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
    schedule.add_event(first, &program).await;
    schedule.add_event(superseded, &program).await;
    tokio::time::sleep(Duration::from_secs(3)).await;
    // T3
    schedule.add_event(second, &program).await;
    schedule.add_event(third, &program).await;
    tokio::time::sleep(Duration::from_secs(7)).await;
    // T11
    assert_eq!(
        logs.logs.read().await.as_ref(),
        vec![
            "EndDeviceControl Started: 1",
            "EndDeviceControl Superseded: 1",
            "EndDeviceControl Started: 2",
            "EndDeviceControl Complete: 2",
            "EndDeviceControl Started: 3",
            "EndDeviceControl Complete: 3"
        ]
    );
}

/// Test the scheduler with events that get cancelled while in progress
#[tokio::test]
async fn cancelling_drlc_scheduler() {
    let program = DemandResponseProgram::default();
    // T0
    let (mut schedule, logs) = test_setup();
    // T1 -> T3
    let mut first = create_event(EventStatusType::Scheduled, 1, current_time().get() + 1, 2);
    // T4 -> T6
    let mut second = create_event(EventStatusType::Scheduled, 2, current_time().get() + 4, 2);
    // T7 -> T9
    let mut third = create_event(EventStatusType::Scheduled, 3, current_time().get() + 7, 2);
    // Schedule in a different order
    schedule.add_event(second.clone(), &program).await;
    schedule.add_event(third.clone(), &program).await;
    schedule.add_event(first.clone(), &program).await;
    // Cancel first event while it's running
    tokio::time::sleep(Duration::from_secs(3)).await;
    first.event_status.current_status = EventStatusType::Cancelled;
    schedule.add_event(first, &program).await;
    // Cancel second event while it's running
    tokio::time::sleep(Duration::from_secs(3)).await;
    second.event_status.current_status = EventStatusType::Cancelled;
    schedule.add_event(second, &program).await;
    // Cancel third event before it starts
    third.event_status.current_status = EventStatusType::Cancelled;
    schedule.add_event(third, &program).await;
    tokio::time::sleep(Duration::from_secs(4)).await;
    assert_eq!(
        logs.logs.read().await.as_ref(),
        vec![
            "EndDeviceControl Started: 1",
            "EndDeviceControl Cancelled: 1",
            "EndDeviceControl Started: 2",
            "EndDeviceControl Cancelled: 2",
            // Client never learns about third event
        ]
    );
}

/// Test the scheduler with events that superseded while scheduled, then unsuperseded
#[tokio::test]
async fn unsupersede_drlc_scheduler() {
    let program = DemandResponseProgram::default();
    // T0
    let (mut schedule, logs) = test_setup();
    // T4 -> T6 (Tentatively superseded)
    let second = create_event(EventStatusType::Scheduled, 0, current_time().get() + 4, 2);
    // T1 -> T5
    let mut first = create_event(EventStatusType::Scheduled, 1, current_time().get() + 1, 4);
    // T7 -> T9
    let third = create_event(EventStatusType::Scheduled, 3, current_time().get() + 7, 2);
    schedule.add_event(first.clone(), &program).await;
    schedule.add_event(second, &program).await;
    schedule.add_event(third, &program).await;
    tokio::time::sleep(Duration::from_secs(3)).await;
    // Cancel the first event, allowing the second to run, since it was created first
    first.event_status.current_status = EventStatusType::Cancelled;
    schedule.add_event(first.clone(), &program).await;
    tokio::time::sleep(Duration::from_secs(7)).await;
    assert_eq!(
        logs.logs.read().await.as_ref(),
        vec![
            "EndDeviceControl Started: 1",
            "EndDeviceControl Cancelled: 1",
            "EndDeviceControl Started: 0",
            "EndDeviceControl Complete: 0",
            "EndDeviceControl Started: 3",
            "EndDeviceControl Complete: 3"
        ]
    );
}

#[tokio::test]
async fn schedule_drlc_differing_primacy() {
    let (program1, mut program2, mut program3) = (
        DemandResponseProgram::default(),
        DemandResponseProgram::default(),
        DemandResponseProgram::default(),
    );
    program2.primacy = PrimacyType::ContractedPremisesServiceProvider;
    program3.primacy = PrimacyType::NonContractualServiceProvider;
    let (mut schedule, logs) = test_setup();
    // T1 -> T4
    let first = create_event(EventStatusType::Scheduled, 1, current_time().get() + 1, 3);
    // T2 -> T4
    let second = create_event(EventStatusType::Scheduled, 2, current_time().get() + 2, 2);
    // T3 -> T5
    let third = create_event(EventStatusType::Scheduled, 3, current_time().get() + 3, 2);
    schedule.add_event(first, &program3).await;
    schedule.add_event(second, &program1).await;
    schedule.add_event(third, &program2).await;

    tokio::time::sleep(Duration::from_secs(5)).await;

    assert_eq!(
        logs.logs.read().await.as_ref(),
        vec![
            "EndDeviceControl Started: 2",
            "EndDeviceControl Complete: 2",
        ]
    );
}
