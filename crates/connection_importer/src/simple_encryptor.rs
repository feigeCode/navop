use aes::Aes256;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use cbc::cipher::{BlockDecryptMut, KeyIvInit, block_padding::Pkcs7};
use hmac::{Hmac, Mac};
use serde_json::Value;
use sha2::{Digest, Sha256};

type Aes256CbcDec = cbc::Decryptor<Aes256>;
type HmacSha256 = Hmac<Sha256>;

pub fn decrypt_value(key: &str, cipher_text: &str) -> Option<Value> {
    let body = verified_body(key, cipher_text)?;
    let iv = hex::decode(body.get(..32)?).ok()?;
    let encrypted_json = body.get(32..)?;
    let mut encrypted = STANDARD.decode(encrypted_json).ok()?;
    let crypto_key = crypto_key(key);
    let decrypted = Aes256CbcDec::new_from_slices(&crypto_key, &iv)
        .ok()?
        .decrypt_padded_mut::<Pkcs7>(&mut encrypted)
        .ok()?;
    let json = String::from_utf8(decrypted.to_vec()).ok()?;
    serde_json::from_str(&json).ok()
}

pub fn looks_encrypted(value: &str) -> bool {
    value.len() > 96 && is_hex(value.get(..96).unwrap_or_default())
}

fn verified_body<'a>(key: &str, cipher_text: &'a str) -> Option<&'a str> {
    if cipher_text.len() <= 96 {
        return None;
    }
    let expected_hmac = hex::decode(cipher_text.get(..64)?).ok()?;
    let body = cipher_text.get(64..)?;
    let mut mac = HmacSha256::new_from_slice(&crypto_key(key)).ok()?;
    mac.update(body.as_bytes());
    mac.verify_slice(&expected_hmac).ok()?;
    Some(body)
}

fn crypto_key(key: &str) -> [u8; 32] {
    Sha256::digest(key.as_bytes()).into()
}

fn is_hex(value: &str) -> bool {
    value.bytes().all(|byte| byte.is_ascii_hexdigit())
}
