use sep2_common::packages::flow_reservation::FlowReservationResponse;

use crate::event::{EventHandler, Schedule};

// Flow Reservation Function Set
impl<H: EventHandler<FlowReservationResponse>> Schedule<FlowReservationResponse, H> {}
