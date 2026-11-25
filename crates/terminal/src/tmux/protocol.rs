#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TmuxControlLine {
    Begin {
        epoch: u64,
        cmdno: u64,
        flags: String,
    },
    End {
        epoch: u64,
        cmdno: u64,
        flags: String,
    },
    Error {
        epoch: u64,
        cmdno: u64,
        flags: String,
    },
    Output {
        pane: String,
        data: Vec<u8>,
    },
    WindowAdded {
        id: String,
    },
    WindowClosed {
        id: String,
    },
    WindowRenamed {
        id: String,
        name: String,
    },
    WindowPaneChanged {
        window: String,
        pane: String,
    },
    LayoutChanged {
        window: String,
        layout: String,
        visible_layout: String,
        flags: String,
    },
    PaneModeChanged {
        pane: String,
    },
    SessionChanged {
        id: String,
        name: String,
    },
    SessionRenamed {
        id: Option<String>,
        name: String,
    },
    SessionWindowChanged {
        session: String,
        window: String,
    },
    SessionsChanged,
    SubscriptionChanged {
        name: String,
        session: String,
        window: String,
        window_index: String,
        pane: Option<String>,
        value: String,
    },
    Exit {
        reason: Option<String>,
    },
    Other(String),
}

pub fn parse_control_line(line: &[u8]) -> Option<TmuxControlLine> {
    let line = line.strip_suffix(b"\n").unwrap_or(line);
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    if line.is_empty() {
        return None;
    }
    let text = String::from_utf8_lossy(line);
    if let Some(rest) = text.strip_prefix("%begin ") {
        let (epoch, cmdno, flags) = parse_begin_end(rest)?;
        return Some(TmuxControlLine::Begin {
            epoch,
            cmdno,
            flags,
        });
    }
    if let Some(rest) = text.strip_prefix("%end ") {
        let (epoch, cmdno, flags) = parse_begin_end(rest)?;
        return Some(TmuxControlLine::End {
            epoch,
            cmdno,
            flags,
        });
    }
    if let Some(rest) = text.strip_prefix("%error ") {
        let (epoch, cmdno, flags) = parse_begin_end(rest)?;
        return Some(TmuxControlLine::Error {
            epoch,
            cmdno,
            flags,
        });
    }
    if let Some(rest) = text.strip_prefix("%output ") {
        let (pane, data) = rest.split_once(' ')?;
        return Some(TmuxControlLine::Output {
            pane: pane.to_string(),
            data: unescape_output(data.as_bytes()),
        });
    }
    if let Some(rest) = text.strip_prefix("%window-add ") {
        return Some(TmuxControlLine::WindowAdded {
            id: rest.trim().to_string(),
        });
    }
    if let Some(rest) = text.strip_prefix("%window-close ") {
        return Some(TmuxControlLine::WindowClosed {
            id: rest.trim().to_string(),
        });
    }
    if let Some(rest) = text.strip_prefix("%window-renamed ") {
        let (id, name) = rest.split_once(' ')?;
        return Some(TmuxControlLine::WindowRenamed {
            id: id.to_string(),
            name: name.to_string(),
        });
    }
    if let Some(rest) = text.strip_prefix("%window-pane-changed ") {
        let (window, pane) = rest.split_once(' ')?;
        return Some(TmuxControlLine::WindowPaneChanged {
            window: window.to_string(),
            pane: pane.trim().to_string(),
        });
    }
    if let Some(rest) = text.strip_prefix("%layout-change ") {
        let mut parts = rest.splitn(4, ' ');
        return Some(TmuxControlLine::LayoutChanged {
            window: parts.next()?.to_string(),
            layout: parts.next()?.to_string(),
            visible_layout: parts.next()?.to_string(),
            flags: parts.next().unwrap_or_default().to_string(),
        });
    }
    if let Some(rest) = text.strip_prefix("%pane-mode-changed ") {
        return Some(TmuxControlLine::PaneModeChanged {
            pane: rest.trim().to_string(),
        });
    }
    if let Some(rest) = text.strip_prefix("%session-changed ") {
        let (id, name) = rest.split_once(' ')?;
        return Some(TmuxControlLine::SessionChanged {
            id: id.to_string(),
            name: name.to_string(),
        });
    }
    if let Some(rest) = text.strip_prefix("%session-renamed ") {
        let mut parts = rest.splitn(2, ' ');
        let first = parts.next()?.to_string();
        let second = parts.next().map(str::to_string);
        return Some(match second {
            Some(name) if first.starts_with('$') => TmuxControlLine::SessionRenamed {
                id: Some(first),
                name,
            },
            Some(name) => TmuxControlLine::SessionRenamed {
                id: None,
                name: format!("{first} {name}"),
            },
            None => TmuxControlLine::SessionRenamed {
                id: None,
                name: first,
            },
        });
    }
    if let Some(rest) = text.strip_prefix("%session-window-changed ") {
        let (session, window) = rest.split_once(' ')?;
        return Some(TmuxControlLine::SessionWindowChanged {
            session: session.to_string(),
            window: window.trim().to_string(),
        });
    }
    if text.starts_with("%sessions-changed") {
        return Some(TmuxControlLine::SessionsChanged);
    }
    if let Some(rest) = text.strip_prefix("%subscription-changed ") {
        return parse_subscription(rest);
    }
    if let Some(rest) = text.strip_prefix("%exit") {
        let reason = rest.trim().strip_prefix(' ').map(str::to_string);
        return Some(TmuxControlLine::Exit { reason });
    }
    Some(TmuxControlLine::Other(text.into_owned()))
}

fn parse_begin_end(rest: &str) -> Option<(u64, u64, String)> {
    let mut parts = rest.split_whitespace();
    Some((
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        parts.next().unwrap_or_default().to_string(),
    ))
}

fn parse_subscription(rest: &str) -> Option<TmuxControlLine> {
    let (targets, value) = rest.split_once(" : ")?;
    let mut parts = targets.split_whitespace();
    let name = parts.next()?.to_string();
    let session = parts.next()?.to_string();
    let window = parts.next()?.to_string();
    let window_index = parts.next()?.to_string();
    let pane = parts.next().map(str::to_string);
    Some(TmuxControlLine::SubscriptionChanged {
        name,
        session,
        window,
        window_index,
        pane,
        value: value.to_string(),
    })
}

pub fn unescape_output(data: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(data.len());
    let mut index = 0;
    while index < data.len() {
        if data[index] == b'\\'
            && index + 3 < data.len()
            && data[index + 1..index + 4].iter().all(u8::is_ascii_digit)
        {
            let value = data[index + 1..index + 4]
                .iter()
                .fold(0u16, |value, digit| value * 8 + u16::from(digit - b'0'));
            if value <= u8::MAX.into() {
                output.push(value as u8);
                index += 4;
                continue;
            }
        }
        output.push(data[index]);
        index += 1;
    }
    output
}

#[cfg(test)]
mod tests {
    use super::{TmuxControlLine, parse_control_line, unescape_output};

    #[test]
    fn parses_command_blocks_and_preserves_command_numbers() {
        assert_eq!(
            parse_control_line(b"%begin 1578920019 258 0\n"),
            Some(TmuxControlLine::Begin {
                epoch: 1578920019,
                cmdno: 258,
                flags: "0".into()
            })
        );
        assert_eq!(
            parse_control_line(b"%end 1578920019 258 0\r\n"),
            Some(TmuxControlLine::End {
                epoch: 1578920019,
                cmdno: 258,
                flags: "0".into()
            })
        );
    }

    #[test]
    fn parses_subscription_context_and_value() {
        assert_eq!(
            parse_control_line(
                b"%subscription-changed navop-panes $0 @1 0 %2 : $0|@1|%2|/srv/app\n"
            ),
            Some(TmuxControlLine::SubscriptionChanged {
                name: "navop-panes".into(),
                session: "$0".into(),
                window: "@1".into(),
                window_index: "0".into(),
                pane: Some("%2".into()),
                value: "$0|@1|%2|/srv/app".into(),
            })
        );
    }

    #[test]
    fn parses_session_renamed_forms() {
        assert_eq!(
            parse_control_line(b"%session-renamed $0 main\n"),
            Some(TmuxControlLine::SessionRenamed {
                id: Some("$0".into()),
                name: "main".into()
            })
        );
        assert_eq!(
            parse_control_line(b"%session-renamed main\n"),
            Some(TmuxControlLine::SessionRenamed {
                id: None,
                name: "main".into()
            })
        );
    }

    #[test]
    fn decodes_output_octal_escapes() {
        assert_eq!(unescape_output(b"a\\134b\\003"), b"a\\b\x03");
    }

    #[test]
    fn ignores_unknown_and_empty_lines() {
        assert_eq!(parse_control_line(b"\n"), None);
        assert!(matches!(
            parse_control_line(b"%future-notification value\n"),
            Some(TmuxControlLine::Other(_))
        ));
    }
}
