#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    pub event: String,
    pub data: String,
    pub id: Option<String>,
}

#[derive(Debug, Default)]
pub struct SseParser {
    buffer: Vec<u8>,
    pending: PendingEvent,
}

#[derive(Debug, Default)]
struct PendingEvent {
    event: Option<String>,
    data: Vec<String>,
    id: Option<String>,
    saw_data: bool,
}

impl SseParser {
    pub fn push(&mut self, chunk: &[u8]) -> Vec<SseEvent> {
        self.buffer.extend_from_slice(chunk);
        let mut events = Vec::new();
        while let Some((line_end, delimiter_len)) = next_line(&self.buffer) {
            let line = String::from_utf8_lossy(&self.buffer[..line_end]).into_owned();
            self.buffer.drain(..line_end + delimiter_len);
            self.process_line(&line, &mut events);
        }
        events
    }

    pub fn finish(&mut self) -> Vec<SseEvent> {
        let mut events = Vec::new();
        if !self.buffer.is_empty() {
            let line_end = self
                .buffer
                .len()
                .saturating_sub(usize::from(self.buffer.ends_with(b"\r")));
            let line = String::from_utf8_lossy(&self.buffer[..line_end]).into_owned();
            self.buffer.clear();
            self.process_line(&line, &mut events);
        }
        self.dispatch(&mut events);
        events
    }

    fn process_line(&mut self, line: &str, events: &mut Vec<SseEvent>) {
        if line.is_empty() {
            self.dispatch(events);
            return;
        }
        if line.starts_with(':') {
            return;
        }
        let (field, value) = split_field(line);
        match field {
            "event" => self.pending.event = Some(value.to_string()),
            "data" => {
                self.pending.saw_data = true;
                self.pending.data.push(value.to_string());
            }
            "id" if !value.contains('\0') => self.pending.id = Some(value.to_string()),
            _ => {}
        }
    }

    fn dispatch(&mut self, events: &mut Vec<SseEvent>) {
        if !self.pending.saw_data {
            self.pending = PendingEvent::default();
            return;
        }
        let pending = std::mem::take(&mut self.pending);
        events.push(SseEvent {
            event: pending.event.unwrap_or_else(|| "message".to_string()),
            data: pending.data.join("\n"),
            id: pending.id,
        });
    }
}

fn next_line(buffer: &[u8]) -> Option<(usize, usize)> {
    let index = buffer
        .iter()
        .position(|byte| matches!(byte, b'\r' | b'\n'))?;
    match buffer[index] {
        b'\n' => Some((index, 1)),
        b'\r' if index + 1 == buffer.len() => None,
        b'\r' if buffer[index + 1] == b'\n' => Some((index, 2)),
        b'\r' => Some((index, 1)),
        _ => unreachable!(),
    }
}

fn split_field(line: &str) -> (&str, &str) {
    let Some((field, raw_value)) = line.split_once(':') else {
        return (line, "");
    };
    (field, raw_value.strip_prefix(' ').unwrap_or(raw_value))
}

#[cfg(test)]
mod tests {
    use super::{SseEvent, SseParser};

    #[test]
    fn parser_handles_multiline_data_and_metadata() {
        let mut parser = SseParser::default();
        let events = parser.push(b"id: 42\nevent: update\ndata: first\ndata: second\n\n");

        assert_eq!(
            events,
            vec![SseEvent {
                event: "update".into(),
                data: "first\nsecond".into(),
                id: Some("42".into()),
            }]
        );
    }

    #[test]
    fn parser_handles_crlf_chunk_boundaries_and_comments() {
        let mut parser = SseParser::default();
        assert!(parser.push(b": keep-alive\r\ndata: hel").is_empty());
        assert!(parser.push(b"lo\r").is_empty());
        assert_eq!(
            parser.push(b"\n\r\n"),
            vec![SseEvent {
                event: "message".into(),
                data: "hello".into(),
                id: None,
            }]
        );
    }

    #[test]
    fn parser_flushes_final_event_at_eof() {
        let mut parser = SseParser::default();
        assert!(parser.push(b"data: tail").is_empty());
        assert_eq!(
            parser.finish(),
            vec![SseEvent {
                event: "message".into(),
                data: "tail".into(),
                id: None,
            }]
        );
    }

    #[test]
    fn parser_treats_trailing_carriage_return_as_a_line_ending_at_eof() {
        let mut parser = SseParser::default();
        assert!(parser.push(b"data: tail\r").is_empty());

        assert_eq!(
            parser.finish(),
            vec![SseEvent {
                event: "message".into(),
                data: "tail".into(),
                id: None,
            }]
        );
    }

    #[test]
    fn parser_preserves_empty_data_and_only_strips_one_space() {
        let mut parser = SseParser::default();
        let events = parser.push(b"data:\n\ndata:  padded\n\n");

        assert_eq!(events.len(), 2);
        assert_eq!(events[0].data, "");
        assert_eq!(events[1].data, " padded");
    }
}
