//! Sample DER Client Binary for the IEEE 2030.5 Client Library
//!
//! This file serves as additional documentation for how the library can be used,
//! including examples for retrievals, notifications, polling and DER Event Scheduling

use std::sync::Arc;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use sep2_client::{
    client::{Client, SepResponse},
    der::der_ei_status_response,
    event::{EIStatus, EventHandler, EventInstance, Schedule},
    pubsub::{ClientNotifServer, NotifHandler},
};
use sep2_common::{
    deserialize,
    packages::{
        der::{DERControl, DERControlList, DERProgramList, DefaultDERControl},
        edev::EndDevice,
        fsa::FunctionSetAssignmentsList,
        identification::ResponseStatus,
        metering::Reading,
        primitives::{Int64, Uint32},
        pubsub::Notification,
        types::SFDIType,
    },
};
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
struct Handler {
    // Store any form of interior-mutable state, or a channel inlet here and
    // have it accessible when handling notifs or events.
    state: Arc<RwLock<TypeMap>>,
}

impl Handler {
    async fn add_reading(&self, resource: &str) -> SepResponse {
        let res = deserialize::<Notification<Reading>>(resource);
        match res {
            Ok(r) => {
                self.state
                    .write()
                    .await
                    .insert::<ReadingResource>(r.resource.unwrap());
                SepResponse::Created("/reading".to_owned())
            }
            // "Undesired subscription ... SHALL return an HTTP 400 error"
            Err(_) => SepResponse::BadRequest(None),
        }
    }
}

// Example defintion for how incoming notifications should be routed.
#[async_trait]
impl NotifHandler for Handler {
    async fn notif_handler(&self, path: &str, resource: &str) -> SepResponse {
        match path {
            "/reading" => self.add_reading(resource).await,
            _ => SepResponse::NotFound,
        }
    }
}

// Example definition of how DER event status updates should be handled.
#[async_trait]
impl EventHandler<DERControl> for Handler {
    async fn event_update(
        &self,
        event: Arc<RwLock<EventInstance<DERControl>>>,
        status: EIStatus,
    ) -> ResponseStatus {
        match status {
            EIStatus::Scheduled => {
                println!("Received DERControl: {:?}", event.read().await.event());
            }
            EIStatus::Active => {
                println!("DERControl Started: {:?}", event.read().await.event());
            }
            EIStatus::Cancelled => {
                println!("DERControl Cancelled: {:?}", event.read().await.event());
            }
            EIStatus::Complete => {
                println!("DERControl Complete: {:?}", event.read().await.event());
            }
            EIStatus::CancelledRandom => {
                println!("DERControl Cancelled: {:?}", event.read().await.event());
            }
            EIStatus::Superseded => {
                println!("DERControl Started: {:?}", event.read().await.event());
            }
        };
        der_ei_status_response(status)
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
        match (derp.der_control_list_link, derp.default_der_control_link) {
            (Some(dercll), _) => {
                let dercl: DERControlList = client.get(&dercll.href).await?;
                for der in dercl.der_control {
                    // Add event to schedule
                    schedule.add_dercontrol(der, derp.primacy).await;
                }
            }
            (_, Some(ddercl)) => {
                let ddercl: DefaultDERControl = client.get(&ddercl.href).await?;
                todo!("Handle DefaultDERControl Case, the user needs to be able to access this somehow.")
            }
            _ => log::warn!("Found a DERP with no DERControls or default"),
        }
    }
    Ok(())
}

// Example: Recursively retrieve all resources required to create events for a DER Schedule
// TODO: Do `.all` unwraps need to be replaced with the request minus query string?
async fn setup_schedule(
    client: &Client,
    edr: Arc<RwLock<EndDevice>>,
    schedule: Schedule<DERControl, Handler>,
) -> Result<()> {
    // Add our device to the server
    let res = client.post("/edev", &*edr.read().await).await.unwrap();
    if let SepResponse::Created(loc) = res {
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

#[tokio::main]
async fn main() -> Result<()> {
    // Initialise an EndDevice resource representing this device
    // (or acquire multiple out of band EndDevices if aggregate client)
    let mut edr = EndDevice::default();
    edr.changed_time = Int64(1379905200);
    edr.sfdi = SFDIType::new(987654321005).unwrap();
    let edr = Arc::new(RwLock::new(edr));
    // Create an event & notif handler with it's own state
    let handler = Handler::default();
    // Create a Notificaton server listening on 1338
    let notifs = ClientNotifServer::new(
        "127.0.0.1:1338",
        "../certs/client_cert.pem",
        "../certs/client_private_key.pem",
        handler.clone(),
    )?;
    // Spawn an async task to run our notif server
    let notif_handle = tokio::task::spawn(notifs.run());
    // Create a HTTPS client for a specfific server
    let client = Client::new(
        "https://127.0.0.1:1337",
        "../certs/client_cert.pem",
        "../certs/client_private_key.pem",
        None,
    )?;

    // Create a DER FS Schedule (DERControl)
    let schedule: Schedule<DERControl, Handler> =
        Schedule::new(client.clone(), edr.clone(), handler);

    // Setup DERControl event polling retrieval
    let _ = setup_schedule(&client, edr, schedule)
        .await
        .map_err(|e| log::warn!("Failed to setup schedule with reason {}", e));
    // All setup, run forever.
    notif_handle.await?
}
