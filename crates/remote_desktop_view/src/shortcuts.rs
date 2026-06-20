use gpui::Keystroke;
use remote_desktop::{RemoteDesktopInput, RemoteDesktopProtocol, RemoteKey};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipboardShortcut {
    Copy,
    Paste,
}

pub fn is_clipboard_platform_shortcut(keystroke: &Keystroke) -> bool {
    keystroke.modifiers.platform
        && !keystroke.modifiers.control
        && !keystroke.modifiers.alt
        && matches!(keystroke.key.as_str(), "c" | "v")
}

pub fn clipboard_shortcut_inputs(
    protocol: RemoteDesktopProtocol,
    shortcut: ClipboardShortcut,
) -> Vec<RemoteDesktopInput> {
    key_chord_inputs(&clipboard_chord(protocol, shortcut))
}

fn clipboard_chord(protocol: RemoteDesktopProtocol, shortcut: ClipboardShortcut) -> Vec<RemoteKey> {
    let key = match shortcut {
        ClipboardShortcut::Copy => RemoteKey::Character('c'),
        ClipboardShortcut::Paste => RemoteKey::Character('v'),
    };

    match protocol {
        RemoteDesktopProtocol::Rdp => vec![RemoteKey::Scancode(0x001d), key],
        RemoteDesktopProtocol::Vnc => vec![RemoteKey::KeySym(0xffe3), key],
    }
}

fn key_chord_inputs(keys: &[RemoteKey]) -> Vec<RemoteDesktopInput> {
    let mut inputs = Vec::with_capacity(keys.len() * 2);
    for key in keys {
        inputs.push(key_input(key.clone(), true));
    }
    for key in keys.iter().rev() {
        inputs.push(key_input(key.clone(), false));
    }
    inputs
}

fn key_input(key: RemoteKey, pressed: bool) -> RemoteDesktopInput {
    RemoteDesktopInput::Key { key, pressed }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifies_platform_clipboard_shortcuts() {
        assert!(is_clipboard_platform_shortcut(&key("cmd-c")));
        assert!(is_clipboard_platform_shortcut(&key("cmd-v")));
        assert!(!is_clipboard_platform_shortcut(&key("cmd-x")));
        assert!(!is_clipboard_platform_shortcut(&key("ctrl-c")));
    }

    #[test]
    fn rdp_clipboard_shortcuts_use_control_key_chords() {
        assert_eq!(
            clipboard_shortcut_inputs(RemoteDesktopProtocol::Rdp, ClipboardShortcut::Copy),
            vec![
                input(RemoteKey::Scancode(0x001d), true),
                input(RemoteKey::Character('c'), true),
                input(RemoteKey::Character('c'), false),
                input(RemoteKey::Scancode(0x001d), false),
            ]
        );
    }

    #[test]
    fn vnc_clipboard_shortcuts_use_standard_control_chords() {
        assert_eq!(
            clipboard_shortcut_inputs(RemoteDesktopProtocol::Vnc, ClipboardShortcut::Paste),
            vec![
                input(RemoteKey::KeySym(0xffe3), true),
                input(RemoteKey::Character('v'), true),
                input(RemoteKey::Character('v'), false),
                input(RemoteKey::KeySym(0xffe3), false),
            ]
        );
    }

    fn key(source: &str) -> Keystroke {
        Keystroke::parse(source).expect("valid test keystroke")
    }

    fn input(key: RemoteKey, pressed: bool) -> RemoteDesktopInput {
        RemoteDesktopInput::Key { key, pressed }
    }
}
