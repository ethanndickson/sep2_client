#![cfg(feature = "pricing")]

use std::{sync::Arc, time::Duration};

use sep2_client::{
    client::Client,
    device::SEDevice,
    event::{EIStatus, EventHandler, EventInstance, Schedule, Scheduler},
    time::current_time,
};
use sep2_common::{
    object::mrid_gen,
    packages::{
        identification::ResponseStatus,
        objects::EventStatusType,
        pricing::{RateComponent, TariffProfile, TimeTariffInterval},
        primitives::{HexBinary128, Int64, Uint32},
        types::{DeviceCategoryType, PrimacyType},
    },
    traits::SEIdentifiedObject,
};
use tokio::sync::RwLock;

fn test_setup() -> (
    Schedule<TimeTariffInterval, TimeTariffIntervalHandler>,
    Arc<TimeTariffIntervalHandler>,
    (TariffProfile, RateComponent),
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
    let device =
        SEDevice::new_from_cert("../certs/client_cert.pem", DeviceCategoryType::all()).unwrap();
    let handler = Arc::new(TimeTariffIntervalHandler {
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
        (TariffProfile::default(), RateComponent::default()),
    )
}

struct TimeTariffIntervalHandler {
    logs: RwLock<Vec<String>>,
}

#[async_trait::async_trait]
impl EventHandler<TimeTariffInterval> for TimeTariffIntervalHandler {
    async fn event_update(&self, event: &EventInstance<TimeTariffInterval>) -> ResponseStatus {
        let log = match event.status() {
            EIStatus::Scheduled => {
                format!("Received TimeTariffInterval: {}", event.event().mrid().0)
            }
            EIStatus::Active => {
                format!("TimeTariffInterval Started: {}", event.event().mrid().0)
            }
            EIStatus::Cancelled => {
                format!("TimeTariffInterval Cancelled: {}", event.event().mrid().0)
            }
            EIStatus::Complete => {
                format!("TimeTariffInterval Complete: {}", event.event().mrid().0)
            }
            EIStatus::CancelledRandom => {
                format!("TimeTariffInterval Cancelled: {}", event.event().mrid().0)
            }
            EIStatus::Superseded => {
                format!("TimeTariffInterval Superseded: {}", event.event().mrid().0)
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
) -> TimeTariffInterval {
    let mut out = TimeTariffInterval::default();
    out.mrid = HexBinary128(count.try_into().unwrap());
    out.creation_time = Int64(count);
    out.event_status.current_status = status;
    out.interval.start = Int64(start);
    out.interval.duration = Uint32(duration);
    out
}

/// Test the scheduler with non-overlapping events
#[tokio::test]
async fn basic_pricing_scheduler() {
    // T0
    let (mut schedule, logs, program) = test_setup();
    // T1 -> T3
    let first = create_event(
        EventStatusType::Scheduled,
        1,
        i64::from(current_time()) + 1,
        2,
    );
    // T4 -> T6
    let second = create_event(
        EventStatusType::Scheduled,
        2,
        i64::from(current_time()) + 4,
        2,
    );
    // T7 -> T9
    let third = create_event(
        EventStatusType::Scheduled,
        3,
        i64::from(current_time()) + 7,
        2,
    );
    // Schedule in a different order
    schedule.add_event(second, &program, 0).await;
    schedule.add_event(third, &program, 0).await;
    schedule.add_event(first, &program, 0).await;
    // Wait until all events end
    tokio::time::sleep(Duration::from_secs(10)).await;
    assert_eq!(
        logs.logs.read().await.as_ref(),
        vec![
            "TimeTariffInterval Started: 1",
            "TimeTariffInterval Complete: 1",
            "TimeTariffInterval Started: 2",
            "TimeTariffInterval Complete: 2",
            "TimeTariffInterval Started: 3",
            "TimeTariffInterval Complete: 3"
        ]
    );
}

/// Test the scheduler with overlapping events that get superseded
#[tokio::test]
async fn superseded_pricing_scheduler() {
    // T0
    let (mut schedule, logs, program) = test_setup();
    // T2 -> T3 - Never gets run
    let superseded = create_event(
        EventStatusType::Scheduled,
        0,
        i64::from(current_time()) + 2,
        1,
    );
    // T1 -> T5
    let first = create_event(
        EventStatusType::Scheduled,
        1,
        i64::from(current_time()) + 1,
        4,
    );
    // T4 -> T6
    let second = create_event(
        EventStatusType::Scheduled,
        2,
        i64::from(current_time()) + 4,
        2,
    );
    // T7 -> T9
    let third = create_event(
        EventStatusType::Scheduled,
        3,
        i64::from(current_time()) + 7,
        2,
    );
    schedule.add_event(first, &program, 0).await;
    schedule.add_event(superseded, &program, 0).await;
    tokio::time::sleep(Duration::from_secs(3)).await;
    // T3
    schedule.add_event(second, &program, 0).await;
    schedule.add_event(third, &program, 0).await;
    tokio::time::sleep(Duration::from_secs(7)).await;
    // T11
    assert_eq!(
        logs.logs.read().await.as_ref(),
        vec![
            "TimeTariffInterval Started: 1",
            "TimeTariffInterval Superseded: 1",
            "TimeTariffInterval Started: 2",
            "TimeTariffInterval Complete: 2",
            "TimeTariffInterval Started: 3",
            "TimeTariffInterval Complete: 3"
        ]
    );
}

/// Test the scheduler with events that get cancelled while in progress
#[tokio::test]
async fn cancelling_pricing_scheduler() {
    // T0
    let (mut schedule, logs, program) = test_setup();
    // T1 -> T3
    let mut first = create_event(
        EventStatusType::Scheduled,
        1,
        i64::from(current_time()) + 1,
        2,
    );
    // T4 -> T6
    let mut second = create_event(
        EventStatusType::Scheduled,
        2,
        i64::from(current_time()) + 4,
        2,
    );
    // T7 -> T9
    let mut third = create_event(
        EventStatusType::Scheduled,
        3,
        i64::from(current_time()) + 7,
        2,
    );
    // Schedule in a different order
    schedule.add_event(second.clone(), &program, 0).await;
    schedule.add_event(third.clone(), &program, 0).await;
    schedule.add_event(first.clone(), &program, 0).await;
    // Cancel first event while it's running
    tokio::time::sleep(Duration::from_secs(3)).await;
    first.event_status.current_status = EventStatusType::Cancelled;
    schedule.add_event(first, &program, 0).await;
    // Cancel second event while it's running
    tokio::time::sleep(Duration::from_secs(3)).await;
    second.event_status.current_status = EventStatusType::Cancelled;
    schedule.add_event(second, &program, 0).await;
    // Cancel third event before it starts
    third.event_status.current_status = EventStatusType::Cancelled;
    schedule.add_event(third, &program, 0).await;
    tokio::time::sleep(Duration::from_secs(4)).await;
    assert_eq!(
        logs.logs.read().await.as_ref(),
        vec![
            "TimeTariffInterval Started: 1",
            "TimeTariffInterval Cancelled: 1",
            "TimeTariffInterval Started: 2",
            "TimeTariffInterval Cancelled: 2",
            // Client never learns about third event
        ]
    );
}

/// Test the scheduler with events that superseded while scheduled, then unsuperseded
#[tokio::test]
async fn unsupersede_pricing_scheduler() {
    // T0
    let (mut schedule, logs, program) = test_setup();
    // T4 -> T6 (Tentatively superseded)
    let second = create_event(
        EventStatusType::Scheduled,
        0,
        i64::from(current_time()) + 4,
        2,
    );
    // T1 -> T5
    let mut first = create_event(
        EventStatusType::Scheduled,
        1,
        i64::from(current_time()) + 1,
        4,
    );
    // T7 -> T9
    let third = create_event(
        EventStatusType::Scheduled,
        3,
        i64::from(current_time()) + 7,
        2,
    );
    schedule.add_event(first.clone(), &program, 0).await;
    schedule.add_event(second, &program, 0).await;
    schedule.add_event(third, &program, 0).await;
    tokio::time::sleep(Duration::from_secs(3)).await;
    // Cancel the first event, allowing the second to run, since it was created first
    first.event_status.current_status = EventStatusType::Cancelled;
    schedule.add_event(first.clone(), &program, 0).await;
    tokio::time::sleep(Duration::from_secs(7)).await;
    assert_eq!(
        logs.logs.read().await.as_ref(),
        vec![
            "TimeTariffInterval Started: 1",
            "TimeTariffInterval Cancelled: 1",
            "TimeTariffInterval Started: 0",
            "TimeTariffInterval Complete: 0",
            "TimeTariffInterval Started: 3",
            "TimeTariffInterval Complete: 3"
        ]
    );
}

/// Test the scheduler with events that come from programs with differing primacies, and that they get superseded as expected
#[tokio::test]
async fn schedule_pricing_differing_primacy() {
    let (program1, mut program2, mut program3) = (
        (TariffProfile::default(), RateComponent::default()),
        (TariffProfile::default(), RateComponent::default()),
        (TariffProfile::default(), RateComponent::default()),
    );
    program2.0.primacy = PrimacyType::ContractedPremisesServiceProvider;
    program3.0.primacy = PrimacyType::NonContractualServiceProvider;
    let (mut schedule, logs, _) = test_setup();
    // T1 -> T4
    let first = create_event(
        EventStatusType::Scheduled,
        1,
        i64::from(current_time()) + 1,
        3,
    );
    // T2 -> T4
    let second = create_event(
        EventStatusType::Scheduled,
        2,
        i64::from(current_time()) + 2,
        2,
    );
    // T3 -> T5
    let third = create_event(
        EventStatusType::Scheduled,
        3,
        i64::from(current_time()) + 3,
        2,
    );
    schedule.add_event(first, &program3, 0).await;
    schedule.add_event(second, &program1, 0).await;
    schedule.add_event(third, &program2, 0).await;

    tokio::time::sleep(Duration::from_secs(5)).await;

    assert_eq!(
        logs.logs.read().await.as_ref(),
        vec![
            "TimeTariffInterval Started: 2",
            "TimeTariffInterval Complete: 2",
        ]
    );
}

/// Test the scheduler that TimeTariffIntervals that come from different RateComponents do not supersede one another
#[tokio::test]
async fn schedule_pricing_differing_rate_components() {
    let (program1, mut program2, mut program3) = (
        (TariffProfile::default(), RateComponent::default()),
        (TariffProfile::default(), RateComponent::default()),
        (TariffProfile::default(), RateComponent::default()),
    );
    program2.1.mrid = mrid_gen(32);
    program3.1.mrid = mrid_gen(32);
    let (mut schedule, logs, _) = test_setup();
    // T1 -> T3
    let first = create_event(
        EventStatusType::Scheduled,
        1,
        i64::from(current_time()) + 1,
        2,
    );
    // T2 -> T4
    let second = create_event(
        EventStatusType::Scheduled,
        2,
        i64::from(current_time()) + 2,
        2,
    );
    // T5 -> T6
    let third = create_event(
        EventStatusType::Scheduled,
        3,
        i64::from(current_time()) + 5,
        1,
    );
    schedule.add_event(first, &program1, 0).await;
    schedule.add_event(second, &program2, 0).await;
    schedule.add_event(third, &program3, 0).await;
    tokio::time::sleep(Duration::from_secs(7)).await;
    assert_eq!(
        logs.logs.read().await.as_ref(),
        vec![
            "TimeTariffInterval Started: 1",
            "TimeTariffInterval Started: 2",
            "TimeTariffInterval Complete: 1",
            "TimeTariffInterval Complete: 2",
            "TimeTariffInterval Started: 3",
            "TimeTariffInterval Complete: 3"
        ]
    );
}
