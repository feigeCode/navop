use std::fmt;

use aes_gcm::{
    Aes256Gcm, Key, Nonce,
    aead::{Aead, KeyInit},
};
use argon2::{Algorithm, Argon2, Params, Version};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use rand::RngCore;
use serde::{Deserialize, Serialize};

use crate::crypto;

pub const TEAM_KEY_ENVELOPE_PREFIX: &str = "TEAMKEY2:";
pub const MIN_NEW_TEAM_KEY_CHARS: usize = 12;

const ENVELOPE_VERSION: u8 = 2;
const KDF_NAME: &str = "argon2id";
const DATA_KEY_LEN: usize = 32;
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;
const WRAPPED_MAGIC: &[u8] = b"NAVOP_TEAM_KEY_V2\0";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TeamKeyScheme {
    Legacy,
    EnvelopeV2,
}

#[derive(Debug, Clone, Copy)]
pub struct TeamKeyKdfParams {
    pub memory_kib: u32,
    pub iterations: u32,
    pub parallelism: u32,
}

impl TeamKeyKdfParams {
    pub const fn production() -> Self {
        Self {
            memory_kib: 65_536,
            iterations: 3,
            parallelism: 1,
        }
    }

    #[cfg(test)]
    pub const fn for_tests() -> Self {
        Self {
            memory_kib: 32,
            iterations: 1,
            parallelism: 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatedTeamKeyEnvelope {
    pub verification: String,
    pub data_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnlockedTeamKey {
    pub scheme: TeamKeyScheme,
    pub data_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TeamKeyEnvelopeError {
    KeyTooShort { minimum: usize },
    InvalidKeyOrEnvelope,
    CreationFailed,
}

impl fmt::Display for TeamKeyEnvelopeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::KeyTooShort { minimum } => write!(f, "团队密钥至少需要 {minimum} 个字符"),
            Self::InvalidKeyOrEnvelope => write!(f, "团队密钥错误或密钥数据已损坏"),
            Self::CreationFailed => write!(f, "无法创建团队密钥数据"),
        }
    }
}

impl std::error::Error for TeamKeyEnvelopeError {}

#[derive(Debug, Serialize, Deserialize)]
struct TeamKeyEnvelopeV2 {
    version: u8,
    kdf: String,
    memory_kib: u32,
    iterations: u32,
    parallelism: u32,
    salt: String,
    nonce: String,
    wrapped_data_key: String,
}

pub fn detect_team_key_scheme(verification: &str) -> TeamKeyScheme {
    if verification.starts_with(TEAM_KEY_ENVELOPE_PREFIX) {
        TeamKeyScheme::EnvelopeV2
    } else {
        TeamKeyScheme::Legacy
    }
}

pub fn create_team_key_envelope(
    passphrase: &str,
    params: TeamKeyKdfParams,
) -> Result<CreatedTeamKeyEnvelope, TeamKeyEnvelopeError> {
    if passphrase.chars().count() < MIN_NEW_TEAM_KEY_CHARS {
        return Err(TeamKeyEnvelopeError::KeyTooShort {
            minimum: MIN_NEW_TEAM_KEY_CHARS,
        });
    }

    let mut salt = [0_u8; SALT_LEN];
    let mut nonce = [0_u8; NONCE_LEN];
    let mut data_key = [0_u8; DATA_KEY_LEN];
    rand::thread_rng().fill_bytes(&mut salt);
    rand::thread_rng().fill_bytes(&mut nonce);
    rand::thread_rng().fill_bytes(&mut data_key);

    let wrapping_key = derive_wrapping_key(passphrase, &salt, params)
        .map_err(|_| TeamKeyEnvelopeError::CreationFailed)?;
    let mut plaintext = Vec::with_capacity(WRAPPED_MAGIC.len() + DATA_KEY_LEN);
    plaintext.extend_from_slice(WRAPPED_MAGIC);
    plaintext.extend_from_slice(&data_key);
    let wrapped = encrypt(&wrapping_key, &nonce, &plaintext)
        .map_err(|_| TeamKeyEnvelopeError::CreationFailed)?;
    let envelope = TeamKeyEnvelopeV2 {
        version: ENVELOPE_VERSION,
        kdf: KDF_NAME.to_string(),
        memory_kib: params.memory_kib,
        iterations: params.iterations,
        parallelism: params.parallelism,
        salt: BASE64.encode(salt),
        nonce: BASE64.encode(nonce),
        wrapped_data_key: BASE64.encode(wrapped),
    };
    let json = serde_json::to_vec(&envelope).map_err(|_| TeamKeyEnvelopeError::CreationFailed)?;

    Ok(CreatedTeamKeyEnvelope {
        verification: format!("{TEAM_KEY_ENVELOPE_PREFIX}{}", BASE64.encode(json)),
        data_key: BASE64.encode(data_key),
    })
}

pub fn unlock_team_key(
    verification: &str,
    passphrase: &str,
) -> Result<UnlockedTeamKey, TeamKeyEnvelopeError> {
    if detect_team_key_scheme(verification) == TeamKeyScheme::Legacy {
        return if crypto::verify_master_key(passphrase, verification) {
            Ok(UnlockedTeamKey {
                scheme: TeamKeyScheme::Legacy,
                data_key: passphrase.to_string(),
            })
        } else {
            Err(TeamKeyEnvelopeError::InvalidKeyOrEnvelope)
        };
    }

    unlock_v2(verification, passphrase)
}

fn unlock_v2(
    verification: &str,
    passphrase: &str,
) -> Result<UnlockedTeamKey, TeamKeyEnvelopeError> {
    let encoded = verification
        .strip_prefix(TEAM_KEY_ENVELOPE_PREFIX)
        .ok_or(TeamKeyEnvelopeError::InvalidKeyOrEnvelope)?;
    let json = BASE64
        .decode(encoded)
        .map_err(|_| TeamKeyEnvelopeError::InvalidKeyOrEnvelope)?;
    let envelope: TeamKeyEnvelopeV2 =
        serde_json::from_slice(&json).map_err(|_| TeamKeyEnvelopeError::InvalidKeyOrEnvelope)?;
    validate_envelope(&envelope)?;
    let salt = decode_exact::<SALT_LEN>(&envelope.salt)?;
    let nonce = decode_exact::<NONCE_LEN>(&envelope.nonce)?;
    let wrapped = BASE64
        .decode(&envelope.wrapped_data_key)
        .map_err(|_| TeamKeyEnvelopeError::InvalidKeyOrEnvelope)?;
    let params = TeamKeyKdfParams {
        memory_kib: envelope.memory_kib,
        iterations: envelope.iterations,
        parallelism: envelope.parallelism,
    };
    let wrapping_key = derive_wrapping_key(passphrase, &salt, params)
        .map_err(|_| TeamKeyEnvelopeError::InvalidKeyOrEnvelope)?;
    let plaintext = decrypt(&wrapping_key, &nonce, &wrapped)?;
    let data_key = plaintext
        .strip_prefix(WRAPPED_MAGIC)
        .filter(|key| key.len() == DATA_KEY_LEN)
        .ok_or(TeamKeyEnvelopeError::InvalidKeyOrEnvelope)?;

    Ok(UnlockedTeamKey {
        scheme: TeamKeyScheme::EnvelopeV2,
        data_key: BASE64.encode(data_key),
    })
}

fn validate_envelope(envelope: &TeamKeyEnvelopeV2) -> Result<(), TeamKeyEnvelopeError> {
    if envelope.version != ENVELOPE_VERSION
        || envelope.kdf != KDF_NAME
        || envelope.parallelism == 0
        || envelope.memory_kib < 8 * envelope.parallelism
        || envelope.iterations == 0
    {
        return Err(TeamKeyEnvelopeError::InvalidKeyOrEnvelope);
    }
    Ok(())
}

fn derive_wrapping_key(
    passphrase: &str,
    salt: &[u8],
    params: TeamKeyKdfParams,
) -> Result<[u8; DATA_KEY_LEN], argon2::Error> {
    let params = Params::new(
        params.memory_kib,
        params.iterations,
        params.parallelism,
        Some(DATA_KEY_LEN),
    )?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = [0_u8; DATA_KEY_LEN];
    argon2.hash_password_into(passphrase.as_bytes(), salt, &mut key)?;
    Ok(key)
}

fn encrypt(key: &[u8; DATA_KEY_LEN], nonce: &[u8; NONCE_LEN], value: &[u8]) -> Result<Vec<u8>, ()> {
    Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key))
        .encrypt(Nonce::from_slice(nonce), value)
        .map_err(|_| ())
}

fn decrypt(
    key: &[u8; DATA_KEY_LEN],
    nonce: &[u8; NONCE_LEN],
    value: &[u8],
) -> Result<Vec<u8>, TeamKeyEnvelopeError> {
    Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key))
        .decrypt(Nonce::from_slice(nonce), value)
        .map_err(|_| TeamKeyEnvelopeError::InvalidKeyOrEnvelope)
}

fn decode_exact<const N: usize>(value: &str) -> Result<[u8; N], TeamKeyEnvelopeError> {
    let decoded = BASE64
        .decode(value)
        .map_err(|_| TeamKeyEnvelopeError::InvalidKeyOrEnvelope)?;
    decoded
        .try_into()
        .map_err(|_| TeamKeyEnvelopeError::InvalidKeyOrEnvelope)
}
