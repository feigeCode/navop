use std::borrow::Cow;

use encoding_rs::{BIG5, Decoder, EUC_JP, EUC_KR, Encoding, GB18030, GBK, SHIFT_JIS, WINDOWS_1252};
use one_core::storage::StoredTerminalEncoding;

use crate::exec_supervisor::TerminalInputSource;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TerminalEncoding {
    #[default]
    Utf8,
    Gbk,
    Gb18030,
    Big5,
    ShiftJis,
    EucJp,
    EucKr,
    Windows1252,
}

impl TerminalEncoding {
    fn encoding(self) -> Option<&'static Encoding> {
        match self {
            Self::Utf8 => None,
            Self::Gbk => Some(GBK),
            Self::Gb18030 => Some(GB18030),
            Self::Big5 => Some(BIG5),
            Self::ShiftJis => Some(SHIFT_JIS),
            Self::EucJp => Some(EUC_JP),
            Self::EucKr => Some(EUC_KR),
            Self::Windows1252 => Some(WINDOWS_1252),
        }
    }
}

impl From<StoredTerminalEncoding> for TerminalEncoding {
    fn from(value: StoredTerminalEncoding) -> Self {
        match value {
            StoredTerminalEncoding::Utf8 => Self::Utf8,
            StoredTerminalEncoding::Gbk => Self::Gbk,
            StoredTerminalEncoding::Gb18030 => Self::Gb18030,
            StoredTerminalEncoding::Big5 => Self::Big5,
            StoredTerminalEncoding::ShiftJis => Self::ShiftJis,
            StoredTerminalEncoding::EucJp => Self::EucJp,
            StoredTerminalEncoding::EucKr => Self::EucKr,
            StoredTerminalEncoding::Windows1252 => Self::Windows1252,
        }
    }
}

pub struct TerminalOutputDecoder {
    decoder: Option<Decoder>,
}

impl TerminalOutputDecoder {
    pub fn new(encoding: TerminalEncoding) -> Self {
        Self {
            decoder: encoding
                .encoding()
                .map(|encoding| encoding.new_decoder_without_bom_handling()),
        }
    }

    pub fn decode(&mut self, input: &[u8]) -> Vec<u8> {
        let Some(decoder) = self.decoder.as_mut() else {
            return input.to_vec();
        };
        decode_to_utf8(decoder, input, false)
    }

    pub fn finish(&mut self) -> Vec<u8> {
        let Some(decoder) = self.decoder.as_mut() else {
            return Vec::new();
        };
        decode_to_utf8(decoder, &[], true)
    }
}

fn decode_to_utf8(decoder: &mut Decoder, input: &[u8], last: bool) -> Vec<u8> {
    let mut output = String::with_capacity(input.len().saturating_mul(3).max(16));
    let mut remaining = input;
    loop {
        let (result, read, _) = decoder.decode_to_string(remaining, &mut output, last);
        remaining = &remaining[read..];
        match result {
            encoding_rs::CoderResult::InputEmpty => break,
            encoding_rs::CoderResult::OutputFull => {
                output.reserve(remaining.len().saturating_mul(3).max(16));
            }
        }
    }
    output.into_bytes()
}

pub(crate) fn encode_terminal_input<'a>(
    encoding: TerminalEncoding,
    source: TerminalInputSource,
    input: &'a [u8],
) -> Cow<'a, [u8]> {
    if !is_text_source(source) || encoding == TerminalEncoding::Utf8 || input.is_ascii() {
        return Cow::Borrowed(input);
    }
    let Ok(text) = std::str::from_utf8(input) else {
        return Cow::Borrowed(input);
    };
    let Some(encoding) = encoding.encoding() else {
        return Cow::Borrowed(input);
    };
    let (encoded, _, _) = encoding.encode(text);
    encoded
}

fn is_text_source(source: TerminalInputSource) -> bool {
    matches!(
        source,
        TerminalInputSource::User
            | TerminalInputSource::ExternalInput
            | TerminalInputSource::AgentCommand
            | TerminalInputSource::InitCommand
    )
}
