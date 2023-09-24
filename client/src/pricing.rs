use sep2_common::packages::pricing::TimeTariffInterval;

use crate::event::{EventHandler, Schedule};

// Pricing Function Set
impl<H: EventHandler<TimeTariffInterval>> Schedule<TimeTariffInterval, H> {}
