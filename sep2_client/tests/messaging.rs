#![cfg(feature = "messaging")]

use std::{sync::Arc, time::Duration};

use sep2_client::{
    client::Client,
    device::SEDevice,
    event::{EIStatus, EventCallback, EventInstance, Schedule, Scheduler},
    time::current_time,
};
use sep2_common::{
    packages::{
        identification::ResponseStatus,
        messaging::{MessagingProgram, TextMessage},
        objects::EventStatusType,
        primitives::{HexBinary128, Int64, Uint32},
        types::{DeviceCategoryType, PrimacyType},
    },
    traits::SEIdentifiedObject,
};
use tokio::sync::RwLock;

fn test_setup() -> (Schedule<TextMessage>, TextMessageHandler) {
    let client = Client::new_https(
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
    let handler = TextMessageHandler {
        logs: Arc::new(RwLock::new(vec![])),
    };
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

#[derive(Clone)]
struct TextMessageHandler {
    logs: Arc<RwLock<Vec<String>>>,
}

impl EventCallback<TextMessage> for TextMessageHandler {
    async fn event_update(&self, event: &EventInstance<TextMessage>) -> ResponseStatus {
        let log = match event.status() {
            EIStatus::Scheduled => {
                format!("Received TextMessage: {}", event.event().mrid().0)
            }
            EIStatus::Active => {
                format!("TextMessage Started: {}", event.event().mrid().0)
            }
            EIStatus::Cancelled => {
                format!("TextMessage Cancelled: {}", event.event().mrid().0)
            }
            EIStatus::Complete => {
                format!("TextMessage Complete: {}", event.event().mrid().0)
            }
            EIStatus::CancelledRandom => {
                format!("TextMessage Cancelled: {}", event.event().mrid().0)
            }
            EIStatus::Superseded => {
                format!("TextMessage Superseded: {}", event.event().mrid().0)
            }
        };
        log::debug!("{log}");
        self.logs.write().await.push(log);
        event.status().into()
    }
}

// Create an event, as would be acquired from the server
fn create_event(status: EventStatusType, count: i64, start: i64, duration: u32) -> TextMessage {
    let mut out = TextMessage::default();
    out.mrid = HexBinary128(count.try_into().unwrap());
    out.creation_time = Int64(count);
    out.event_status.current_status = status;
    out.interval.start = Int64(start);
    out.interval.duration = Uint32(duration);
    out
}

/// Test the scheduler with non-overlapping events
#[tokio::test]
async fn basic_msg_scheduler() {
    let program = MessagingProgram::default();
    // T0
    let (mut schedule, logs) = test_setup();
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
            "TextMessage Started: 1",
            "TextMessage Complete: 1",
            "TextMessage Started: 2",
            "TextMessage Complete: 2",
            "TextMessage Started: 3",
            "TextMessage Complete: 3"
        ]
    );
}

/// Test the scheduler with overlapping events that would get superseded in another schedule, but don't in messaging
#[tokio::test]
async fn superseded_msg_scheduler() {
    let program = MessagingProgram::default();
    // T0
    let (mut schedule, logs) = test_setup();
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
    // T2 -> T3
    let fourth = create_event(
        EventStatusType::Scheduled,
        4,
        i64::from(current_time()) + 2,
        1,
    );
    schedule.add_event(first, &program, 0).await;
    schedule.add_event(fourth, &program, 0).await;
    schedule.add_event(second, &program, 0).await;
    schedule.add_event(third, &program, 0).await;
    tokio::time::sleep(Duration::from_secs(10)).await;
    assert_eq!(
        logs.logs.read().await.as_ref(),
        vec![
            "TextMessage Started: 1",
            "TextMessage Started: 4",
            "TextMessage Complete: 4",
            "TextMessage Started: 2",
            "TextMessage Complete: 1",
            "TextMessage Complete: 2",
            "TextMessage Started: 3",
            "TextMessage Complete: 3"
        ]
    );
}

/// Test the scheduler with events that get cancelled while in progress
#[tokio::test]
async fn cancelling_msg_scheduler() {
    let program = MessagingProgram::default();
    // T0
    let (mut schedule, logs) = test_setup();
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
            "TextMessage Started: 1",
            "TextMessage Cancelled: 1",
            "TextMessage Started: 2",
            "TextMessage Cancelled: 2",
            // Client never learns about third event
        ]
    );
}

/// Test that events with differing primacys do not effect event execution order
#[tokio::test]
async fn schedule_msg_differing_primacy() {
    let (program1, mut program2, mut program3) = (
        MessagingProgram::default(),
        MessagingProgram::default(),
        MessagingProgram::default(),
    );
    program2.primacy = PrimacyType::ContractedPremisesServiceProvider;
    program3.primacy = PrimacyType::NonContractualServiceProvider;
    let (mut schedule, logs) = test_setup();
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
    schedule.add_event(first, &program3, 0).await;
    schedule.add_event(second, &program1, 0).await;
    schedule.add_event(third, &program2, 0).await;

    tokio::time::sleep(Duration::from_secs(10)).await;

    assert_eq!(
        logs.logs.read().await.as_ref(),
        [
            "TextMessage Started: 1",
            "TextMessage Complete: 1",
            "TextMessage Started: 2",
            "TextMessage Complete: 2",
            "TextMessage Started: 3",
            "TextMessage Complete: 3"
        ]
    );
}
