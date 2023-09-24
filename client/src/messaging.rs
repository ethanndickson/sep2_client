use sep2_common::packages::messaging::TextMessage;

use crate::event::{EventHandler, Schedule};

// Messaging Function Set
impl<H: EventHandler<TextMessage>> Schedule<TextMessage, H> {}
