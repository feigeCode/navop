const OSC_COMMAND_FINISHED_PREFIX: &[u8] = b"\x1b]133;D";

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
