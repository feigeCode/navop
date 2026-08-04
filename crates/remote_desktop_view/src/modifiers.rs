use gpui::{Capslock, Modifiers};
use remote_desktop::{RemoteDesktopInput, RemoteKey};

const CAPSLOCK_SCANCODE: u16 = 0x003a;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RdpKeyboardState {
    pub modifiers: Modifiers,
    pub capslock: Capslock,
}

impl RdpKeyboardState {
    pub(crate) fn with_capslock(self, capslock: Capslock) -> Self {
        Self { capslock, ..self }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ModifierBinding {
    was_pressed: bool,
    is_pressed: bool,
    scancode: u16,
}

pub fn modifier_inputs(previous: Modifiers, current: Modifiers) -> Vec<RemoteDesktopInput> {
    [
        ModifierBinding {
            was_pressed: previous.shift,
            is_pressed: current.shift,
            scancode: 0x002a,
        },
        ModifierBinding {
            was_pressed: previous.control,
            is_pressed: current.control,
            scancode: 0x001d,
        },
        ModifierBinding {
            was_pressed: previous.alt,
            is_pressed: current.alt,
            scancode: 0x0038,
        },
        ModifierBinding {
            was_pressed: previous.platform,
            is_pressed: current.platform,
            scancode: 0xe05b,
        },
    ]
    .into_iter()
    .filter_map(modifier_input)
    .collect()
}

pub fn keyboard_state_inputs(
    previous: RdpKeyboardState,
    current: RdpKeyboardState,
) -> Vec<RemoteDesktopInput> {
    modifier_inputs(previous.modifiers, current.modifiers)
        .into_iter()
        .chain(capslock_inputs(previous.capslock, current.capslock))
        .collect()
}

fn capslock_inputs(previous: Capslock, current: Capslock) -> Vec<RemoteDesktopInput> {
    if previous == current {
        return Vec::new();
    }

    // Caps Lock is a toggle key, so each state transition is one complete key press.
    vec![
        RemoteDesktopInput::Key {
            key: RemoteKey::Scancode(CAPSLOCK_SCANCODE),
            pressed: true,
        },
        RemoteDesktopInput::Key {
            key: RemoteKey::Scancode(CAPSLOCK_SCANCODE),
            pressed: false,
        },
    ]
}

fn modifier_input(binding: ModifierBinding) -> Option<RemoteDesktopInput> {
    if binding.was_pressed == binding.is_pressed {
        return None;
    }

    Some(RemoteDesktopInput::Key {
        key: RemoteKey::Scancode(binding.scancode),
        pressed: binding.is_pressed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emits_pressed_modifiers() {
        let previous = Modifiers::default();
        let current = Modifiers {
            shift: true,
            control: true,
            alt: false,
            platform: false,
            function: false,
        };

        assert_eq!(
            modifier_inputs(previous, current),
            vec![key(0x002a, true), key(0x001d, true),]
        );
    }

    #[test]
    fn emits_released_modifiers() {
        let previous = Modifiers {
            shift: true,
            control: false,
            alt: true,
            platform: true,
            function: false,
        };
        let current = Modifiers::default();

        assert_eq!(
            modifier_inputs(previous, current),
            vec![key(0x002a, false), key(0x0038, false), key(0xe05b, false),]
        );
    }

    #[test]
    fn ignores_unchanged_and_function_modifier() {
        let previous = Modifiers {
            shift: true,
            control: false,
            alt: false,
            platform: false,
            function: false,
        };
        let current = Modifiers {
            shift: true,
            control: false,
            alt: false,
            platform: false,
            function: true,
        };

        assert_eq!(modifier_inputs(previous, current), Vec::new());
    }

    #[test]
    fn emits_capslock_key_press_when_toggle_state_changes() {
        assert_eq!(
            capslock_inputs(Capslock { on: false }, Capslock { on: true }),
            vec![key(CAPSLOCK_SCANCODE, true), key(CAPSLOCK_SCANCODE, false)]
        );
        assert_eq!(
            capslock_inputs(Capslock { on: true }, Capslock { on: false }),
            vec![key(CAPSLOCK_SCANCODE, true), key(CAPSLOCK_SCANCODE, false)]
        );
    }

    #[test]
    fn ignores_unchanged_capslock_state() {
        assert_eq!(
            capslock_inputs(Capslock { on: true }, Capslock { on: true }),
            Vec::new()
        );
    }

    #[test]
    fn synchronizes_existing_keyboard_state_before_a_key_event() {
        assert_eq!(
            keyboard_state_inputs(
                RdpKeyboardState::default(),
                RdpKeyboardState {
                    modifiers: Modifiers {
                        shift: true,
                        ..Modifiers::default()
                    },
                    capslock: Capslock { on: true },
                },
            ),
            vec![
                key(0x002a, true),
                key(CAPSLOCK_SCANCODE, true),
                key(CAPSLOCK_SCANCODE, false),
            ]
        );
    }

    #[test]
    fn capslock_refresh_preserves_a_pressed_shift_modifier() {
        let previous = RdpKeyboardState {
            modifiers: Modifiers {
                shift: true,
                ..Modifiers::default()
            },
            capslock: Capslock { on: false },
        };
        let current = previous.with_capslock(Capslock { on: true });

        assert_eq!(
            keyboard_state_inputs(previous, current),
            vec![key(CAPSLOCK_SCANCODE, true), key(CAPSLOCK_SCANCODE, false)]
        );
    }

    fn key(scancode: u16, pressed: bool) -> RemoteDesktopInput {
        RemoteDesktopInput::Key {
            key: RemoteKey::Scancode(scancode),
            pressed,
        }
    }
}
