use crate::TerminalExecCompletion as TerminalExecCaptureCompletion;
use crate::osc::OscEvent;
use anyhow::{Result, anyhow};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

const OSC_COMMAND_FINISHED_PREFIX: &[u8] = b"\x1b]133;D";

#[derive(Clone)]
pub(crate) struct SshTerminalExecCapture {
    inner: Arc<CaptureInner>,
}

struct CaptureInner {
    state: Mutex<CaptureState>,
    changed: Condvar,
}

struct CaptureState {
    active: Option<ActiveCapture>,
    next_id: u64,
}

struct ActiveCapture {
    id: u64,
    command: String,
    started_at: Instant,
    deadline: Instant,
    raw: Vec<u8>,
    command_started: bool,
    completed: Option<CompletedCapture>,
}

struct CompletedCapture {
    completion: TerminalExecCaptureCompletion,
    exit_code: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TerminalExecCaptureResult {
    pub completion: TerminalExecCaptureCompletion,
    pub exit_code: Option<i32>,
    pub output: String,
    pub duration_ms: u64,
}

pub(crate) struct SshTerminalExecCaptureSession {
    id: u64,
    capture: SshTerminalExecCapture,
}

impl SshTerminalExecCapture {
    pub(crate) fn new() -> Self {
        Self {
            inner: Arc::new(CaptureInner {
                state: Mutex::new(CaptureState {
                    active: None,
                    next_id: 1,
                }),
                changed: Condvar::new(),
            }),
        }
    }

    pub(crate) fn start(
        &self,
        command: String,
        timeout: Duration,
    ) -> Result<SshTerminalExecCaptureSession> {
        let now = Instant::now();
        let mut state = self
            .inner
            .state
            .lock()
            .expect("terminal capture lock poisoned");
        if state.active.is_some() {
            return Err(anyhow!("terminal exec capture is already active"));
        }

        let id = state.next_id;
        state.next_id = state.next_id.saturating_add(1);
        state.active = Some(ActiveCapture {
            id,
            command,
            started_at: now,
            deadline: now + timeout,
            raw: Vec::new(),
            command_started: false,
            completed: None,
        });
        Ok(SshTerminalExecCaptureSession {
            id,
            capture: self.clone(),
        })
    }

    pub(crate) fn record_chunk(&self, data: &[u8], events: &[OscEvent]) {
        let mut state = self
            .inner
            .state
            .lock()
            .expect("terminal capture lock poisoned");
        let Some(active) = state.active.as_mut() else {
            return;
        };

        if !data.is_empty() {
            active.raw.extend_from_slice(data);
        }

        for event in events {
            match event {
                OscEvent::CommandStart => active.command_started = true,
                OscEvent::CommandFinished { exit_code } => {
                    active.completed = Some(CompletedCapture {
                        completion: TerminalExecCaptureCompletion::ShellIntegrationExit,
                        exit_code: Some(*exit_code),
                    });
                }
                _ => {}
            }
        }
        drop(state);
        self.inner.changed.notify_all();
    }

    pub(crate) fn cancel(&self, id: u64) {
        let mut state = self
            .inner
            .state
            .lock()
            .expect("terminal capture lock poisoned");
        if state.active.as_ref().is_some_and(|active| active.id == id) {
            state.active = None;
        }
        drop(state);
        self.inner.changed.notify_all();
    }

    pub(crate) fn finish_active_on_disconnect(&self) {
        let mut state = self
            .inner
            .state
            .lock()
            .expect("terminal capture lock poisoned");
        if let Some(active) = state.active.as_mut() {
            active.completed = Some(CompletedCapture {
                completion: TerminalExecCaptureCompletion::TimedOut,
                exit_code: None,
            });
        }
        drop(state);
        self.inner.changed.notify_all();
    }

    fn wait_for_result(&self, id: u64) -> TerminalExecCaptureResult {
        let mut state = self
            .inner
            .state
            .lock()
            .expect("terminal capture lock poisoned");
        loop {
            let now = Instant::now();
            let action = capture_wait_action(&state, id, now);
            match action {
                CaptureWaitAction::Finish(completed) => {
                    return finish_capture(&mut state, id, completed);
                }
                CaptureWaitAction::Wait(timeout) => {
                    let (next_state, _) = self
                        .inner
                        .changed
                        .wait_timeout(state, timeout)
                        .expect("terminal capture lock poisoned while waiting");
                    state = next_state;
                }
            }
        }
    }
}

impl SshTerminalExecCaptureSession {
    pub(crate) fn id(&self) -> u64 {
        self.id
    }

    pub(crate) fn wait(self) -> TerminalExecCaptureResult {
        self.capture.wait_for_result(self.id)
    }
}

enum CaptureWaitAction {
    Finish(CompletedCapture),
    Wait(Duration),
}

fn capture_wait_action(state: &CaptureState, id: u64, now: Instant) -> CaptureWaitAction {
    let Some(active) = state.active.as_ref().filter(|active| active.id == id) else {
        return CaptureWaitAction::Finish(CompletedCapture {
            completion: TerminalExecCaptureCompletion::TimedOut,
            exit_code: None,
        });
    };

    if let Some(completed) = &active.completed {
        return CaptureWaitAction::Finish(CompletedCapture {
            completion: completed.completion,
            exit_code: completed.exit_code,
        });
    }

    if now >= active.deadline {
        return CaptureWaitAction::Finish(CompletedCapture {
            completion: TerminalExecCaptureCompletion::TimedOut,
            exit_code: None,
        });
    }

    if text_prompt_has_returned(active) {
        return CaptureWaitAction::Finish(CompletedCapture {
            completion: TerminalExecCaptureCompletion::ObservedOutput,
            exit_code: None,
        });
    }

    CaptureWaitAction::Wait(active.deadline.saturating_duration_since(now))
}

fn text_prompt_has_returned(active: &ActiveCapture) -> bool {
    if active.command_started {
        return false;
    }
    captured_terminal_text(&active.raw, &active.command)
        .as_deref()
        .is_some_and(has_trailing_shell_prompt)
}

fn finish_capture(
    state: &mut CaptureState,
    id: u64,
    completed: CompletedCapture,
) -> TerminalExecCaptureResult {
    let Some(active) = state.active.take().filter(|active| active.id == id) else {
        return TerminalExecCaptureResult {
            completion: completed.completion,
            exit_code: completed.exit_code,
            output: String::new(),
            duration_ms: 0,
        };
    };

    TerminalExecCaptureResult {
        completion: completed.completion,
        exit_code: completed.exit_code,
        output: sanitize_captured_terminal_output(&active.raw, &active.command),
        duration_ms: active
            .started_at
            .elapsed()
            .as_millis()
            .try_into()
            .unwrap_or(u64::MAX),
    }
}

pub(crate) fn sanitize_captured_terminal_output(raw: &[u8], command: &str) -> String {
    let Some(text) = captured_terminal_text(raw, command) else {
        return String::new();
    };
    strip_trailing_shell_prompt(&text).trim().to_string()
}

fn captured_terminal_text(raw: &[u8], command: &str) -> Option<String> {
    let raw = truncate_at_command_finished(raw);
    let stripped = strip_terminal_escape_sequences(raw);
    let text = String::from_utf8_lossy(&stripped);
    let text = apply_backspace(&text);
    let text = normalize_terminal_newlines(&text);
    let text = strip_remaining_controls(&text);
    let output = strip_command_echo(&text, command);
    (!output.trim().is_empty()).then_some(output)
}

fn truncate_at_command_finished(raw: &[u8]) -> &[u8] {
    raw.windows(OSC_COMMAND_FINISHED_PREFIX.len())
        .position(|window| window == OSC_COMMAND_FINISHED_PREFIX)
        .map_or(raw, |index| &raw[..index])
}

fn strip_terminal_escape_sequences(input: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(input.len());
    let mut index = 0;
    while index < input.len() {
        match input[index] {
            0x1b => index = skip_escape_sequence(input, index + 1),
            byte => {
                output.push(byte);
                index += 1;
            }
        }
    }
    output
}

fn skip_escape_sequence(input: &[u8], mut index: usize) -> usize {
    if index >= input.len() {
        return index;
    }
    match input[index] {
        b'[' => skip_csi(input, index + 1),
        b']' => skip_string_escape(input, index + 1, true),
        b'P' | b'^' | b'_' | b'X' => skip_string_escape(input, index + 1, false),
        _ => {
            index += 1;
            index
        }
    }
}

fn skip_csi(input: &[u8], mut index: usize) -> usize {
    while index < input.len() {
        let byte = input[index];
        index += 1;
        if (0x40..=0x7e).contains(&byte) {
            break;
        }
    }
    index
}

fn skip_string_escape(input: &[u8], mut index: usize, bel_terminated: bool) -> usize {
    while index < input.len() {
        if bel_terminated && input[index] == 0x07 {
            return index + 1;
        }
        if input[index] == 0x1b && input.get(index + 1) == Some(&b'\\') {
            return index + 2;
        }
        index += 1;
    }
    index
}

fn apply_backspace(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '\u{8}' | '\u{7f}' => {
                output.pop();
            }
            _ => output.push(ch),
        }
    }
    output
}

fn normalize_terminal_newlines(input: &str) -> String {
    input.replace("\r\n", "\n").replace('\r', "\n")
}

fn strip_remaining_controls(input: &str) -> String {
    input
        .chars()
        .filter(|ch| *ch == '\n' || *ch == '\t' || !ch.is_control())
        .collect()
}

fn strip_command_echo(output: &str, command: &str) -> String {
    let Some((_command_offset, command_end)) = find_wrapped_command_echo(output, command) else {
        return output.to_string();
    };
    strip_echo_separator(&output[command_end..])
}

fn find_wrapped_command_echo(output: &str, command: &str) -> Option<(usize, usize)> {
    const SEARCH_LIMIT_BYTES: usize = 8192;
    let command = command.trim();
    let first = command.chars().find(|ch| !ch.is_whitespace())?;
    for (start, ch) in output.char_indices() {
        if start > SEARCH_LIMIT_BYTES {
            break;
        }
        if ch == first {
            if let Some(end) = match_wrapped_command_from(output, start, command) {
                return Some((start, end));
            }
        }
    }
    None
}

fn match_wrapped_command_from(output: &str, start: usize, command: &str) -> Option<usize> {
    let mut chars = output[start..].char_indices().peekable();
    let mut end = start;
    for command_ch in command.chars() {
        if command_ch.is_whitespace() {
            end = consume_echo_whitespace(start, &mut chars)?;
            continue;
        }
        loop {
            let (offset, output_ch) = chars.next()?;
            end = start + offset + output_ch.len_utf8();
            if output_ch == '\n' {
                continue;
            }
            if output_ch != command_ch {
                return None;
            }
            break;
        }
    }
    Some(end)
}

fn consume_echo_whitespace(
    start: usize,
    chars: &mut std::iter::Peekable<std::str::CharIndices<'_>>,
) -> Option<usize> {
    let mut consumed = None;
    while let Some(&(offset, ch)) = chars.peek() {
        if !ch.is_whitespace() {
            break;
        }
        consumed = Some(start + offset + ch.len_utf8());
        chars.next();
    }
    consumed
}

fn strip_echo_separator(rest: &str) -> String {
    let mut start = 0;
    let mut saw_newline = false;
    for (offset, ch) in rest.char_indices() {
        if matches!(ch, ' ' | '\t') && !saw_newline {
            start = offset + ch.len_utf8();
            continue;
        }
        if ch == '\n' {
            saw_newline = true;
            start = offset + ch.len_utf8();
            continue;
        }
        break;
    }
    rest[start..].to_string()
}

fn strip_trailing_shell_prompt(output: &str) -> String {
    trailing_shell_prompt_start(output)
        .map(|index| output[..index].to_string())
        .unwrap_or_else(|| output.to_string())
}

fn has_trailing_shell_prompt(output: &str) -> bool {
    trailing_shell_prompt_start(output).is_some()
}

fn trailing_shell_prompt_start(output: &str) -> Option<usize> {
    let trimmed_end = output.trim_end_matches([' ', '\t']);
    if trimmed_end.ends_with('\n') {
        return None;
    }
    let line_start = trimmed_end.rfind('\n').map(|index| index + 1).unwrap_or(0);
    let line = &trimmed_end[line_start..];
    is_shell_prompt_line(line).then_some(line_start)
}

fn is_shell_prompt_line(line: &str) -> bool {
    let prompt = line.trim();
    let Some(last) = prompt.chars().last() else {
        return false;
    };
    if prompt.len() > 160 || !matches!(last, '#' | '$' | '%' | '>') {
        return false;
    }
    if matches!(prompt, "#" | "$" | "%" | ">") {
        return true;
    }
    let has_prompt_marker = prompt.contains(['@', '~', ':', '[', ']']);
    let has_whitespace = prompt.chars().any(char::is_whitespace);
    has_prompt_marker || !has_whitespace
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::osc::OscEvent;
    use std::time::Duration;

    fn test_capture() -> SshTerminalExecCapture {
        SshTerminalExecCapture::new()
    }

    #[test]
    fn stream_capture_returns_shell_exit_output_from_raw_pty_chunks() {
        let capture = test_capture();
        let session = capture
            .start("df -h".to_string(), Duration::from_millis(200))
            .expect("capture should start");

        capture.record_chunk(
            b"\x1b[32mroot@prod:~# df -h\r\nFilesystem  Size\r\n/dev/vda1   40G\r\n\x1b]133;D;0\x07",
            &[OscEvent::CommandFinished { exit_code: 0 }],
        );

        let result = session.wait();

        assert_eq!(
            TerminalExecCaptureCompletion::ShellIntegrationExit,
            result.completion
        );
        assert_eq!(Some(0), result.exit_code);
        assert_eq!("Filesystem  Size\n/dev/vda1   40G", result.output);
    }

    #[test]
    fn stream_capture_observes_output_when_prompt_returns_without_shell_integration() {
        let capture = test_capture();
        let session = capture
            .start(
                "systemctl list-units --type=service".to_string(),
                Duration::from_millis(200),
            )
            .expect("capture should start");

        capture.record_chunk(
            b"systemctl list-units --type=service\r\nssh.service loaded active running\r\nroot@prod:~# ",
            &[],
        );

        let result = session.wait();

        assert_eq!(
            TerminalExecCaptureCompletion::ObservedOutput,
            result.completion
        );
        assert_eq!("ssh.service loaded active running", result.output);
        assert_eq!(None, result.exit_code);
    }

    #[test]
    fn stream_capture_does_not_treat_quiet_output_as_complete_without_prompt() {
        let capture = SshTerminalExecCapture::new();
        let session = capture
            .start(
                "systemctl list-units --type=service".to_string(),
                Duration::from_millis(20),
            )
            .expect("capture should start");

        capture.record_chunk(
            b"systemctl list-units --type=service\r\nssh.service loaded active running\r\n",
            &[],
        );

        let result = session.wait();

        assert_eq!(TerminalExecCaptureCompletion::TimedOut, result.completion);
        assert_eq!("ssh.service loaded active running", result.output);
    }

    #[test]
    fn stream_capture_waits_for_prompt_before_returning_long_output() {
        let capture = SshTerminalExecCapture::new();
        let session = capture
            .start(
                "systemctl list-units --type=service".to_string(),
                Duration::from_millis(300),
            )
            .expect("capture should start");
        let (result_tx, result_rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = result_tx.send(session.wait());
        });

        capture.record_chunk(
            b"[root@zn-53 ~]# systemctl list-units --type=service\r\nNetworkManager.service loaded active running Network Manager\r\n",
            &[],
        );
        std::thread::sleep(Duration::from_millis(30));
        assert!(
            result_rx.try_recv().is_err(),
            "capture must not finish before prompt returns"
        );

        capture.record_chunk(
            b"wpa_supplicant.service loaded active running WPA Supplicant daemon\r\n44 loaded units listed.\r\n[root@zn-53 ~]# ",
            &[],
        );
        let result = result_rx
            .recv_timeout(Duration::from_millis(200))
            .expect("capture should finish after prompt returns");

        assert_eq!(
            TerminalExecCaptureCompletion::ObservedOutput,
            result.completion
        );
        assert!(result.output.contains("NetworkManager.service"));
        assert!(result.output.contains("wpa_supplicant.service"));
        assert!(result.output.contains("44 loaded units listed."));
        assert!(!result.output.contains("[root@zn-53 ~]#"));
    }

    #[test]
    fn stream_capture_times_out_with_partial_output() {
        let capture = SshTerminalExecCapture::new();
        let session = capture
            .start(
                "tail -f /tmp/app.log".to_string(),
                Duration::from_millis(20),
            )
            .expect("capture should start");

        capture.record_chunk(
            b"root@prod:~# tail -f /tmp/app.log\r\npartial line\r\n",
            &[],
        );

        let result = session.wait();

        assert_eq!(TerminalExecCaptureCompletion::TimedOut, result.completion);
        assert_eq!("partial line", result.output);
    }

    #[test]
    fn stream_capture_rejects_concurrent_terminal_exec() {
        let capture = test_capture();
        let _session = capture
            .start("sleep 1".to_string(), Duration::from_millis(200))
            .expect("first capture should start");

        let error = match capture.start("df -h".to_string(), Duration::from_millis(200)) {
            Ok(_) => panic!("second capture should be rejected"),
            Err(error) => error,
        };

        assert!(
            error
                .to_string()
                .contains("terminal exec capture is already active")
        );
    }

    #[test]
    fn strip_terminal_controls_removes_ansi_osc_and_backspace() {
        let output = sanitize_captured_terminal_output(
            b"\x1b[31mroot# echo hi\r\nhix\x08\r\n\x1b]133;D;0\x07root# ",
            "echo hi",
        );

        assert_eq!("hi", output);
    }

    #[test]
    fn strip_command_echo_handles_wrapped_terminal_input() {
        let command = r#"systemctl list-units --type=service --state=running --no-pager | grep -c "\.service""#;
        let raw = b"[root@zn-53 ~]#systemctl list-units --type=service --state=running --no-pager | grep -c \"\\.serv\r\nice\"\r\n44\r\n[root@zn-53 ~]# ";

        let output = sanitize_captured_terminal_output(raw, command);

        assert_eq!("44", output);
    }
}
