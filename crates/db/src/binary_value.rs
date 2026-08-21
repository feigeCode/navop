use base64::Engine as _;

const BASE64_PREFIX: &str = "base64:";
const HEX_PREFIX: &str = "hex:";
const TEXT_PREFIX: &str = "text:";

/// An explicitly encoded binary editor value was malformed.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum BinaryInputError {
    #[error("invalid base64 binary value: {0}")]
    InvalidBase64(#[from] base64::DecodeError),
    #[error("invalid hexadecimal binary value")]
    InvalidHex,
}

/// Parse the unambiguous binary editor/transport representation.
///
/// Supported forms:
/// - `base64:<payload>`
/// - `hex:<digits>`
/// - `text:<utf8 text>`
/// - `0x<digits>` / `0X<digits>` for legacy hexadecimal input
/// - unprefixed input as its exact UTF-8 bytes
pub fn parse_binary_input(value: &str) -> Result<Vec<u8>, BinaryInputError> {
    if let Some(payload) = value.strip_prefix(BASE64_PREFIX) {
        return base64::engine::general_purpose::STANDARD
            .decode(payload)
            .map_err(BinaryInputError::from);
    }

    if let Some(payload) = value.strip_prefix(HEX_PREFIX) {
        return decode_hex_payload(payload, true);
    }

    if let Some(payload) = value.strip_prefix(TEXT_PREFIX) {
        return Ok(payload.as_bytes().to_vec());
    }

    if let Some(payload) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        return decode_hex_payload(payload, false);
    }

    Ok(value.as_bytes().to_vec())
}

/// Format bytes for copying back into a binary editor without content guessing.
pub fn format_binary_input(bytes: &[u8]) -> String {
    format!(
        "{BASE64_PREFIX}{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )
}

fn decode_hex_payload(payload: &str, allow_empty: bool) -> Result<Vec<u8>, BinaryInputError> {
    if (!allow_empty && payload.is_empty())
        || payload.len() % 2 != 0
        || !payload.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(BinaryInputError::InvalidHex);
    }
    hex::decode(payload).map_err(|_| BinaryInputError::InvalidHex)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_explicit_encodings_and_unprefixed_utf8() {
        assert_eq!(parse_binary_input("base64:AQID").unwrap(), [1, 2, 3]);
        assert_eq!(parse_binary_input("hex:010203").unwrap(), [1, 2, 3]);
        assert_eq!(parse_binary_input("0x010203").unwrap(), [1, 2, 3]);
        assert_eq!(parse_binary_input("0X010203").unwrap(), [1, 2, 3]);
        assert_eq!(parse_binary_input("text:true").unwrap(), b"true");
        assert_eq!(parse_binary_input("true").unwrap(), b"true");
        assert_eq!(parse_binary_input("AQID").unwrap(), b"AQID");
    }

    #[test]
    fn empty_values_are_binary_not_null() {
        assert_eq!(parse_binary_input("").unwrap(), b"");
        assert_eq!(parse_binary_input("base64:").unwrap(), b"");
        assert_eq!(parse_binary_input("hex:").unwrap(), b"");
        assert_eq!(parse_binary_input("text:").unwrap(), b"");
        assert_eq!(
            parse_binary_input("0x"),
            Err(BinaryInputError::InvalidHex)
        );
    }

    #[test]
    fn explicit_invalid_encodings_fail_instead_of_becoming_text() {
        assert!(matches!(
            parse_binary_input("base64:not-valid!"),
            Err(BinaryInputError::InvalidBase64(_))
        ));
        assert_eq!(
            parse_binary_input("hex:abc"),
            Err(BinaryInputError::InvalidHex)
        );
    }

    #[test]
    fn copy_format_is_self_describing_and_roundtrips() {
        let bytes = [0, 1, 2, 0xff];
        let encoded = format_binary_input(&bytes);
        assert_eq!(encoded, "base64:AAEC/w==");
        assert_eq!(parse_binary_input(&encoded).unwrap(), bytes);
    }
}
