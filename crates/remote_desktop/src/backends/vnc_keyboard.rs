use crate::{RemoteKey, RemoteNamedKey};

pub fn remote_key_to_keysym(key: &RemoteKey) -> Option<u32> {
    Some(match key {
        RemoteKey::KeySym(value) => *value,
        RemoteKey::Character(ch) if ch.is_ascii() => *ch as u32,
        RemoteKey::Named(named) => match named {
            RemoteNamedKey::Backspace => 0xff08,
            RemoteNamedKey::Tab => 0xff09,
            RemoteNamedKey::Enter => 0xff0d,
            RemoteNamedKey::Escape => 0xff1b,
            RemoteNamedKey::Insert => 0xff63,
            RemoteNamedKey::Delete => 0xffff,
            RemoteNamedKey::Home => 0xff50,
            RemoteNamedKey::End => 0xff57,
            RemoteNamedKey::PageUp => 0xff55,
            RemoteNamedKey::PageDown => 0xff56,
            RemoteNamedKey::ArrowLeft => 0xff51,
            RemoteNamedKey::ArrowUp => 0xff52,
            RemoteNamedKey::ArrowRight => 0xff53,
            RemoteNamedKey::ArrowDown => 0xff54,
            RemoteNamedKey::Space => 0x20,
            RemoteNamedKey::Shift => 0xffe1,
            RemoteNamedKey::Control => 0xffe3,
            RemoteNamedKey::Alt => 0xffe9,
            RemoteNamedKey::Meta => 0xffeb,
            RemoteNamedKey::CapsLock => 0xffe5,
            RemoteNamedKey::F(n) if (1..=12).contains(n) => 0xffbe + (*n as u32 - 1),
            RemoteNamedKey::F(_) => return None,
        },
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_common_named_keys_to_x11_keysyms() {
        assert_eq!(
            remote_key_to_keysym(&RemoteKey::Named(RemoteNamedKey::Enter)),
            Some(0xff0d)
        );
        assert_eq!(
            remote_key_to_keysym(&RemoteKey::Named(RemoteNamedKey::ArrowUp)),
            Some(0xff52)
        );
        assert_eq!(
            remote_key_to_keysym(&RemoteKey::Named(RemoteNamedKey::F(5))),
            Some(0xffc2)
        );
    }

    #[test]
    fn maps_ascii_character_to_keysym() {
        assert_eq!(remote_key_to_keysym(&RemoteKey::Character('a')), Some(0x61));
        assert_eq!(remote_key_to_keysym(&RemoteKey::Character('A')), Some(0x41));
    }
}
