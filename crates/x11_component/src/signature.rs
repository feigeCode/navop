use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use ed25519_dalek::{Signature, VerifyingKey};

use crate::X11ComponentError;

/// 使用独立公钥对原始字节执行 Ed25519 严格验签。
pub fn verify_ed25519(
    payload: &[u8],
    signature_base64: &str,
    public_key_base64: &str,
) -> Result<(), X11ComponentError> {
    let public = decode::<32>(public_key_base64, "公钥")?;
    let signature = decode::<64>(signature_base64, "签名")?;
    let key = VerifyingKey::from_bytes(&public)
        .map_err(|error| X11ComponentError::SignatureInvalid(error.to_string()))?;
    key.verify_strict(payload, &Signature::from_bytes(&signature))
        .map_err(|error| X11ComponentError::SignatureInvalid(error.to_string()))
}

fn decode<const N: usize>(text: &str, name: &str) -> Result<[u8; N], X11ComponentError> {
    let bytes = BASE64
        .decode(text.trim())
        .map_err(|error| X11ComponentError::SignatureInvalid(format!("{name}编码：{error}")))?;
    bytes
        .try_into()
        .map_err(|_| X11ComponentError::SignatureInvalid(format!("{name}长度必须为 {N} 字节")))
}
