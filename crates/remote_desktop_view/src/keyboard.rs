use gpui::Keystroke;
use remote_desktop::{RemoteDesktopProtocol, RemoteKey, RemoteNamedKey};

pub fn keystroke_to_remote_key_for_protocol(
    keystroke: &Keystroke,
    protocol: RemoteDesktopProtocol,
) -> Option<RemoteKey> {
    match protocol {
        RemoteDesktopProtocol::Rdp => rdp_key_to_scancode(&keystroke.key).map(RemoteKey::Scancode),
        RemoteDesktopProtocol::Vnc => keystroke_to_remote_key(keystroke),
    }
}

pub fn keystroke_to_remote_key(keystroke: &Keystroke) -> Option<RemoteKey> {
    if let Some(character) = keystroke
        .key_char
        .as_deref()
        .and_then(single_character)
        .filter(|ch| !ch.is_control())
    {
        return Some(RemoteKey::Character(character));
    }

    if let Some(character) = single_character(&keystroke.key).filter(|ch| !ch.is_control()) {
        return Some(RemoteKey::Character(character));
    }

    Some(RemoteKey::Named(match keystroke.key.as_str() {
        "escape" => RemoteNamedKey::Escape,
        "backspace" => RemoteNamedKey::Backspace,
        "tab" => RemoteNamedKey::Tab,
        "enter" => RemoteNamedKey::Enter,
        "space" => RemoteNamedKey::Space,
        "insert" => RemoteNamedKey::Insert,
        "delete" => RemoteNamedKey::Delete,
        "home" => RemoteNamedKey::Home,
        "end" => RemoteNamedKey::End,
        "pageup" => RemoteNamedKey::PageUp,
        "pagedown" => RemoteNamedKey::PageDown,
        "up" | "arrowup" => RemoteNamedKey::ArrowUp,
        "down" | "arrowdown" => RemoteNamedKey::ArrowDown,
        "left" | "arrowleft" => RemoteNamedKey::ArrowLeft,
        "right" | "arrowright" => RemoteNamedKey::ArrowRight,
        "shift" => RemoteNamedKey::Shift,
        "control" | "ctrl" => RemoteNamedKey::Control,
        "alt" | "option" => RemoteNamedKey::Alt,
        "cmd" | "super" | "meta" => RemoteNamedKey::Meta,
        "capslock" => RemoteNamedKey::CapsLock,
        key if key.starts_with('f') => RemoteNamedKey::F(key[1..].parse().ok()?),
        _ => return None,
    }))
}

fn single_character(value: &str) -> Option<char> {
    let mut chars = value.chars();
    let ch = chars.next()?;
    chars.next().is_none().then_some(ch)
}

fn rdp_key_to_scancode(key: &str) -> Option<u16> {
    let normalized = key.to_ascii_lowercase();
    let code = match normalized.as_str() {
        "escape" | "esc" => 0x0001,
        "1" | "!" => 0x0002,
        "2" | "@" => 0x0003,
        "3" | "#" => 0x0004,
        "4" | "$" => 0x0005,
        "5" | "%" => 0x0006,
        "6" | "^" => 0x0007,
        "7" | "&" => 0x0008,
        "8" | "*" => 0x0009,
        "9" | "(" => 0x000a,
        "0" | ")" => 0x000b,
        "-" | "_" => 0x000c,
        "=" | "+" => 0x000d,
        "backspace" => 0x000e,
        "tab" => 0x000f,
        "q" => 0x0010,
        "w" => 0x0011,
        "e" => 0x0012,
        "r" => 0x0013,
        "t" => 0x0014,
        "y" => 0x0015,
        "u" => 0x0016,
        "i" => 0x0017,
        "o" => 0x0018,
        "p" => 0x0019,
        "[" | "{" => 0x001a,
        "]" | "}" => 0x001b,
        "enter" | "return" => 0x001c,
        "control" | "ctrl" => 0x001d,
        "a" => 0x001e,
        "s" => 0x001f,
        "d" => 0x0020,
        "f" => 0x0021,
        "g" => 0x0022,
        "h" => 0x0023,
        "j" => 0x0024,
        "k" => 0x0025,
        "l" => 0x0026,
        ";" | ":" => 0x0027,
        "'" | "\"" => 0x0028,
        "`" | "~" => 0x0029,
        "shift" => 0x002a,
        "\\" | "|" => 0x002b,
        "z" => 0x002c,
        "x" => 0x002d,
        "c" => 0x002e,
        "v" => 0x002f,
        "b" => 0x0030,
        "n" => 0x0031,
        "m" => 0x0032,
        "," | "<" => 0x0033,
        "." | ">" => 0x0034,
        "/" | "?" => 0x0035,
        "alt" | "option" => 0x0038,
        "space" | " " => 0x0039,
        "capslock" | "caps_lock" => 0x003a,
        "f1" => 0x003b,
        "f2" => 0x003c,
        "f3" => 0x003d,
        "f4" => 0x003e,
        "f5" => 0x003f,
        "f6" => 0x0040,
        "f7" => 0x0041,
        "f8" => 0x0042,
        "f9" => 0x0043,
        "f10" => 0x0044,
        "f11" => 0x0057,
        "f12" => 0x0058,
        "home" => 0xe047,
        "up" | "arrowup" => 0xe048,
        "pageup" | "page_up" => 0xe049,
        "left" | "arrowleft" => 0xe04b,
        "right" | "arrowright" => 0xe04d,
        "end" => 0xe04f,
        "down" | "arrowdown" => 0xe050,
        "pagedown" | "page_down" => 0xe051,
        "insert" => 0xe052,
        "delete" | "del" => 0xe053,
        "cmd" | "super" | "win" | "meta" | "platform" => 0xe05b,
        _ => return None,
    };

    Some(code)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::Keystroke;
    use remote_desktop::RemoteDesktopProtocol;

    #[test]
    fn maps_rdp_keystrokes_to_windows_scancodes() {
        assert_eq!(key("space"), Some(RemoteKey::Scancode(0x0039)));
        assert_eq!(key("?"), Some(RemoteKey::Scancode(0x0035)));
        assert_eq!(key("up"), Some(RemoteKey::Scancode(0xe048)));
        assert_eq!(key("tab"), Some(RemoteKey::Scancode(0x000f)));
    }

    fn key(source: &str) -> Option<RemoteKey> {
        let keystroke = Keystroke::parse(source).expect("valid test keystroke");
        keystroke_to_remote_key_for_protocol(&keystroke, RemoteDesktopProtocol::Rdp)
    }

    #[test]
    fn maps_vnc_tab_to_named_remote_key() {
        let keystroke = Keystroke::parse("tab").expect("valid test keystroke");

        assert_eq!(
            Some(RemoteKey::Named(RemoteNamedKey::Tab)),
            keystroke_to_remote_key_for_protocol(&keystroke, RemoteDesktopProtocol::Vnc)
        );
    }
}
