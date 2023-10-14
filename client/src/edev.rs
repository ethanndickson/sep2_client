use crate::{security::security_init, time::current_time};
use anyhow::Result;
use sep2_common::packages::{edev::EndDevice, primitives::HexBinary160, types::SFDIType};

/// A representation of an IEEE 2030.5 End Device.
pub struct SEDevice {
    pub lfdi: HexBinary160,
    pub sfdi: SFDIType,
    pub edev: EndDevice,
    // TODO: What else might users want here?
}

impl SEDevice {
    pub fn new_from_cert(cert_path: &str) -> Result<Self> {
        let (lfdi, sfdi) = security_init(cert_path)?;
        Ok(Self::new(lfdi, sfdi))
    }
    pub fn new(lfdi: HexBinary160, sfdi: SFDIType) -> Self {
        SEDevice {
            lfdi: lfdi,
            sfdi: sfdi,
            edev: EndDevice {
                changed_time: current_time(),
                enabled: Some(false),
                flow_reservation_request_list_link: None,
                flow_reservation_response_list_link: None,
                function_set_assignments_list_link: None,
                post_rate: None,
                registration_link: None,
                subscription_list_link: None,
                configuration_link: None,
                der_list_link: None,
                device_category: None,
                device_information_link: None,
                device_status_link: None,
                file_status_link: None,
                ip_interface_list_link: None,
                load_shed_availability_list_link: None,
                log_event_list_link: None,
                power_status_link: None,
                lfdi: Some(lfdi),
                sfdi,
                subscribable: None,
                href: None,
            },
        }
    }
}
