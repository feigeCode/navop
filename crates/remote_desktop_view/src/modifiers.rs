use gpui::Modifiers;
use remote_desktop::{RemoteDesktopInput, RemoteKey};

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

    fn key(scancode: u16, pressed: bool) -> RemoteDesktopInput {
        RemoteDesktopInput::Key {
            key: RemoteKey::Scancode(scancode),
            pressed,
        }
    }
}
