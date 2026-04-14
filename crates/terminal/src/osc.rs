//! 共享 OSC 事件解析模块
//!
//! 提取自 ssh_backend.rs，供 SSH 和本地终端后端共用。
//! 支持 OSC 133（shell 集成协议）、OSC 7（工作目录）和 OSC 1337（命令记录）。

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;

/// OSC 事件类型（基于 OSC 133 协议）
#[derive(Debug, PartialEq, Eq)]
pub enum OscEvent {
    /// 提示符开始（OSC 133;A）
    PromptStart,
    /// 输入区域开始（OSC 133;B）
    InputStart,
    /// 命令执行开始（OSC 133;C）
    CommandStart,
    /// 命令执行完毕（OSC 133;D;<exit_code>）
    CommandFinished { exit_code: i32 },
    /// 工作目录变更（OSC 7;file://host/path）
    WorkingDirChanged(String),
    /// 记录 shell 实际执行过的命令（OSC 1337;Command=<base64>）
    CommandRecorded(String),
}

/// 从字节流中提取所有 OSC 事件（一次 data 里可能含多个）
pub fn extract_osc_events(data: &[u8]) -> Vec<OscEvent> {
    let text = String::from_utf8_lossy(data);
    let mut events = Vec::new();

    // OSC 格式: ESC ] <payload> BEL  或  ESC ] <payload> ESC \
    // 用简单的状态机扫描
    let mut i = 0;
    let chars: Vec<char> = text.chars().collect();

    while i < chars.len() {
        // 找 ESC ]
        if chars[i] == '\x1b' && i + 1 < chars.len() && chars[i + 1] == ']' {
            i += 2;
            let start = i;

            // 找结束符 BEL(\x07) 或 ST(ESC \)
            while i < chars.len() {
                if chars[i] == '\x07' {
                    let payload: String = chars[start..i].iter().collect();
                    if let Some(ev) = parse_osc_payload(&payload) {
                        events.push(ev);
                    }
                    i += 1;
                    break;
                }
                if chars[i] == '\x1b' && i + 1 < chars.len() && chars[i + 1] == '\\' {
                    let payload: String = chars[start..i].iter().collect();
                    if let Some(ev) = parse_osc_payload(&payload) {
                        events.push(ev);
                    }
                    i += 2;
                    break;
                }
                i += 1;
            }
        } else {
            i += 1;
        }
    }

    events
}

/// 解析 OSC payload 内容
pub fn parse_osc_payload(payload: &str) -> Option<OscEvent> {
    // OSC 133 协议：shell 集成标记
    if let Some(rest) = payload.strip_prefix("133;") {
        return match rest {
            "A" => Some(OscEvent::PromptStart),
            "B" => Some(OscEvent::InputStart),
            "C" => Some(OscEvent::CommandStart),
            d if d.starts_with("D;") => {
                let code = d[2..].parse::<i32>().unwrap_or(-1);
                Some(OscEvent::CommandFinished { exit_code: code })
            }
            _ => None,
        };
    }

    // OSC 7：工作目录变更
    if let Some(rest) = payload.strip_prefix("7;file://") {
        // "hostname/path/to/dir" 或 "/path/to/dir"
        let path = rest
            .split_once('/')
            .map(|(_, p)| format!("/{p}"))
            .unwrap_or_default();
        return Some(OscEvent::WorkingDirChanged(path));
    }

    // OSC 1337：命令记录
    if let Some(encoded) = payload.strip_prefix("1337;Command=") {
        let command = BASE64_STANDARD
            .decode(encoded)
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok())?;
        return Some(OscEvent::CommandRecorded(command));
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_osc_133_prompt_start() {
        assert_eq!(parse_osc_payload("133;A"), Some(OscEvent::PromptStart));
    }

    #[test]
    fn parse_osc_133_input_start() {
        assert_eq!(parse_osc_payload("133;B"), Some(OscEvent::InputStart));
    }

    #[test]
    fn parse_osc_133_command_start() {
        assert_eq!(parse_osc_payload("133;C"), Some(OscEvent::CommandStart));
    }

    #[test]
    fn parse_osc_133_command_finished() {
        assert_eq!(
            parse_osc_payload("133;D;0"),
            Some(OscEvent::CommandFinished { exit_code: 0 })
        );
        assert_eq!(
            parse_osc_payload("133;D;127"),
            Some(OscEvent::CommandFinished { exit_code: 127 })
        );
    }

    #[test]
    fn parse_osc_7_working_dir() {
        assert_eq!(
            parse_osc_payload("7;file://hostname/home/user/project"),
            Some(OscEvent::WorkingDirChanged(
                "/home/user/project".to_string()
            ))
        );
    }

    #[test]
    fn parse_osc_1337_command_recorded() {
        use base64::engine::general_purpose::STANDARD;
        let encoded = STANDARD.encode("git status");
        let payload = format!("1337;Command={encoded}");
        assert_eq!(
            parse_osc_payload(&payload),
            Some(OscEvent::CommandRecorded("git status".to_string()))
        );
    }

    #[test]
    fn extract_multiple_osc_events_from_byte_stream() {
        // 构造含两个 OSC 序列的字节流: ESC ] 133;A BEL ... ESC ] 133;D;0 BEL
        let data = b"\x1b]133;A\x07some output\x1b]133;D;0\x07";
        let events = extract_osc_events(data);

        assert_eq!(events.len(), 2);
        assert_eq!(events[0], OscEvent::PromptStart);
        assert_eq!(events[1], OscEvent::CommandFinished { exit_code: 0 });
    }

    #[test]
    fn extract_osc_with_st_terminator() {
        // ESC ] 133;C ESC \ 格式
        let data = b"\x1b]133;C\x1b\\";
        let events = extract_osc_events(data);

        assert_eq!(events.len(), 1);
        assert_eq!(events[0], OscEvent::CommandStart);
    }

    #[test]
    fn extract_ignores_non_osc_data() {
        let data = b"Hello, world! No OSC here.";
        let events = extract_osc_events(data);
        assert!(events.is_empty());
    }

    #[test]
    fn parse_unknown_osc_returns_none() {
        assert_eq!(parse_osc_payload("999;unknown"), None);
        assert_eq!(parse_osc_payload("133;X"), None);
    }
}
