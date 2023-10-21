use crate::{security::security_init, time::current_time};
use anyhow::Result;
use sep2_common::packages::{
    edev::EndDevice,
    primitives::{HexBinary160, Uint16},
    types::{DeviceCategoryType, SFDIType},
};

#[cfg(feature = "drlc")]
use sep2_common::packages::{
    drlc::{ApplianceLoadReduction, DutyCycle, Offset, SetPoint},
    response::AppliedTargetReduction,
};

/// A representation of an IEEE 2030.5 End Device.
/// DRLC fields allow for the scheduler to create `DrResponse` instances with the correct information
pub struct SEDevice {
    pub lfdi: HexBinary160,
    pub sfdi: SFDIType,
    pub edev: EndDevice,
    pub device_categories: DeviceCategoryType,
    #[cfg(feature = "drlc")]
    pub appliance_load_reduction: Option<ApplianceLoadReduction>,
    #[cfg(feature = "drlc")]
    pub applied_target_reduction: Option<AppliedTargetReduction>,
    #[cfg(feature = "drlc")]
    pub duty_cycle: Option<DutyCycle>,
    #[cfg(feature = "drlc")]
    pub offset: Option<Offset>,
    #[cfg(feature = "drlc")]
    pub override_duration: Option<Uint16>,
    #[cfg(feature = "drlc")]
    pub set_point: Option<SetPoint>,
    // TODO: What else might users want here?
}

impl SEDevice {
    pub fn new_from_cert(cert_path: &str, device_category: DeviceCategoryType) -> Result<Self> {
        let (lfdi, sfdi) = security_init(cert_path)?;
        Ok(Self::new(lfdi, sfdi, device_category))
    }
    pub fn new(lfdi: HexBinary160, sfdi: SFDIType, device_category: DeviceCategoryType) -> Self {
        SEDevice {
            lfdi,
            sfdi,
            device_categories: device_category,
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
            appliance_load_reduction: None,
            applied_target_reduction: None,
            duty_cycle: None,
            offset: None,
            override_duration: None,
            set_point: None,
        }
    }
}
