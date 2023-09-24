use sep2_common::packages::drlc::EndDeviceControl;

use crate::event::{EventHandler, Schedule};

// Demand Response Load Control Function Set
impl<H: EventHandler<EndDeviceControl>> Schedule<EndDeviceControl, H> {}
