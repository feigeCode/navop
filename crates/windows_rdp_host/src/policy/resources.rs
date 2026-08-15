use crate::ffi::{
    RESOURCE_FLAG_CAMERAS, RESOURCE_FLAG_CLIPBOARD, RESOURCE_FLAG_DRIVES,
    RESOURCE_FLAG_DYNAMIC_DEVICES, RESOURCE_FLAG_DYNAMIC_DRIVES, RESOURCE_FLAG_MICROPHONES,
    RESOURCE_FLAG_POS_DEVICES, RESOURCE_FLAG_PRINTERS, RESOURCE_FLAG_SERIAL_PORTS,
    RESOURCE_FLAG_SMART_CARDS,
};

use super::collect_flags;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowsRdpResourcePolicy {
    pub clipboard: bool,
    pub drives: bool,
    pub dynamic_drives: bool,
    pub dynamic_devices: bool,
    pub printers: bool,
    pub serial_ports: bool,
    pub smart_cards: bool,
    pub cameras: bool,
    pub microphones: bool,
    pub pos_devices: bool,
}

impl Default for WindowsRdpResourcePolicy {
    fn default() -> Self {
        Self {
            clipboard: true,
            drives: false,
            dynamic_drives: false,
            dynamic_devices: false,
            printers: false,
            serial_ports: false,
            smart_cards: false,
            cameras: false,
            microphones: false,
            pos_devices: false,
        }
    }
}

impl WindowsRdpResourcePolicy {
    pub(crate) fn flags(&self) -> u32 {
        collect_flags([
            (self.clipboard, RESOURCE_FLAG_CLIPBOARD),
            (self.drives, RESOURCE_FLAG_DRIVES),
            (self.dynamic_drives, RESOURCE_FLAG_DYNAMIC_DRIVES),
            (self.dynamic_devices, RESOURCE_FLAG_DYNAMIC_DEVICES),
            (self.printers, RESOURCE_FLAG_PRINTERS),
            (self.serial_ports, RESOURCE_FLAG_SERIAL_PORTS),
            (self.smart_cards, RESOURCE_FLAG_SMART_CARDS),
            (self.cameras, RESOURCE_FLAG_CAMERAS),
            (self.microphones, RESOURCE_FLAG_MICROPHONES),
            (self.pos_devices, RESOURCE_FLAG_POS_DEVICES),
        ])
    }
}
