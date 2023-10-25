//! Sample DER Client Binary for the IEEE 2030.5 Client Library

use std::{sync::Arc, time::Duration};

use anyhow::{anyhow, Result};
use clap::Parser;
use sep2_client::{
    client::{Client, SEPResponse},
    device::SEDevice,
    event::{EIStatus, EventHandler, EventInstance, Schedule, Scheduler},
    pubsub::ClientNotifServer,
};
use sep2_common::packages::{
    dcap::DeviceCapability,
    der::{DERControl, DERControlList, DERProgramList, DefaultDERControl},
    edev::EndDevice,
    fsa::FunctionSetAssignmentsList,
    identification::ResponseStatus,
    metering::Reading,
    primitives::Uint32,
    pubsub::Notification,
    types::DeviceCategoryType,
};
use simple_logger::SimpleLogger;
use tokio::sync::{
    mpsc::{self, Receiver},
    RwLock,
};
use typemap_rev::{TypeMap, TypeMapKey};

struct ReadingResource;
impl TypeMapKey for ReadingResource {
    type Value = Reading;
}

#[derive(Default, Clone)]
struct Handler {}

// Example definition of how DER event status updates should be handled.
#[async_trait::async_trait]
impl EventHandler<DERControl> for Handler {
    async fn event_update(&self, event: &EventInstance<DERControl>) -> ResponseStatus {
        match event.status() {
            EIStatus::Scheduled => {
                println!("Received DERControl: {:?}", event.event());
            }
            EIStatus::Active => {
                println!("DERControl Started: {:?}", event.event());
            }
            EIStatus::Cancelled => {
                println!("DERControl Cancelled: {:?}", event.event());
            }
            EIStatus::Complete => {
                println!("DERControl Complete: {:?}", event.event());
            }
            EIStatus::CancelledRandom => {
                println!("DERControl Cancelled: {:?}", event.event());
            }
            EIStatus::Superseded => {
                println!("DERControl Started: {:?}", event.event());
            }
        };
        event.status().into()
    }
}

// Example implementation of asynchronous polling
async fn poll_derprograms(client: &Client, path: &str) -> Result<Receiver<DERProgramList>> {
    let dcap = client.get::<DERProgramList>(path).await?;
    let (tx, rx) = mpsc::channel::<DERProgramList>(100);
    client
        .start_poll(
            dcap.href.unwrap(),
            Some(Uint32(1)),
            move |dcap: DERProgramList| {
                let tx = tx.clone();
                async move { tx.send(dcap).await.unwrap() }
            },
        )
        .await;
    Ok(rx)
}

// A task to be run asynchronously - given a DERProgramList, add all events to the schedule
async fn process_derpl_task(
    client: &Client,
    mut schedule: Schedule<DERControl, Handler>,
    derpl: DERProgramList,
) -> Result<()> {
    for derp in derpl.der_program {
        match (&derp.der_control_list_link, &derp.default_der_control_link) {
            (Some(dercll), _) => {
                let dercl: DERControlList = client.get(&dercll.href).await?;
                for der in dercl.der_control {
                    // Add event to schedule
                    schedule.add_event(der, &derp).await;
                }
            }
            (_, Some(ddercl)) => {
                let _: DefaultDERControl = client.get(&ddercl.href).await?;
                // Client would apply DefaultDERControl to the device here
            }
            _ => log::warn!("Found a DERProgram with no DERControls or default"),
        }
    }
    Ok(())
}

// Example: Recursively retrieve all resources required to create events for a DER Schedule
async fn setup_schedule(
    client: &Client,
    edr: Arc<RwLock<SEDevice>>,
    schedule: Schedule<DERControl, Handler>,
) -> Result<()> {
    // Add our device to the server
    let res = client.post("/edev", &edr.read().await.edev).await.unwrap();
    if let SEPResponse::Created(loc) = res {
        let loc = loc.ok_or(anyhow!("No location header provided."))?;
        // EndDevice resource is now populated,
        // use the returned location header to determine where it is
        let edr: EndDevice = client
            .get(&loc)
            .await
            .map_err(|_| anyhow!("Failed to retrieve EndDevice resource"))?;
        // Get FSAL
        let fsal = edr.function_set_assignments_list_link.unwrap();
        let fsal: FunctionSetAssignmentsList = client
            .get(&format!("{}?l={}", fsal.href, fsal.all.unwrap()))
            .await
            .map_err(|_| anyhow!("Failed to retrieve FunctionSetAssignmentsList resource"))?;
        // Find FSA with DER Program List Link
        let fsa = fsal
            .function_set_assignments
            .iter()
            .find(|e| e.der_program_list_link.is_some())
            .ok_or(anyhow!("FSA List did not contain a DER Program List Link"))?;
        // Get all the DER Programs
        let derpll = fsa.der_program_list_link.as_ref().unwrap();
        // Set a poll task on these DER Programs
        let mut rx = poll_derprograms(
            client,
            &format!("{}?l={}", derpll.href, derpll.all.unwrap()),
        )
        .await
        .map_err(|_| anyhow!("Failed to retrieve an initial instance of a DERProgramList"))?;
        let schedule = schedule.clone();
        let client = client.clone();
        tokio::task::spawn(async move {
            while let Some(derpl) = rx.recv().await {
                let _ = process_derpl_task(&client, schedule.clone(), derpl)
                    .await
                    .map_err(|e| log::warn!("Failed to process DERPL with reason: {e}"));
            }
        });
    }
    Ok(())
}

async fn incoming_dcap(notif: Notification<DeviceCapability>) -> SEPResponse {
    println!("Notif Received: {:?}", notif);
    SEPResponse::Created(None)
}

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// IP address of an IEEE 2030.5 server
    target_addr: String,
    /// IP address that the NotifServer should listen on
    notif_addr: String,
    /// Port that the NotifServer should listen on
    notif_port: i32,
    /// Path to an IEEE 2030.5 Device/Client Certificate
    cert: String,
    /// Path to the Client's TLS Private Key
    key: String,
    /// Path to the (SERCA) rootCA
    ca: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    SimpleLogger::new().init().unwrap();
    let args = Args::parse();
    // Initialise a typemap for storing Resources
    let state: Arc<RwLock<TypeMap>> = Arc::new(RwLock::new(TypeMap::new()));

    // Initialise an EndDevice resource representing this device
    // (or acquire multiple out of band EndDevices if aggregate client)
    let edr = SEDevice::new_from_cert(&args.cert, DeviceCategoryType::all()).unwrap();
    let edr = Arc::new(RwLock::new(edr));

    // Create a Notificaton server listening on 1338
    // Make it listen for reading resources on "/reading"
    let notif_state = state.clone();
    let notifs = ClientNotifServer::new(
        &format!("{}:{}", &args.notif_addr, &args.notif_port),
        &args.cert,
        &args.key,
        &args.ca,
    )?
    // Example route that adds to some thread-safe state
    .add("/reading", move |notif: Notification<Reading>| {
        let notif_state = notif_state.clone();
        async move {
            match notif.resource {
                Some(r) => {
                    notif_state.write().await.insert::<ReadingResource>(r);
                    SEPResponse::Created(None)
                }
                None => SEPResponse::BadRequest(None),
            }
        }
    })
    // Example route that uses a function pointer
    .add("/dcap", incoming_dcap);

    // Spawn an async task to run our notif server
    let notif_handle = tokio::task::spawn(notifs.run(tokio::signal::ctrl_c()));
    // Create a HTTPS client for a specfific server
    let client = Client::new(
        "https://127.0.0.1:1337",
        &args.cert,
        &args.key,
        &args.ca,
        // No KeepAlive
        None,
        // Default Poll Tick Rate (10 minutes)
        None,
    )?;
    // Create an event handler with it's own state
    let handler = Handler::default();
    // Create a DER FS Schedule (DERControl)
    let schedule: Schedule<DERControl, Handler> = Scheduler::new(
        client.clone(),
        edr.clone(),
        Arc::new(handler),
        // 10 minute intermittent sleeps
        Duration::from_secs(60 * 10),
    );

    // Setup DERControl event polling retrieval
    let _ = setup_schedule(&client, edr, schedule)
        .await
        .map_err(|e| log::warn!("Failed to setup schedule with reason {}", e));
    // All setup, run forever.
    notif_handle.await?
}
