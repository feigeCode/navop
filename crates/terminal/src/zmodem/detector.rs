use zmodem2::{Action, FileInfo, Position, Receiver, Sender};

const MAX_HANDSHAKE_BYTES: usize = 64;
const HEADER_PREFIXES: [&[u8]; 3] = [b"**\x18B", b"*\x18A", b"*\x18C"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ZmodemDirection {
    Upload,
    Download,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct DetectedZmodem {
    pub(crate) direction: ZmodemDirection,
    pub(crate) wire: Vec<u8>,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct ZmodemRouting {
    pub(crate) terminal: Vec<u8>,
    pub(crate) transfer: Option<DetectedZmodem>,
}

#[derive(Default)]
pub(crate) struct ZmodemDetector {
    pending: Vec<u8>,
}

impl ZmodemDetector {
    pub(crate) fn push(&mut self, data: &[u8]) -> ZmodemRouting {
        self.pending.extend_from_slice(data);
        let mut terminal = Vec::new();

        loop {
            let Some(candidate_start) = find_candidate(&self.pending) else {
                let retained = partial_prefix_len(&self.pending);
                let terminal_len = self.pending.len() - retained;
                terminal.extend(self.pending.drain(..terminal_len));
                return ZmodemRouting {
                    terminal,
                    transfer: None,
                };
            };

            terminal.extend(self.pending.drain(..candidate_start));
            match probe_handshake(&self.pending) {
                ProbeResult::Detected(direction) => {
                    return ZmodemRouting {
                        terminal,
                        transfer: Some(DetectedZmodem {
                            direction,
                            wire: std::mem::take(&mut self.pending),
                        }),
                    };
                }
                ProbeResult::Invalid => terminal.push(self.pending.remove(0)),
                ProbeResult::Pending if self.pending.len() < MAX_HANDSHAKE_BYTES => {
                    return ZmodemRouting {
                        terminal,
                        transfer: None,
                    };
                }
                ProbeResult::Pending => terminal.extend(std::mem::take(&mut self.pending)),
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProbeResult {
    Detected(ZmodemDirection),
    Invalid,
    Pending,
}

fn find_candidate(bytes: &[u8]) -> Option<usize> {
    (0..bytes.len()).find(|&index| {
        HEADER_PREFIXES
            .iter()
            .any(|prefix| bytes[index..].starts_with(prefix))
    })
}

fn partial_prefix_len(bytes: &[u8]) -> usize {
    HEADER_PREFIXES
        .iter()
        .flat_map(|prefix| 1..prefix.len())
        .filter(|&len| {
            HEADER_PREFIXES
                .iter()
                .any(|prefix| bytes.ends_with(&prefix[..len]))
        })
        .max()
        .unwrap_or(0)
}

fn probe_handshake(bytes: &[u8]) -> ProbeResult {
    let upload = probe_upload(bytes);
    let download = probe_download(bytes);

    match (upload, download) {
        (_, ProbeState::Detected) => ProbeResult::Detected(ZmodemDirection::Download),
        (ProbeState::Detected, _) => ProbeResult::Detected(ZmodemDirection::Upload),
        (ProbeState::Invalid, ProbeState::Invalid) => ProbeResult::Invalid,
        _ => ProbeResult::Pending,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProbeState {
    Detected,
    Invalid,
    Pending,
}

fn probe_upload(bytes: &[u8]) -> ProbeState {
    let Ok(mut sender) = Sender::new() else {
        return ProbeState::Invalid;
    };
    let probe = FileInfo::new(b"probe", Some(Position::new(1)));
    if sender.start_file(probe).is_err() {
        return ProbeState::Invalid;
    }
    acknowledge_sender_initial(&mut sender);
    if sender.submit_wire(bytes).is_err() {
        return ProbeState::Invalid;
    }

    match sender.poll() {
        Action::WriteWire(_) => ProbeState::Detected,
        _ => ProbeState::Pending,
    }
}

fn probe_download(bytes: &[u8]) -> ProbeState {
    let Ok(mut receiver) = Receiver::new() else {
        return ProbeState::Invalid;
    };
    acknowledge_receiver_initial(&mut receiver);
    if receiver.submit_wire(bytes).is_err() {
        return ProbeState::Invalid;
    }

    match receiver.poll() {
        Action::WriteWire(_) => ProbeState::Detected,
        _ => ProbeState::Pending,
    }
}

fn acknowledge_sender_initial(sender: &mut Sender) {
    let wire_len = match sender.poll() {
        Action::WriteWire(bytes) => bytes.len(),
        _ => return,
    };
    if wire_len > 0 {
        sender.wire_written(wire_len);
    }
}

fn acknowledge_receiver_initial(receiver: &mut Receiver) {
    let wire_len = match receiver.poll() {
        Action::WriteWire(bytes) => bytes.len(),
        _ => return,
    };
    if wire_len > 0 {
        receiver.wire_written(wire_len);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn initial_wire(action: Action<'_>) -> Vec<u8> {
        match action {
            Action::WriteWire(bytes) => bytes.to_vec(),
            other => panic!("expected initial wire, got {other:?}"),
        }
    }

    fn remote_zrqinit() -> Vec<u8> {
        let mut sender = Sender::new().expect("sender");
        initial_wire(sender.poll())
    }

    fn remote_zrinit() -> Vec<u8> {
        let mut receiver = Receiver::new().expect("receiver");
        initial_wire(receiver.poll())
    }

    #[test]
    fn plain_terminal_bytes_pass_through() {
        let mut detector = ZmodemDetector::default();
        assert_eq!(
            detector.push(b"normal output"),
            ZmodemRouting {
                terminal: b"normal output".to_vec(),
                transfer: None,
            }
        );
    }

    #[test]
    fn detects_download_handshake_after_terminal_output() {
        let mut detector = ZmodemDetector::default();
        let mut bytes = b"starting download\r\n".to_vec();
        bytes.extend(remote_zrqinit());

        let routed = detector.push(&bytes);
        let detected = routed.transfer.expect("detected result");

        assert_eq!(detected.direction, ZmodemDirection::Download);
        assert_eq!(routed.terminal, b"starting download\r\n");
        assert_eq!(detected.wire, remote_zrqinit());
    }

    #[test]
    fn detects_upload_handshake_across_chunks() {
        let wire = remote_zrinit();
        let split = wire.len() / 2;
        let mut detector = ZmodemDetector::default();

        let first = detector.push(&wire[..split]);
        assert!(first.terminal.is_empty());
        assert!(first.transfer.is_none());
        let routed = detector.push(&wire[split..]);
        let detected = routed.transfer.expect("completed handshake");

        assert_eq!(detected.direction, ZmodemDirection::Upload);
        assert!(routed.terminal.is_empty());
        assert_eq!(detected.wire, wire);
    }

    #[test]
    fn corrupted_handshake_is_restored_to_terminal_output() {
        let mut wire = remote_zrqinit();
        let crc_index = wire.len().saturating_sub(5);
        wire[crc_index] = if wire[crc_index] == b'0' { b'1' } else { b'0' };
        wire.extend(std::iter::repeat_n(b'x', MAX_HANDSHAKE_BYTES));
        let mut detector = ZmodemDetector::default();

        let routed = detector.push(&wire);

        assert_eq!(routed.terminal, wire);
        assert!(routed.transfer.is_none());
    }
}
