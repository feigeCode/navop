use crate::{RemoteKey, RemoteNamedKey};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RdpScanCode {
    pub code: u16,
    pub extended: bool,
}

pub fn remote_key_to_scancode(key: &RemoteKey) -> Option<RdpScanCode> {
    Some(match key {
        RemoteKey::Scancode(code) => prefixed_scancode(*code),
        RemoteKey::Named(RemoteNamedKey::Enter) => RdpScanCode {
            code: 0x1c,
            extended: false,
        },
        RemoteKey::Named(RemoteNamedKey::Backspace) => RdpScanCode {
            code: 0x0e,
            extended: false,
        },
        RemoteKey::Named(RemoteNamedKey::Tab) => RdpScanCode {
            code: 0x0f,
            extended: false,
        },
        RemoteKey::Named(RemoteNamedKey::Escape) => RdpScanCode {
            code: 0x01,
            extended: false,
        },
        RemoteKey::Named(RemoteNamedKey::Space) => RdpScanCode {
            code: 0x39,
            extended: false,
        },
        RemoteKey::Named(RemoteNamedKey::ArrowUp) => RdpScanCode {
            code: 0x48,
            extended: true,
        },
        RemoteKey::Named(RemoteNamedKey::ArrowDown) => RdpScanCode {
            code: 0x50,
            extended: true,
        },
        RemoteKey::Named(RemoteNamedKey::ArrowLeft) => RdpScanCode {
            code: 0x4b,
            extended: true,
        },
        RemoteKey::Named(RemoteNamedKey::ArrowRight) => RdpScanCode {
            code: 0x4d,
            extended: true,
        },
        RemoteKey::Named(RemoteNamedKey::Delete) => RdpScanCode {
            code: 0x53,
            extended: true,
        },
        RemoteKey::Named(RemoteNamedKey::Home) => RdpScanCode {
            code: 0x47,
            extended: true,
        },
        RemoteKey::Named(RemoteNamedKey::End) => RdpScanCode {
            code: 0x4f,
            extended: true,
        },
        RemoteKey::Named(RemoteNamedKey::PageUp) => RdpScanCode {
            code: 0x49,
            extended: true,
        },
        RemoteKey::Named(RemoteNamedKey::PageDown) => RdpScanCode {
            code: 0x51,
            extended: true,
        },
        _ => return None,
    })
}

fn prefixed_scancode(code: u16) -> RdpScanCode {
    if code & 0xff00 == 0xe000 {
        RdpScanCode {
            code: code & 0x00ff,
            extended: true,
        }
    } else {
        RdpScanCode {
            code,
            extended: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_navigation_keys_to_rdp_scancodes() {
        assert_eq!(
            remote_key_to_scancode(&RemoteKey::Named(RemoteNamedKey::ArrowUp)),
            Some(RdpScanCode {
                code: 0x48,
                extended: true
            })
        );
        assert_eq!(
            remote_key_to_scancode(&RemoteKey::Named(RemoteNamedKey::Enter)),
            Some(RdpScanCode {
                code: 0x1c,
                extended: false
            })
        );
    }

    #[test]
    fn maps_space_to_rdp_scancode() {
        assert_eq!(
            remote_key_to_scancode(&RemoteKey::Named(RemoteNamedKey::Space)),
            Some(RdpScanCode {
                code: 0x39,
                extended: false
            })
        );
    }

    #[test]
    fn maps_prefixed_scancode_to_extended_rdp_scancode() {
        assert_eq!(
            remote_key_to_scancode(&RemoteKey::Scancode(0xe048)),
            Some(RdpScanCode {
                code: 0x48,
                extended: true
            })
        );
    }
}
