//! 共享 OSC 事件解析模块
//!
//! 提取自 ssh_backend.rs，供 SSH 和本地终端后端共用。
//! 支持 OSC 133（shell 集成协议）、OSC 7（工作目录）和 OSC 1337（命令记录）。

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;

const MAX_PENDING_OSC_BYTES: usize = 16 * 1024;

/// OSC 事件类型（基于 OSC 133 协议）
#[derive(Clone, Debug, PartialEq, Eq)]
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

/// 跨 PTY/SSH read chunk 保存未完成 OSC 序列的增量解析器。
///
/// ConPTY 和网络 channel 都可能在任意字节边界拆分一次 shell 输出，
/// 因此不能假设 `ESC ] ... BEL` 会完整出现在同一个 read chunk 中。
#[derive(Default)]
pub(crate) struct OscStreamParser {
    pending: Vec<u8>,
}

impl OscStreamParser {
    pub(crate) fn push(&mut self, data: &[u8]) -> Vec<OscEvent> {
        self.pending.extend_from_slice(data);
        let mut events = Vec::new();
        let mut cursor = 0;

        while let Some(relative_start) = find_osc_start(&self.pending[cursor..]) {
            let start = cursor + relative_start;
            let payload_start = start + 2;
            let Some((payload_end, terminator_len)) = find_osc_end(&self.pending[payload_start..])
            else {
                if start > 0 {
                    self.pending.drain(..start);
                }
                self.truncate_oversized_pending();
                return events;
            };
            let payload_end = payload_start + payload_end;
            if let Ok(payload) = std::str::from_utf8(&self.pending[payload_start..payload_end]) {
                if let Some(event) = parse_osc_payload(payload) {
                    events.push(event);
                }
            }
            cursor = payload_end + terminator_len;
        }

        let retain_from = if self.pending.last() == Some(&b'\x1b') {
            self.pending.len() - 1
        } else {
            self.pending.len()
        };
        if retain_from > 0 {
            self.pending.drain(..retain_from);
        }
        self.truncate_oversized_pending();
        events
    }

    fn truncate_oversized_pending(&mut self) {
        if self.pending.len() <= MAX_PENDING_OSC_BYTES {
            return;
        }
        let retain_from = self.pending.len().saturating_sub(2);
        self.pending.drain(..retain_from);
    }
}

/// 从字节流中提取所有 OSC 事件（一次 data 里可能含多个）
pub fn extract_osc_events(data: &[u8]) -> Vec<OscEvent> {
    OscStreamParser::default().push(data)
}

fn find_osc_start(data: &[u8]) -> Option<usize> {
    data.windows(2).position(|window| window == b"\x1b]")
}

fn find_osc_end(data: &[u8]) -> Option<(usize, usize)> {
    let mut index = 0;
    while index < data.len() {
        if data[index] == b'\x07' {
            return Some((index, 1));
        }
        if data[index] == b'\x1b' && data.get(index + 1).is_some_and(|byte| *byte == b'\\') {
            return Some((index, 2));
        }
        index += 1;
    }
    None
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
        let path = normalize_osc_file_path(path, cfg!(target_os = "windows"));
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

fn normalize_osc_file_path(path: String, windows: bool) -> String {
    if !windows {
        return path;
    }

    let mut path = path.replace('\\', "/");
    let drive_path = path.as_bytes().get(1).is_some_and(u8::is_ascii_alphabetic)
        && path.as_bytes().get(2) == Some(&b':');
    if drive_path || path.starts_with("///") {
        path.remove(0);
    }
    path
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
    fn normalizes_windows_drive_path_from_osc_file_uri() {
        assert_eq!(
            normalize_osc_file_path(r"/C:\Users\alice\project".to_string(), true),
            "C:/Users/alice/project"
        );
    }

    #[test]
    fn keeps_unix_path_when_normalizing_osc_file_uri() {
        assert_eq!(
            normalize_osc_file_path("/home/alice/project".to_string(), false),
            "/home/alice/project"
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
    fn stream_parser_preserves_osc_split_across_chunks() {
        let mut parser = OscStreamParser::default();

        assert!(parser.push(b"\x1b]133;").is_empty());
        assert_eq!(parser.push(b"B\x07"), vec![OscEvent::InputStart]);
        assert!(parser.push(b"\x1b]133;D;").is_empty());
        assert_eq!(
            parser.push(b"0\x1b\\"),
            vec![OscEvent::CommandFinished { exit_code: 0 }]
        );
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
