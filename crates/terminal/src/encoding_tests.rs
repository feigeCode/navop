use std::borrow::Cow;

use crate::encoding::{TerminalEncoding, TerminalOutputDecoder, encode_terminal_input};
use crate::exec_supervisor::TerminalInputSource;

#[test]
fn streaming_decoder_preserves_split_euc_jp_and_gb18030_characters() {
    let mut euc_jp = TerminalOutputDecoder::new(TerminalEncoding::EucJp);
    assert!(euc_jp.decode(&[0xA4]).is_empty());
    assert_eq!(euc_jp.decode(&[0xA2]), "あ".as_bytes());

    let mut gb18030 = TerminalOutputDecoder::new(TerminalEncoding::Gb18030);
    assert!(gb18030.decode(&[0x94, 0x39]).is_empty());
    assert_eq!(gb18030.decode(&[0xFC, 0x36]), "😀".as_bytes());
}

#[test]
fn decoder_preserves_terminal_protocol_bytes_around_euc_jp_text() {
    let mut decoder = TerminalOutputDecoder::new(TerminalEncoding::EucJp);
    assert_eq!(
        decoder.decode(b"\x1b]0;\xA4\xA2\x07"),
        b"\x1b]0;\xE3\x81\x82\x07"
    );
}

#[test]
fn source_aware_input_encoding_transcodes_only_textual_sources() {
    assert_eq!(
        encode_terminal_input(
            TerminalEncoding::EucJp,
            TerminalInputSource::User,
            "あ".as_bytes(),
        )
        .as_ref(),
        &[0xA4, 0xA2]
    );
    assert_eq!(
        encode_terminal_input(
            TerminalEncoding::EucJp,
            TerminalInputSource::AgentCommand,
            "あ\r".as_bytes(),
        )
        .as_ref(),
        &[0xA4, 0xA2, b'\r']
    );
    assert!(matches!(
        encode_terminal_input(
            TerminalEncoding::EucJp,
            TerminalInputSource::TerminalResponse,
            b"\x1b[0n",
        ),
        Cow::Borrowed(_)
    ));
    assert!(matches!(
        encode_terminal_input(
            TerminalEncoding::EucJp,
            TerminalInputSource::AgentPreflight,
            &[0x03],
        ),
        Cow::Borrowed(_)
    ));

    let invalid_utf8 = [0xFF, 0x00, 0x1B];
    assert_eq!(
        encode_terminal_input(
            TerminalEncoding::EucJp,
            TerminalInputSource::User,
            &invalid_utf8,
        )
        .as_ref(),
        invalid_utf8
    );
}
