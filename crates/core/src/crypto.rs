//! 密码加密模块
//!
//! 使用 AES-256-GCM 对密码进行加密存储。
//! 用户需要设置一个主密钥，该密钥用于派生加密密钥。
//! 如果主密钥丢失，所有加密的密码将无法恢复。
//!
//! 支持两种使用方式：
//! 1. 全局密钥模式：通过 set_master_key 设置全局密钥，使用 encrypt_password/decrypt_password
//! 2. 指定密钥模式：使用 encrypt_with_key/decrypt_with_key 直接传入密钥
//!
//! 密钥派生（KDF）：
//! - 新版本使用 Argon2id + 随机盐，验证信封格式为 `NAVOPKDF2:` 前缀的 JSON
//! - 旧版 `SHA-256(主密钥 || 固定盐)` 的验证数据与密文仍然可以解密（legacy 兜底），
//!   解锁后由上层迁移流程重新加密
//! - 迁移是 crash-safe 的：新信封先落盘，迁移窗口内旧密文靠 legacy 派生兜底
//!
//! 密钥持久化：
//! - 通过 key_storage 模块统一封装存储后端接口
//! - 应用启动时可自动从存储后端恢复密钥

use aes_gcm::{
    Aes256Gcm, Key, Nonce,
    aead::{Aead, KeyInit},
};
use argon2::{Algorithm, Argon2, Params, Version};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, RwLock};
use zeroize::Zeroizing;

use crate::key_storage;

/// 加密错误类型
#[derive(Debug, Clone, PartialEq)]
pub enum CryptoError {
    /// 旧密码错误
    InvalidOldPassword,
    /// 新密码为空
    EmptyNewPassword,
    /// 两次输入的新密码不一致
    PasswordMismatch,
    /// 未设置过主密码
    NoPasswordSet,
    /// 保存验证数据失败
    SaveVerificationFailed,
    /// 读取验证数据失败
    LoadVerificationFailed,
    /// 更新自动解锁密钥存储失败
    KeyStorageFailed(String),
    /// 解密失败
    DecryptionFailed,
    /// 编码失败
    EncodingFailed,
    /// 数据格式错误
    InvalidDataFormat,
}

impl std::fmt::Display for CryptoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CryptoError::InvalidOldPassword => write!(f, "旧密码错误"),
            CryptoError::EmptyNewPassword => write!(f, "新密码不能为空"),
            CryptoError::PasswordMismatch => write!(f, "两次输入的新密码不一致"),
            CryptoError::NoPasswordSet => write!(f, "未设置过主密码"),
            CryptoError::SaveVerificationFailed => write!(f, "保存验证数据失败"),
            CryptoError::LoadVerificationFailed => write!(f, "读取验证数据失败"),
            CryptoError::KeyStorageFailed(error) => write!(f, "更新密钥存储失败: {error}"),
            CryptoError::DecryptionFailed => write!(f, "解密失败"),
            CryptoError::EncodingFailed => write!(f, "编码失败"),
            CryptoError::InvalidDataFormat => write!(f, "数据格式错误"),
        }
    }
}

impl std::error::Error for CryptoError {}

/// 加密前缀标识，用于识别已加密的密码
///
/// 旧格式：`ENC:` + base64(nonce + ciphertext)，按 SHA-256 固定盐派生解密。
/// 新格式：`ENC:` + base64(JSON)，密文自带 Argon2id 参数与盐，解密不再依赖
/// 进程内的 KDF 状态，跨设备与重启后都能用同一口令解开。
const ENCRYPTED_PREFIX: &str = "ENC:";
const NONCE_LENGTH: usize = 12;
const AUTH_TAG_LENGTH: usize = 16;
const MIN_ENCRYPTED_LENGTH: usize = NONCE_LENGTH + AUTH_TAG_LENGTH;

/// 验证数据的魔术字符串，用于验证密钥是否正确
const VERIFICATION_MAGIC: &str = "ONEHUB_KEY_VERIFY_V1";

/// 新版 KDF 信封前缀（Argon2id + 随机盐）
const KDF_ENVELOPE_PREFIX: &str = "NAVOPKDF2:";
/// KDF 信封版本
const KDF_ENVELOPE_VERSION: u8 = 2;
/// KDF 算法名
const KDF_NAME: &str = "argon2id";
/// 随机盐长度
const KDF_SALT_LENGTH: usize = 16;
/// Argon2 内存上限（KiB），防止被篡改的信封触发内存炸弹
const MAX_KDF_MEMORY_KIB: u32 = 1 << 20;
/// Argon2 迭代次数上限
const MAX_KDF_ITERATIONS: u32 = 64;
/// Argon2 并行度上限
const MAX_KDF_PARALLELISM: u32 = 16;
/// 已识别 KDF 规范的上限（解密时按序尝试，用于兼容迁移窗口内的多代信封）
const MAX_KNOWN_KDF_SPECS: usize = 8;
/// 派生密钥缓存上限
const MAX_DERIVED_KEY_CACHE: usize = 16;

/// 密钥验证文件名
const KEY_VERIFICATION_FILE: &str = "key_verification";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MasterKeyPersistence {
    Persistent,
    SessionOnly,
}

struct MasterKeySetup<'a> {
    master_key: &'a str,
    persistence: MasterKeyPersistence,
    create_verification: bool,
}

/// Argon2id 参数
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct KdfParams {
    memory_kib: u32,
    iterations: u32,
    parallelism: u32,
}

impl KdfParams {
    /// 生产参数：64 MiB / 3 轮 / 1 并行（与团队密钥信封一致）
    #[cfg(not(test))]
    const fn production() -> Self {
        Self {
            memory_kib: 65536,
            iterations: 3,
            parallelism: 1,
        }
    }

    /// 测试参数：保持单测速度
    #[cfg(test)]
    const fn for_tests() -> Self {
        Self {
            memory_kib: 32,
            iterations: 1,
            parallelism: 1,
        }
    }

    fn default_for_build() -> Self {
        #[cfg(test)]
        {
            Self::for_tests()
        }
        #[cfg(not(test))]
        {
            Self::production()
        }
    }

    /// 参数必须落在合理区间，避免被篡改的信封触发超大内存分配
    fn is_valid(&self) -> bool {
        self.parallelism >= 1
            && self.parallelism <= MAX_KDF_PARALLELISM
            && self.iterations >= 1
            && self.iterations <= MAX_KDF_ITERATIONS
            && self.memory_kib >= 8 * self.parallelism
            && self.memory_kib <= MAX_KDF_MEMORY_KIB
    }

    fn fingerprint(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(12);
        bytes.extend_from_slice(&self.memory_kib.to_le_bytes());
        bytes.extend_from_slice(&self.iterations.to_le_bytes());
        bytes.extend_from_slice(&self.parallelism.to_le_bytes());
        bytes
    }
}

/// 派生主密钥所用的 KDF 规范
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum KdfSpec {
    /// 旧版：SHA-256(主密钥 || 固定盐)，无迭代、无随机盐（仅为兼容旧数据保留）
    LegacySha256,
    /// 新版：Argon2id + 随机盐
    Argon2id {
        params: KdfParams,
        salt: [u8; KDF_SALT_LENGTH],
    },
}

impl KdfSpec {
    fn fingerprint(&self) -> Vec<u8> {
        match self {
            Self::LegacySha256 => b"sha256-fixed-v1".to_vec(),
            Self::Argon2id { params, salt } => {
                let mut bytes = b"argon2id-v2".to_vec();
                bytes.extend_from_slice(&params.fingerprint());
                bytes.extend_from_slice(salt);
                bytes
            }
        }
    }
}

/// 解析后的验证数据
enum ParsedVerification {
    /// 旧版：base64(nonce + ciphertext)
    Legacy(Vec<u8>),
    /// 新版：Argon2id 信封
    Argon2 {
        spec: KdfSpec,
        nonce: [u8; NONCE_LENGTH],
        verifier: Vec<u8>,
    },
}

/// 新版 KDF 信封的 JSON 结构
#[derive(Clone, Debug, Serialize, Deserialize)]
struct KdfEnvelopeRecord {
    version: u8,
    kdf: String,
    memory_kib: u32,
    iterations: u32,
    parallelism: u32,
    salt: String,
    nonce: String,
    verifier: String,
}

/// 自描述密文的 JSON 结构（`ENC:` + base64(JSON)）
#[derive(Clone, Debug, Serialize, Deserialize)]
struct EncryptedRecord {
    version: u8,
    kdf: String,
    memory_kib: u32,
    iterations: u32,
    parallelism: u32,
    salt: String,
    nonce: String,
    ciphertext: String,
}

/// 全局加密密钥存储（派生后的密钥）
static ENCRYPTION_KEY: RwLock<Option<[u8; 32]>> = RwLock::new(None);

/// 全局原始主密钥存储（用于云同步等需要原始密钥的场景）
static RAW_MASTER_KEY: RwLock<Option<Zeroizing<String>>> = RwLock::new(None);

/// 当前生效的 KDF 规范（None 时按 legacy 派生，保证旧行为确定性）
static ACTIVE_KDF_SPEC: RwLock<Option<KdfSpec>> = RwLock::new(None);

/// 已识别过的 KDF 规范（解密时兜底尝试，覆盖迁移窗口与并行测试）
static KNOWN_KDF_SPECS: Mutex<Vec<KdfSpec>> = Mutex::new(Vec::new());

/// 派生密钥缓存条目：规范指纹 -> 派生密钥
type DerivedKeyCacheEntry = ([u8; 32], Zeroizing<[u8; 32]>);

/// 派生密钥缓存（Argon2 较慢，迁移/轮换会反复用同一口令派生）
static DERIVED_KEY_CACHE: Mutex<Vec<DerivedKeyCacheEntry>> = Mutex::new(Vec::new());

/// Serializes tests that read or mutate the process-global master key
/// (`ENCRYPTION_KEY` / `RAW_MASTER_KEY`). The crypto module tests and the
/// cloud-sync tests that derive team keys from the master key must all hold this
/// lock, otherwise a concurrent `set_master_key` from one test can make another
/// test's refresh/sync path observe an unexpected master-key state.
#[cfg(test)]
pub(crate) fn crypto_test_lock() -> &'static std::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
}

/// 获取数据目录路径
fn get_data_dir() -> Option<PathBuf> {
    // 单元测试绝不能读写真实用户的数据目录。
    //
    // 此前 `reset_repo_password()` 会在测试里删掉真实安装的 key_verification，
    // 随后的 `set_master_key()` 又用 KdfParams::for_tests() 把它重写成正式版
    // 本读不懂的信封，导致已安装的 Navop 再也解不开自己的密码（2026-10-26）。
    #[cfg(test)]
    {
        key_storage::test_data_dir_override().or_else(|| Some(crate::app_dirs::test_data_dir()))
    }
    #[cfg(not(test))]
    {
        crate::app_dirs::data_dir()
    }
}

/// 获取密钥验证文件路径
fn get_verification_file_path() -> Option<PathBuf> {
    get_data_dir().map(|p| p.join(KEY_VERIFICATION_FILE))
}

/// 检查是否已设置过主密钥（验证文件是否存在）
pub fn has_repo_password_set() -> bool {
    get_verification_file_path()
        .map(|p| p.exists())
        .unwrap_or(false)
}

/// 保存密钥验证数据到文件
fn save_verification_data(data: &str) -> bool {
    if let Some(path) = get_verification_file_path() {
        key_storage::atomic_write_file(&path, data.as_bytes()).is_ok()
    } else {
        false
    }
}

/// 从文件读取密钥验证数据
fn load_verification_data() -> Option<String> {
    get_verification_file_path().and_then(|p| fs::read_to_string(p).ok())
}

/// 旧版派生：SHA-256(主密钥 || 固定盐)
///
/// 仅用于读取升级前的数据，禁止用于新写入。
fn derive_key_legacy(master_key: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(master_key.as_bytes());
    // 旧版固定盐，仅为兼容历史数据保留
    hasher.update(b"onehub_password_encryption_salt_v1");
    let result = hasher.finalize();
    let mut key = [0u8; 32];
    key.copy_from_slice(&result);
    key
}

fn derive_key_argon2(
    master_key: &str,
    params: &KdfParams,
    salt: &[u8; KDF_SALT_LENGTH],
) -> Option<[u8; 32]> {
    let params = Params::new(
        params.memory_kib,
        params.iterations,
        params.parallelism,
        Some(32),
    )
    .ok()?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = [0u8; 32];
    argon2
        .hash_password_into(master_key.as_bytes(), salt, &mut key)
        .ok()?;
    Some(key)
}

fn derive_key_with_spec(master_key: &str, spec: &KdfSpec) -> Option<[u8; 32]> {
    match spec {
        KdfSpec::LegacySha256 => Some(derive_key_legacy(master_key)),
        KdfSpec::Argon2id { params, salt } => derive_key_argon2(master_key, params, salt),
    }
}

fn derive_fingerprint(master_key: &str, spec: &KdfSpec) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"navop-kdf-derivation-v2");
    hasher.update(spec.fingerprint());
    hasher.update(master_key.as_bytes());
    let digest = hasher.finalize();
    let mut fingerprint = [0u8; 32];
    fingerprint.copy_from_slice(&digest);
    fingerprint
}

/// 带缓存的派生：Argon2 较慢，迁移与轮换会反复用同一口令派生同一密钥
fn derive_key_with_cache(master_key: &str, spec: &KdfSpec) -> Option<[u8; 32]> {
    if matches!(spec, KdfSpec::LegacySha256) {
        return Some(derive_key_legacy(master_key));
    }
    let fingerprint = derive_fingerprint(master_key, spec);
    if let Ok(cache) = DERIVED_KEY_CACHE.lock()
        && let Some((_, key)) = cache.iter().find(|(cached, _)| cached == &fingerprint)
    {
        return Some(**key);
    }
    let key = derive_key_with_spec(master_key, spec)?;
    if let Ok(mut cache) = DERIVED_KEY_CACHE.lock()
        && cache.iter().all(|(cached, _)| cached != &fingerprint)
    {
        if cache.len() >= MAX_DERIVED_KEY_CACHE {
            cache.remove(0);
        }
        cache.push((fingerprint, Zeroizing::new(key)));
    }
    Some(key)
}

fn register_known_spec(spec: KdfSpec) {
    if matches!(spec, KdfSpec::LegacySha256) {
        return;
    }
    let Ok(mut known) = KNOWN_KDF_SPECS.lock() else {
        return;
    };
    if known.contains(&spec) {
        return;
    }
    if known.len() >= MAX_KNOWN_KDF_SPECS {
        known.remove(0);
    }
    known.push(spec);
}

fn active_kdf_spec() -> Option<KdfSpec> {
    ACTIVE_KDF_SPEC.read().ok().and_then(|guard| *guard)
}

fn set_active_kdf_spec(spec: KdfSpec) {
    register_known_spec(spec);
    if let Ok(mut guard) = ACTIVE_KDF_SPEC.write() {
        *guard = Some(spec);
    }
}

/// 新建随机盐的 Argon2id 规范
fn default_kdf_spec() -> KdfSpec {
    let mut salt = [0u8; KDF_SALT_LENGTH];
    rand::thread_rng().fill_bytes(&mut salt);
    KdfSpec::Argon2id {
        params: KdfParams::default_for_build(),
        salt,
    }
}

/// 从用户主密钥派生 AES-256 密钥
///
/// 使用当前生效的 KDF；未识别时按 legacy 派生，保证旧行为可复现。
fn derive_key(master_key: &str) -> [u8; 32] {
    let spec = active_kdf_spec().unwrap_or(KdfSpec::LegacySha256);
    derive_key_with_cache(master_key, &spec).unwrap_or_else(|| derive_key_legacy(master_key))
}

/// 解密候选密钥：当前规范 → 已知规范 → legacy 兜底
///
/// 迁移窗口内同一口令可能派生过多代密钥，逐代尝试并靠 AES-GCM 认证标签判定。
fn candidate_keys(master_key: &str) -> Vec<[u8; 32]> {
    let mut specs = Vec::with_capacity(MAX_KNOWN_KDF_SPECS + 2);
    if let Some(active) = active_kdf_spec() {
        specs.push(active);
    }
    if let Ok(known) = KNOWN_KDF_SPECS.lock() {
        specs.extend(known.iter().copied());
    }
    specs.push(KdfSpec::LegacySha256);

    let mut keys = Vec::new();
    for spec in specs {
        if let Some(key) = derive_key_with_cache(master_key, &spec)
            && !keys.contains(&key)
        {
            keys.push(key);
        }
    }
    keys
}

fn decode_fixed<const N: usize>(encoded: &str) -> Option<[u8; N]> {
    let bytes = BASE64.decode(encoded).ok()?;
    bytes.as_slice().try_into().ok()
}

fn encode_kdf_envelope(
    spec: &KdfSpec,
    verifier: &[u8],
    nonce: &[u8; NONCE_LENGTH],
) -> Option<String> {
    let KdfSpec::Argon2id { params, salt } = spec else {
        return None;
    };
    let record = KdfEnvelopeRecord {
        version: KDF_ENVELOPE_VERSION,
        kdf: KDF_NAME.to_string(),
        memory_kib: params.memory_kib,
        iterations: params.iterations,
        parallelism: params.parallelism,
        salt: BASE64.encode(salt),
        nonce: BASE64.encode(nonce),
        verifier: BASE64.encode(verifier),
    };
    let json = serde_json::to_vec(&record).ok()?;
    Some(format!("{KDF_ENVELOPE_PREFIX}{}", BASE64.encode(json)))
}

fn parse_verification_data(data: &str) -> Option<ParsedVerification> {
    if data.is_empty() {
        return None;
    }

    if let Some(encoded) = data.strip_prefix(KDF_ENVELOPE_PREFIX) {
        let json = BASE64.decode(encoded).ok()?;
        let record: KdfEnvelopeRecord = serde_json::from_slice(&json).ok()?;
        if record.version != KDF_ENVELOPE_VERSION || record.kdf != KDF_NAME {
            return None;
        }
        let params = KdfParams {
            memory_kib: record.memory_kib,
            iterations: record.iterations,
            parallelism: record.parallelism,
        };
        if !params.is_valid() {
            return None;
        }
        let salt: [u8; KDF_SALT_LENGTH] = decode_fixed(&record.salt)?;
        let nonce: [u8; NONCE_LENGTH] = decode_fixed(&record.nonce)?;
        let verifier = BASE64.decode(&record.verifier).ok()?;
        if verifier.len() < AUTH_TAG_LENGTH {
            return None;
        }
        return Some(ParsedVerification::Argon2 {
            spec: KdfSpec::Argon2id { params, salt },
            nonce,
            verifier,
        });
    }

    let combined = BASE64.decode(data).ok()?;
    if combined.len() < MIN_ENCRYPTED_LENGTH {
        return None;
    }
    Some(ParsedVerification::Legacy(combined))
}

/// 编码自描述密文：`ENC:` + base64(JSON{version,kdf,params,salt,nonce,ciphertext})
fn encode_encrypted_blob(
    spec: &KdfSpec,
    nonce: &[u8; NONCE_LENGTH],
    ciphertext: &[u8],
) -> Option<String> {
    let KdfSpec::Argon2id { params, salt } = spec else {
        return None;
    };
    let record = EncryptedRecord {
        version: KDF_ENVELOPE_VERSION,
        kdf: KDF_NAME.to_string(),
        memory_kib: params.memory_kib,
        iterations: params.iterations,
        parallelism: params.parallelism,
        salt: BASE64.encode(salt),
        nonce: BASE64.encode(nonce),
        ciphertext: BASE64.encode(ciphertext),
    };
    let json = serde_json::to_vec(&record).ok()?;
    Some(format!("{}{}", ENCRYPTED_PREFIX, BASE64.encode(json)))
}

/// 解析自描述密文；不是 JSON 记录（旧密文）时返回 None
fn parse_encrypted_blob(data: &str) -> Option<(KdfSpec, [u8; NONCE_LENGTH], Vec<u8>)> {
    let encoded = data.strip_prefix(ENCRYPTED_PREFIX)?;
    let bytes = BASE64.decode(encoded).ok()?;
    // 旧密文是 12 字节 nonce + 密文的原始字节，绝不会以 '{' 开头
    if bytes.first() != Some(&b'{') {
        return None;
    }
    let record: EncryptedRecord = serde_json::from_slice(&bytes).ok()?;
    if record.version != KDF_ENVELOPE_VERSION || record.kdf != KDF_NAME {
        return None;
    }
    let params = KdfParams {
        memory_kib: record.memory_kib,
        iterations: record.iterations,
        parallelism: record.parallelism,
    };
    if !params.is_valid() {
        return None;
    }
    let salt: [u8; KDF_SALT_LENGTH] = decode_fixed(&record.salt)?;
    let nonce: [u8; NONCE_LENGTH] = decode_fixed(&record.nonce)?;
    let ciphertext = BASE64.decode(&record.ciphertext).ok()?;
    if ciphertext.len() < AUTH_TAG_LENGTH {
        return None;
    }
    Some((KdfSpec::Argon2id { params, salt }, nonce, ciphertext))
}

/// 解码旧版密文：`ENC:` + base64(nonce + ciphertext)
fn decode_legacy_blob(data: &str) -> Option<Vec<u8>> {
    let encoded = data.strip_prefix(ENCRYPTED_PREFIX)?;
    let combined = BASE64.decode(encoded).ok()?;
    (combined.len() >= MIN_ENCRYPTED_LENGTH).then_some(combined)
}

/// 用给定口令与规范加密明文；Argon2 写自描述 JSON，legacy 写旧格式
fn encrypt_blob_with_spec(plaintext: &str, master_key: &str, spec: &KdfSpec) -> Option<String> {
    let key = derive_key_with_cache(master_key, spec)?;
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
    let mut nonce_bytes = [0u8; NONCE_LENGTH];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce_bytes), plaintext.as_bytes())
        .ok()?;
    match spec {
        KdfSpec::LegacySha256 => {
            let mut combined = Vec::with_capacity(NONCE_LENGTH + ciphertext.len());
            combined.extend_from_slice(&nonce_bytes);
            combined.extend_from_slice(&ciphertext);
            Some(format!("{}{}", ENCRYPTED_PREFIX, BASE64.encode(&combined)))
        }
        KdfSpec::Argon2id { .. } => encode_encrypted_blob(spec, &nonce_bytes, &ciphertext),
    }
}

fn kdf_spec_from_verification(verification_data: &str) -> Option<KdfSpec> {
    match parse_verification_data(verification_data)? {
        ParsedVerification::Legacy(_) => Some(KdfSpec::LegacySha256),
        ParsedVerification::Argon2 { spec, .. } => Some(spec),
    }
}

fn decrypt_verification_magic(key: &[u8; 32], nonce: &[u8; NONCE_LENGTH], verifier: &[u8]) -> bool {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    match cipher.decrypt(Nonce::from_slice(nonce), verifier) {
        Ok(plaintext) => plaintext == VERIFICATION_MAGIC.as_bytes(),
        Err(_) => false,
    }
}

fn verify_legacy_blob(master_key: &str, combined: &[u8]) -> bool {
    if combined.len() < MIN_ENCRYPTED_LENGTH {
        return false;
    }
    let key = derive_key_legacy(master_key);
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
    match cipher.decrypt(
        Nonce::from_slice(&combined[..NONCE_LENGTH]),
        &combined[NONCE_LENGTH..],
    ) {
        Ok(plaintext) => plaintext == VERIFICATION_MAGIC.as_bytes(),
        Err(_) => false,
    }
}

fn generate_key_verification_with_spec(master_key: &str, spec: &KdfSpec) -> Option<String> {
    let key = derive_key_with_cache(master_key, spec)?;
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));

    let mut nonce_bytes = [0u8; NONCE_LENGTH];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher.encrypt(nonce, VERIFICATION_MAGIC.as_bytes()).ok()?;

    match spec {
        KdfSpec::LegacySha256 => {
            let mut combined = Vec::with_capacity(NONCE_LENGTH + ciphertext.len());
            combined.extend_from_slice(&nonce_bytes);
            combined.extend_from_slice(&ciphertext);
            Some(BASE64.encode(&combined))
        }
        KdfSpec::Argon2id { .. } => encode_kdf_envelope(spec, &ciphertext, &nonce_bytes),
    }
}

/// 生成密钥验证数据
///
/// 返回一个加密的魔术字符串，用于验证用户输入的密钥是否正确。
/// 新格式为 `NAVOPKDF2:` + base64(JSON 信封)，包含随机盐与 Argon2 参数；
/// 仅在已有旧版信封时才会生成旧格式。
pub fn generate_key_verification(master_key: &str) -> String {
    // 沿用当前规范；只有从未识别过信封（首次设置）时才新建随机盐的 Argon2id。
    // 不能在这里把 legacy 安装悄悄切到 Argon2，否则信封与尚未重加密的数据不一致。
    let spec = active_kdf_spec().unwrap_or_else(default_kdf_spec);
    set_active_kdf_spec(spec);
    generate_key_verification_with_spec(master_key, &spec).unwrap_or_default()
}

/// 验证密钥是否正确
///
/// 通过尝试解密验证数据来验证密钥是否正确。
/// 同时兼容旧版（base64(nonce + ciphertext)）与新版（Argon2id 信封）格式。
pub fn verify_master_key(master_key: &str, verification_data: &str) -> bool {
    let Some(parsed) = parse_verification_data(verification_data) else {
        return false;
    };

    match parsed {
        ParsedVerification::Legacy(combined) => verify_legacy_blob(master_key, &combined),
        ParsedVerification::Argon2 {
            spec,
            nonce,
            verifier,
        } => {
            register_known_spec(spec);
            let Some(key) = derive_key_with_cache(master_key, &spec) else {
                return false;
            };
            decrypt_verification_magic(&key, &nonce, &verifier)
        }
    }
}

/// 设置主密钥
///
/// 用户提供的主密钥将被哈希后存储在内存中，用于后续的加密/解密操作。
/// 如果是首次设置，会生成并保存验证数据。
/// 同时会将密钥保存到存储后端，以便应用重启后自动恢复。
pub fn set_master_key(master_key: &str) -> Result<(), CryptoError> {
    set_master_key_with_persistence(master_key, MasterKeyPersistence::Persistent)
}

/// 仅为当前应用会话设置主密钥。
///
/// 该模式会删除存储后端中的主密钥，确保下次启动仍需手动解锁。
pub fn set_master_key_for_session(master_key: &str) -> Result<(), CryptoError> {
    set_master_key_with_persistence(master_key, MasterKeyPersistence::SessionOnly)
}

fn set_master_key_with_persistence(
    master_key: &str,
    persistence: MasterKeyPersistence,
) -> Result<(), CryptoError> {
    let storage = key_storage::get_key_storage();
    let setup = MasterKeySetup {
        master_key,
        persistence,
        create_verification: !has_repo_password_set(),
    };
    set_master_key_with_storage(setup, storage.as_ref(), save_verification_data)
}

fn set_master_key_with_storage(
    setup: MasterKeySetup<'_>,
    storage: &dyn key_storage::KeyStorage,
    save_verification: impl FnOnce(&str) -> bool,
) -> Result<(), CryptoError> {
    let previous = storage.load();
    if let Err(error) = update_key_storage(storage, setup.master_key, setup.persistence) {
        // 系统凭据库不可用（如 Linux 未接入、钥匙串被锁）不应阻塞本次解锁：
        // 降级为「仅本次会话」，下次启动重新输入主密钥，绝不回退到弱存储。
        if matches!(setup.persistence, MasterKeyPersistence::Persistent) {
            tracing::warn!("主密钥无法保存到系统凭据库，本次会话仍需手动输入: {error}");
            if let Err(cleanup) = storage.delete() {
                tracing::warn!("清理旧的自动解锁主密钥副本失败: {cleanup}");
            }
        } else {
            restore_key_storage(storage, previous)?;
            return Err(CryptoError::KeyStorageFailed(error));
        }
    }
    if setup.create_verification {
        let verification = generate_key_verification(setup.master_key);
        if !save_verification(&verification) {
            restore_key_storage(storage, previous)?;
            return Err(CryptoError::SaveVerificationFailed);
        }
    }
    publish_master_key(setup.master_key);
    Ok(())
}

/// 验证并设置主密钥
///
/// 如果已设置过密码，需要先验证密钥是否正确。
/// 返回 Ok(()) 表示设置成功，Err 表示密码错误。
pub fn verify_and_set_master_key(master_key: &str) -> Result<(), CryptoError> {
    verify_and_set_master_key_with_persistence(master_key, MasterKeyPersistence::Persistent)
}

/// 验证主密钥并仅在当前应用会话中解锁。
pub fn verify_and_set_master_key_for_session(master_key: &str) -> Result<(), CryptoError> {
    verify_and_set_master_key_with_persistence(master_key, MasterKeyPersistence::SessionOnly)
}

fn verify_and_set_master_key_with_persistence(
    master_key: &str,
    persistence: MasterKeyPersistence,
) -> Result<(), CryptoError> {
    if has_repo_password_set() {
        let verification_data = load_verification_data();
        verify_existing_master_key(master_key, verification_data.as_deref())?;
        if let Some(verification_data) = verification_data {
            activate_kdf_from_verification(&verification_data);
        }
    }

    set_master_key_with_persistence(master_key, persistence)
}

fn verify_existing_master_key(
    master_key: &str,
    verification_data: Option<&str>,
) -> Result<(), CryptoError> {
    let verification_data = verification_data.ok_or(CryptoError::LoadVerificationFailed)?;
    if verify_master_key(master_key, verification_data) {
        Ok(())
    } else {
        Err(CryptoError::InvalidOldPassword)
    }
}

fn update_key_storage(
    storage: &dyn key_storage::KeyStorage,
    master_key: &str,
    persistence: MasterKeyPersistence,
) -> Result<(), String> {
    match persistence {
        MasterKeyPersistence::Persistent => storage.save(master_key),
        MasterKeyPersistence::SessionOnly => storage.delete(),
    }
}

/// 删除为自动解锁而持久化的主密钥，不影响当前会话中的解锁状态。
pub fn forget_persisted_master_key() -> Result<(), String> {
    key_storage::get_key_storage().delete()
}

/// 将当前会话中的主密钥保存到存储后端，供后续启动自动解锁。
pub fn remember_master_key_for_future_startups() -> Result<(), String> {
    let master_key = get_raw_master_key().ok_or_else(|| "主密钥尚未解锁，无法保存".to_string())?;
    key_storage::get_key_storage().save(&master_key)
}

/// 清除主密钥
pub fn clear_master_key() {
    if let Ok(mut guard) = ENCRYPTION_KEY.write() {
        *guard = None;
    }
    if let Ok(mut guard) = RAW_MASTER_KEY.write() {
        *guard = None;
    }
    // 同时清除存储后端中的密钥
    let storage = key_storage::get_key_storage();
    let _ = storage.delete();
}

/// 检查是否已设置主密钥
/// 测试专用：只清空内存中的主密钥，保留已持久化的副本（用于模拟重启）
#[cfg(test)]
pub(crate) fn clear_in_memory_master_key_for_tests() {
    if let Ok(mut guard) = ENCRYPTION_KEY.write() {
        *guard = None;
    }
    if let Ok(mut guard) = RAW_MASTER_KEY.write() {
        *guard = None;
    }
}

pub fn has_master_key() -> bool {
    ENCRYPTION_KEY
        .read()
        .map(|guard| guard.is_some())
        .unwrap_or(false)
}

/// 获取原始主密钥
///
/// 返回设置的原始主密钥字符串，用于云同步等需要原始密钥的场景。
/// 如果未设置主密钥，返回 None。
pub fn get_raw_master_key() -> Option<String> {
    RAW_MASTER_KEY
        .read()
        .ok()
        .and_then(|guard| guard.as_ref().map(|key| key.to_string()))
}

/// 加密密码
///
/// 如果未设置主密钥，返回原始密码。
/// 加密后的密码格式：`ENC:base64(nonce + ciphertext)`
pub fn encrypt_password(password: &str) -> String {
    if password.is_empty() || is_encrypted(password) {
        return password.to_string();
    }

    // 有原始口令时一律写自描述密文：密文自带 KDF 参数与盐，重启或换设备后
    // 不需要依赖进程内的「当前规范」也能解开
    if let Some(raw) = get_raw_master_key() {
        let spec = active_kdf_spec().unwrap_or(KdfSpec::LegacySha256);
        if let Some(encrypted) = encrypt_blob_with_spec(password, &raw, &spec) {
            return encrypted;
        }
        if !matches!(spec, KdfSpec::LegacySha256)
            && let Some(encrypted) = encrypt_blob_with_spec(password, &raw, &KdfSpec::LegacySha256)
        {
            return encrypted;
        }
        return password.to_string();
    }

    // 兼容路径：只有派生密钥、没有原始口令时无法写自描述密文
    let Ok(guard) = ENCRYPTION_KEY.read() else {
        return password.to_string();
    };
    let Some(key) = *guard else {
        return password.to_string();
    };
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
    let mut nonce_bytes = [0u8; NONCE_LENGTH];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    match cipher.encrypt(Nonce::from_slice(&nonce_bytes), password.as_bytes()) {
        Ok(ciphertext) => {
            let mut combined = Vec::with_capacity(NONCE_LENGTH + ciphertext.len());
            combined.extend_from_slice(&nonce_bytes);
            combined.extend_from_slice(&ciphertext);
            format!("{}{}", ENCRYPTED_PREFIX, BASE64.encode(&combined))
        }
        Err(_) => password.to_string(),
    }
}

/// 用候选密钥解密同一 nonce 下的密文
fn decrypt_fields(
    nonce_bytes: &[u8; NONCE_LENGTH],
    ciphertext: &[u8],
    keys: &[[u8; 32]],
) -> Option<Vec<u8>> {
    let nonce = Nonce::from_slice(nonce_bytes);
    for key in keys {
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
        if let Ok(plaintext) = cipher.decrypt(nonce, ciphertext) {
            return Some(plaintext);
        }
    }
    None
}

/// 依次用候选密钥解密 nonce + ciphertext 拼接体
fn decrypt_combined(combined: &[u8], keys: &[[u8; 32]]) -> Option<Vec<u8>> {
    if combined.len() < MIN_ENCRYPTED_LENGTH {
        return None;
    }
    let nonce_bytes: [u8; NONCE_LENGTH] = combined[..NONCE_LENGTH].try_into().ok()?;
    decrypt_fields(&nonce_bytes, &combined[NONCE_LENGTH..], keys)
}

/// 全局解密候选密钥：当前派生密钥 + 原始口令按候选 KDF 规范派生的密钥
fn global_decrypt_keys() -> Vec<[u8; 32]> {
    let mut keys = Vec::new();
    if let Ok(guard) = ENCRYPTION_KEY.read()
        && let Some(key) = *guard
    {
        keys.push(key);
    }
    if let Ok(guard) = RAW_MASTER_KEY.read()
        && let Some(raw) = guard.as_ref()
    {
        for key in candidate_keys(raw) {
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
    }
    keys
}

/// 解密密码
///
/// 如果密码未加密（不以 `ENC:` 开头），返回原始密码。
/// 如果未设置主密钥或解密失败，返回空字符串。
pub fn decrypt_password(encrypted: &str) -> String {
    if encrypted.is_empty() {
        return encrypted.to_string();
    }

    // 如果不是加密的，直接返回
    if !encrypted.starts_with(ENCRYPTED_PREFIX) {
        return encrypted.to_string();
    }

    // 自描述密文：用密文里记录的规范派生，不依赖全局 KDF 状态
    if let Some((spec, nonce, ciphertext)) = parse_encrypted_blob(encrypted) {
        let mut keys = Vec::new();
        if let Some(raw) = get_raw_master_key()
            && let Some(key) = derive_key_with_cache(&raw, &spec)
        {
            keys.push(key);
        }
        if let Ok(guard) = ENCRYPTION_KEY.read()
            && let Some(key) = *guard
            && !keys.contains(&key)
        {
            keys.push(key);
        }
        return decrypt_fields(&nonce, &ciphertext, &keys)
            .and_then(|plaintext| String::from_utf8(plaintext).ok())
            .unwrap_or_default();
    }

    // 旧密文：解码 base64 后按候选密钥尝试
    let Some(combined) = decode_legacy_blob(encrypted) else {
        return String::new();
    };
    decrypt_combined(&combined, &global_decrypt_keys())
        .and_then(|plaintext| String::from_utf8(plaintext).ok())
        .unwrap_or_default()
}

/// 检查密码是否已加密
pub fn is_encrypted(password: &str) -> bool {
    password.starts_with(ENCRYPTED_PREFIX)
}

// ============================================================================
// 指定密钥模式的加解密函数（用于云同步和密钥迁移）
// ============================================================================

/// 使用指定密钥加密密码
///
/// 不依赖全局密钥状态，直接使用传入的主密钥进行加密。
/// 适用于云同步场景和密钥迁移场景。
pub fn encrypt_with_key(plaintext: &str, master_key: &str) -> String {
    if plaintext.is_empty() {
        return plaintext.to_string();
    }

    // 如果已经是加密的，直接返回
    if is_encrypted(plaintext) {
        return plaintext.to_string();
    }

    // 当前规范为 Argon2id 时写自描述密文；否则保持旧格式（跨设备仍靠固定盐）
    let spec = active_kdf_spec().unwrap_or(KdfSpec::LegacySha256);
    if let Some(encrypted) = encrypt_blob_with_spec(plaintext, master_key, &spec) {
        return encrypted;
    }
    if !matches!(spec, KdfSpec::LegacySha256)
        && let Some(encrypted) =
            encrypt_blob_with_spec(plaintext, master_key, &KdfSpec::LegacySha256)
    {
        return encrypted;
    }
    plaintext.to_string()
}

/// 使用指定密钥解密密码
///
/// 不依赖全局密钥状态，直接使用传入的主密钥进行解密。
/// 适用于云同步场景和密钥迁移场景。
pub fn decrypt_with_key(encrypted: &str, master_key: &str) -> Result<String, CryptoError> {
    if encrypted.is_empty() {
        return Ok(encrypted.to_string());
    }

    // 如果不是加密的，直接返回
    if !encrypted.starts_with(ENCRYPTED_PREFIX) {
        return Ok(encrypted.to_string());
    }

    // 自描述密文：按密文自带的规范派生，跨设备/重启都能解
    if let Some((spec, nonce, ciphertext)) = parse_encrypted_blob(encrypted) {
        let key = derive_key_with_spec(master_key, &spec).ok_or(CryptoError::DecryptionFailed)?;
        let plaintext =
            decrypt_fields(&nonce, &ciphertext, &[key]).ok_or(CryptoError::DecryptionFailed)?;
        return String::from_utf8(plaintext).map_err(|_| CryptoError::EncodingFailed);
    }

    let encoded = &encrypted[ENCRYPTED_PREFIX.len()..];
    let combined = BASE64
        .decode(encoded)
        .map_err(|_| CryptoError::EncodingFailed)?;

    if combined.len() < MIN_ENCRYPTED_LENGTH {
        return Err(CryptoError::InvalidDataFormat);
    }

    let keys = candidate_keys(master_key);
    match decrypt_combined(&combined, &keys) {
        Some(plaintext) => String::from_utf8(plaintext).map_err(|_| CryptoError::EncodingFailed),
        None => Err(CryptoError::DecryptionFailed),
    }
}

/// 重新加密数据（从旧密钥迁移到新密钥）
///
/// 用于修改主密钥时重新加密已加密的数据。
pub fn re_encrypt_data(
    encrypted_data: &str,
    old_key: &str,
    new_key: &str,
) -> Result<String, CryptoError> {
    // 如果数据未加密，直接用新密钥加密
    if !encrypted_data.starts_with(ENCRYPTED_PREFIX) {
        return Ok(encrypt_with_key(encrypted_data, new_key));
    }

    // 用旧密钥解密
    let plaintext = decrypt_with_key(encrypted_data, old_key)?;

    // 用新密钥重新加密
    Ok(encrypt_with_key(&plaintext, new_key))
}

// ============================================================================
// 修改主密钥相关函数
// ============================================================================

/// 修改主密钥
///
/// 验证旧密钥后更新验证数据和内存中的密钥。
/// 注意：此函数不负责重新加密已有数据，调用方需要自行处理数据迁移。
///
/// # 参数
/// - `old_key`: 当前的主密钥
/// - `new_key`: 新的主密钥
/// - `confirm_new_key`: 确认新密钥（需要与 new_key 一致）
///
/// # 返回
/// - `Ok(())`: 修改成功
/// - `Err(CryptoError)`: 修改失败的原因
pub fn change_master_key(
    old_key: &str,
    new_key: &str,
    confirm_new_key: &str,
) -> Result<(), CryptoError> {
    change_master_key_with_persistence(
        old_key,
        new_key,
        confirm_new_key,
        MasterKeyPersistence::Persistent,
    )
}

/// 修改主密钥，但不为后续启动持久化新密钥。
pub fn change_master_key_for_session(
    old_key: &str,
    new_key: &str,
    confirm_new_key: &str,
) -> Result<(), CryptoError> {
    change_master_key_with_persistence(
        old_key,
        new_key,
        confirm_new_key,
        MasterKeyPersistence::SessionOnly,
    )
}

fn change_master_key_with_persistence(
    old_key: &str,
    new_key: &str,
    confirm_new_key: &str,
    persistence: MasterKeyPersistence,
) -> Result<(), CryptoError> {
    validate_master_key_change(old_key, new_key, confirm_new_key)?;

    // 沿用当前信封的 KDF 规范：旧版安装改密码时不能悄悄切到 Argon2，
    // 否则新信封与尚未重加密的数据会短暂不一致（KDF 升级由迁移流程单独完成）。
    let new_verification = match load_verification_data() {
        Some(existing) => {
            let spec = kdf_spec_from_verification(&existing)
                .or_else(active_kdf_spec)
                .unwrap_or(KdfSpec::LegacySha256);
            generate_key_verification_with_spec(new_key, &spec)
                .ok_or(CryptoError::SaveVerificationFailed)?
        }
        None => generate_key_verification(new_key),
    };
    let storage = key_storage::get_key_storage();
    persist_master_key_change(storage.as_ref(), new_key, persistence, || {
        save_verification_data(&new_verification)
    })?;
    publish_master_key(new_key);
    Ok(())
}

pub fn validate_master_key_change(
    old_key: &str,
    new_key: &str,
    confirm_new_key: &str,
) -> Result<(), CryptoError> {
    if !has_repo_password_set() {
        return Err(CryptoError::NoPasswordSet);
    }

    if let Some(verification_data) = load_verification_data() {
        if !verify_master_key(old_key, &verification_data) {
            return Err(CryptoError::InvalidOldPassword);
        }
    } else {
        return Err(CryptoError::NoPasswordSet);
    }

    if new_key.is_empty() {
        return Err(CryptoError::EmptyNewPassword);
    }

    if new_key != confirm_new_key {
        return Err(CryptoError::PasswordMismatch);
    }
    Ok(())
}

fn persist_master_key_change(
    storage: &dyn key_storage::KeyStorage,
    new_key: &str,
    persistence: MasterKeyPersistence,
    save_verification: impl FnOnce() -> bool,
) -> Result<(), CryptoError> {
    let previous = storage.load();
    if let Err(error) = update_key_storage(storage, new_key, persistence) {
        restore_key_storage(storage, previous)?;
        return Err(CryptoError::KeyStorageFailed(error));
    }
    if save_verification() {
        return Ok(());
    }
    restore_key_storage(storage, previous)?;
    Err(CryptoError::SaveVerificationFailed)
}

fn restore_key_storage(
    storage: &dyn key_storage::KeyStorage,
    previous: Option<String>,
) -> Result<(), CryptoError> {
    let result = match previous {
        Some(key) => storage.save(&key),
        None => storage.delete(),
    };
    result.map_err(|error| CryptoError::KeyStorageFailed(format!("恢复旧密钥失败: {error}")))
}

fn publish_master_key(master_key: &str) {
    let key = derive_key(master_key);
    if let Ok(mut guard) = ENCRYPTION_KEY.write() {
        *guard = Some(key);
    }

    if let Ok(mut guard) = RAW_MASTER_KEY.write() {
        *guard = Some(Zeroizing::new(master_key.to_string()));
    }
}

/// 重置主密钥
///
/// 删除验证文件，允许用户重新设置密钥。
/// 警告：这将导致所有已加密的密码无法解密！
pub fn reset_repo_password() -> bool {
    // 清除内存中的密钥
    clear_master_key();
    if let Ok(mut guard) = ACTIVE_KDF_SPEC.write() {
        *guard = None;
    }
    if let Ok(mut known) = KNOWN_KDF_SPECS.lock() {
        known.clear();
    }

    // 删除验证文件
    if let Some(path) = get_verification_file_path()
        && path.exists()
    {
        return fs::remove_file(path).is_ok();
    }
    true
}

/// 获取当前密钥的验证数据
///
/// 用于云端存储，验证用户输入的密钥是否正确。
pub fn get_current_verification_data() -> Option<String> {
    load_verification_data()
}

/// 判断验证数据是否为新版 Argon2id 信封
pub fn is_kdf_envelope(verification_data: &str) -> bool {
    verification_data.starts_with(KDF_ENVELOPE_PREFIX)
}

/// 验证数据是否需要升级到 Argon2id（旧版固定盐 SHA-256 格式）
pub fn verification_needs_kdf_upgrade(verification_data: &str) -> bool {
    !verification_data.is_empty() && !is_kdf_envelope(verification_data)
}

/// 生成一份新的 Argon2id 验证信封，但不改变当前生效的 KDF。
///
/// 调用方应先落盘信封、再调用 [`activate_kdf_from_verification`]：
/// 即使迁移中途崩溃，下次启动仍能用新信封解锁，旧密文由 legacy 派生兜底。
pub fn create_upgraded_verification(master_key: &str) -> Option<String> {
    generate_key_verification_with_spec(master_key, &default_kdf_spec())
}

/// 采用验证数据中记录的 KDF 规范（成功识别返回 true）。
///
/// 只更新内存中的 KDF 规范与已知规范集合，不改动已派生的全局密钥；
/// 需要同步刷新的场景（如 KDF 迁移）请再调用 [`republish_master_key`]。
pub fn activate_kdf_from_verification(verification_data: &str) -> bool {
    match parse_verification_data(verification_data) {
        Some(ParsedVerification::Legacy(_)) => {
            set_active_kdf_spec(KdfSpec::LegacySha256);
            true
        }
        Some(ParsedVerification::Argon2 { spec, .. }) => {
            set_active_kdf_spec(spec);
            true
        }
        None => false,
    }
}

/// 持久化验证数据（原子写入），用于 KDF 升级时先落盘新信封
pub fn save_verification(verification_data: &str) -> bool {
    save_verification_data(verification_data)
}

/// 用当前生效的 KDF 重新派生并发布内存中的主密钥
pub fn republish_master_key() -> bool {
    let Some(master_key) = get_raw_master_key() else {
        return false;
    };
    publish_master_key(&master_key);
    true
}

/// 从存储后端恢复主密钥
///
/// 尝试从当前配置的存储后端读取密钥。
/// 如果密钥存在且验证通过，则自动设置到内存。
/// 返回是否成功恢复。
pub fn try_restore_master_key() -> bool {
    let storage = key_storage::get_key_storage();
    tracing::info!("[密钥恢复] 尝试从「{}」恢复主密钥...", storage.name());

    // 如果没有设置过密码验证文件，不需要恢复
    if !has_repo_password_set() {
        tracing::info!("[密钥恢复] 未设置过主密钥，跳过恢复");
        return false;
    }

    // 从存储后端读取
    let master_key = match storage.load() {
        Some(key) => key,
        None => {
            tracing::warn!(
                "[密钥恢复] 从「{}」读取失败，需要用户手动输入",
                storage.name()
            );
            return false;
        }
    };

    // 验证密钥是否正确
    if let Some(verification_data) = load_verification_data() {
        if !verify_master_key(&master_key, &verification_data) {
            tracing::warn!("[密钥恢复] 密钥验证失败，可能密码已修改");
            let _ = storage.delete();
            return false;
        }
        // 先采用信封记录的 KDF 规范，再用它派生内存密钥
        activate_kdf_from_verification(&verification_data);
    }

    // 直接设置密钥到内存（不再保存，避免循环）
    let key = derive_key(&master_key);
    if let Ok(mut guard) = ENCRYPTION_KEY.write() {
        *guard = Some(key);
    }
    if let Ok(mut guard) = RAW_MASTER_KEY.write() {
        *guard = Some(Zeroizing::new(master_key));
    }

    tracing::info!("[密钥恢复] 主密钥恢复成功");
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::key_storage::KeyStorage;
    use std::sync::Mutex;

    #[derive(Default)]
    struct MemoryKeyStorage {
        key: Mutex<Option<String>>,
    }

    struct FailOnceKeyStorage {
        key: Mutex<Option<String>>,
        fail_next_save: Mutex<bool>,
        fail_next_delete: Mutex<bool>,
    }

    impl FailOnceKeyStorage {
        fn fail_save(key: &str) -> Self {
            Self {
                key: Mutex::new(Some(key.to_string())),
                fail_next_save: Mutex::new(true),
                fail_next_delete: Mutex::new(false),
            }
        }

        fn fail_delete(key: &str) -> Self {
            Self {
                key: Mutex::new(Some(key.to_string())),
                fail_next_save: Mutex::new(false),
                fail_next_delete: Mutex::new(true),
            }
        }
    }

    impl KeyStorage for MemoryKeyStorage {
        fn name(&self) -> &'static str {
            "test-memory"
        }

        fn save(&self, master_key: &str) -> Result<(), String> {
            *self.key.lock().expect("test key storage lock") = Some(master_key.to_string());
            Ok(())
        }

        fn load(&self) -> Option<String> {
            self.key.lock().expect("test key storage lock").clone()
        }

        fn delete(&self) -> Result<(), String> {
            *self.key.lock().expect("test key storage lock") = None;
            Ok(())
        }

        fn exists(&self) -> bool {
            self.key.lock().expect("test key storage lock").is_some()
        }
    }

    impl KeyStorage for FailOnceKeyStorage {
        fn name(&self) -> &'static str {
            "test-fail-once"
        }

        fn save(&self, master_key: &str) -> Result<(), String> {
            *self.key.lock().expect("test key storage lock") = Some(master_key.to_string());
            let mut fail = self.fail_next_save.lock().expect("test failure state lock");
            if *fail {
                *fail = false;
                return Err("injected save failure".to_string());
            }
            Ok(())
        }

        fn load(&self) -> Option<String> {
            self.key.lock().expect("test key storage lock").clone()
        }

        fn delete(&self) -> Result<(), String> {
            *self.key.lock().expect("test key storage lock") = None;
            let mut fail = self
                .fail_next_delete
                .lock()
                .expect("test failure state lock");
            if *fail {
                *fail = false;
                return Err("injected delete failure".to_string());
            }
            Ok(())
        }

        fn exists(&self) -> bool {
            self.key.lock().expect("test key storage lock").is_some()
        }
    }

    fn test_mutex() -> &'static Mutex<()> {
        super::crypto_test_lock()
    }

    fn crypto_guard() -> std::sync::MutexGuard<'static, ()> {
        // Tolerate poisoning: a panic in any test while holding the shared
        // master-key lock must not cascade-fail every other lock acquisition.
        test_mutex()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    #[test]
    fn test_encrypt_decrypt() {
        let _guard = crypto_guard();
        set_master_key("test_key_123").expect("configure test master key");

        let original = "my_secret_password";
        let encrypted = encrypt_password(original);

        assert!(encrypted.starts_with(ENCRYPTED_PREFIX));
        assert_ne!(encrypted, original);

        let decrypted = decrypt_password(&encrypted);
        assert_eq!(decrypted, original);

        clear_master_key();
    }

    #[test]
    fn test_key_verification() {
        let _guard = crypto_guard();
        let master_key = "test_key_123";
        let verification = generate_key_verification(master_key);

        assert!(verify_master_key(master_key, &verification));
        assert!(!verify_master_key("wrong_key", &verification));
    }

    #[test]
    fn test_empty_password() {
        let _guard = crypto_guard();
        set_master_key("test_key").expect("configure test master key");

        let encrypted = encrypt_password("");
        assert_eq!(encrypted, "");

        let decrypted = decrypt_password("");
        assert_eq!(decrypted, "");

        clear_master_key();
    }

    #[test]
    fn test_no_master_key() {
        let _guard = crypto_guard();
        clear_master_key();

        let original = "password123";
        let result = encrypt_password(original);
        assert_eq!(result, original); // 未设置密钥时不加密

        let decrypted = decrypt_password(&format!("{}abc", ENCRYPTED_PREFIX));
        assert_eq!(decrypted, ""); // 未设置密钥时解密返回空

        clear_master_key();
    }

    #[test]
    fn verification_files_stay_outside_the_real_data_dir() {
        let isolated = get_data_dir().expect("isolated test data dir");

        assert_ne!(
            crate::app_dirs::data_dir().as_deref(),
            Some(isolated.as_path()),
            "单元测试的数据目录必须与真实用户数据目录隔离"
        );
        assert!(isolated.starts_with(std::env::temp_dir()));
    }

    #[test]
    fn verification_files_follow_the_isolated_test_data_dir() {
        let _guard = crypto_guard();
        crate::key_storage::with_test_data_dir(|data_dir| {
            assert_eq!(
                Some(data_dir.join(KEY_VERIFICATION_FILE)),
                get_verification_file_path(),
                "with_test_data_dir 生效时验证文件必须跟着进临时目录"
            );
        });
    }

    #[test]
    fn test_already_encrypted() {
        let _guard = crypto_guard();
        set_master_key("test_key").expect("configure test master key");

        let original = "password";
        let encrypted = encrypt_password(original);
        let double_encrypted = encrypt_password(&encrypted);

        // 已加密的不应再次加密
        assert_eq!(encrypted, double_encrypted);

        clear_master_key();
    }

    #[test]
    fn session_only_master_key_forgets_any_persisted_copy() {
        let storage = MemoryKeyStorage::default();
        storage.save("stale-key").expect("seed persisted key");

        update_key_storage(&storage, "session-key", MasterKeyPersistence::SessionOnly)
            .expect("session-only persistence policy");

        assert_eq!(None, storage.load());
    }

    #[test]
    fn remembered_master_key_is_saved_for_future_startups() {
        let storage = MemoryKeyStorage::default();

        update_key_storage(&storage, "remembered-key", MasterKeyPersistence::Persistent)
            .expect("persistent policy");

        assert_eq!(Some("remembered-key".to_string()), storage.load());
    }

    #[test]
    fn decrypt_with_key_rejects_data_without_a_complete_authentication_tag() {
        for length in [12, 27] {
            let encrypted = format!("{}{}", ENCRYPTED_PREFIX, BASE64.encode(vec![0_u8; length]));
            assert_eq!(
                Err(CryptoError::InvalidDataFormat),
                decrypt_with_key(&encrypted, "test-key")
            );
        }
    }

    #[test]
    fn master_key_change_restores_persisted_key_when_verification_save_fails() {
        let storage = MemoryKeyStorage::default();
        storage.save("old-key").expect("seed old key");

        let result = persist_master_key_change(
            &storage,
            "new-key",
            MasterKeyPersistence::Persistent,
            || false,
        );

        assert_eq!(Err(CryptoError::SaveVerificationFailed), result);
        assert_eq!(Some("old-key".to_string()), storage.load());
    }

    #[test]
    fn master_key_change_restores_persisted_key_after_partial_storage_failure() {
        let storage = FailOnceKeyStorage::fail_save("old-key");

        let result = persist_master_key_change(
            &storage,
            "new-key",
            MasterKeyPersistence::Persistent,
            || true,
        );

        assert!(matches!(result, Err(CryptoError::KeyStorageFailed(_))));
        assert_eq!(Some("old-key".to_string()), storage.load());
    }

    #[test]
    fn first_setup_does_not_publish_when_verification_save_fails() {
        let _guard = crypto_guard();
        let storage = MemoryKeyStorage::default();
        clear_in_memory_master_key();

        let result = set_master_key_with_storage(
            MasterKeySetup {
                master_key: "new-key",
                persistence: MasterKeyPersistence::Persistent,
                create_verification: true,
            },
            &storage,
            |_| false,
        );

        assert_eq!(Err(CryptoError::SaveVerificationFailed), result);
        assert_eq!(None, storage.load());
        assert!(!has_master_key());
    }

    #[test]
    fn persistent_unlock_degrades_when_system_store_rejects_save() {
        let _guard = crypto_guard();
        let storage = FailOnceKeyStorage::fail_save("stale-key");
        clear_in_memory_master_key();

        let result = set_master_key_with_storage(
            MasterKeySetup {
                master_key: "new-key",
                persistence: MasterKeyPersistence::Persistent,
                create_verification: false,
            },
            &storage,
            |_| true,
        );

        // 系统凭据库写入失败不应阻塞解锁，只是本次会话不再自动保存
        assert_eq!(Ok(()), result);
        assert!(has_master_key());
        assert_eq!(None, storage.load(), "残留的旧副本应被尽力清理");
        clear_in_memory_master_key();
    }

    #[test]
    fn session_only_unlock_does_not_publish_after_partial_delete_failure() {
        let _guard = crypto_guard();
        let storage = FailOnceKeyStorage::fail_delete("old-key");
        clear_in_memory_master_key();

        let result = set_master_key_with_storage(
            MasterKeySetup {
                master_key: "new-key",
                persistence: MasterKeyPersistence::SessionOnly,
                create_verification: false,
            },
            &storage,
            |_| true,
        );

        assert!(matches!(result, Err(CryptoError::KeyStorageFailed(_))));
        assert_eq!(Some("old-key".to_string()), storage.load());
        assert!(!has_master_key());
    }

    #[test]
    fn successful_setup_publishes_after_persistence() {
        let _guard = crypto_guard();
        let storage = MemoryKeyStorage::default();
        clear_in_memory_master_key();

        let result = set_master_key_with_storage(
            MasterKeySetup {
                master_key: "new-key",
                persistence: MasterKeyPersistence::Persistent,
                create_verification: true,
            },
            &storage,
            |_| true,
        );

        assert_eq!(Ok(()), result);
        assert_eq!(Some("new-key".to_string()), storage.load());
        assert!(has_master_key());
        clear_in_memory_master_key();
    }

    #[test]
    fn self_describing_ciphertext_does_not_depend_on_global_kdf_state() {
        let _guard = crypto_guard();
        reset_kdf_state();
        set_active_kdf_spec(KdfSpec::Argon2id {
            params: KdfParams::for_tests(),
            salt: [3u8; KDF_SALT_LENGTH],
        });
        set_master_key("cross-device-key").expect("configure test master key");

        let encrypted = encrypt_password("cloud-secret");
        assert!(encrypted.starts_with(ENCRYPTED_PREFIX));

        // 换设备/重启：没有任何进程内状态，仅凭口令与密文本身即可解开
        assert_eq!(
            "cloud-secret",
            decrypt_with_key(&encrypted, "cross-device-key").expect("cross-device decrypt")
        );

        // 重新输入同一口令后仍能解，即使本次派生用了另一个盐
        clear_in_memory_master_key();
        reset_kdf_state();
        publish_master_key("cross-device-key");
        assert_eq!("cloud-secret", decrypt_password(&encrypted));

        clear_master_key();
        reset_kdf_state();
    }

    #[test]
    fn legacy_blob_stays_readable_after_kdf_upgrade() {
        let _guard = crypto_guard();
        reset_kdf_state();
        let blob = encrypt_with_key("legacy-secret", "legacy-key");
        assert!(blob.starts_with(ENCRYPTED_PREFIX));

        // 升级到 Argon2 后，旧 ENC: 密文仍可解（候选密钥包含 legacy 派生）
        set_active_kdf_spec(KdfSpec::Argon2id {
            params: KdfParams::for_tests(),
            salt: [7u8; KDF_SALT_LENGTH],
        });
        assert_eq!(
            "legacy-secret",
            decrypt_with_key(&blob, "legacy-key").expect("legacy decrypt")
        );
        reset_kdf_state();
    }

    #[test]
    fn unlock_rejects_an_unreadable_verification_file() {
        assert_eq!(
            Err(CryptoError::LoadVerificationFailed),
            verify_existing_master_key("test-key", None)
        );
    }

    fn clear_in_memory_master_key() {
        *ENCRYPTION_KEY.write().expect("test encryption key lock") = None;
        *RAW_MASTER_KEY.write().expect("test raw master key lock") = None;
    }

    fn reset_kdf_state() {
        if let Ok(mut guard) = ACTIVE_KDF_SPEC.write() {
            *guard = None;
        }
        if let Ok(mut known) = KNOWN_KDF_SPECS.lock() {
            known.clear();
        }
        if let Ok(mut cache) = DERIVED_KEY_CACHE.lock() {
            cache.clear();
        }
        clear_in_memory_master_key();
    }

    #[test]
    fn fresh_setup_uses_argon2_envelope() {
        let _guard = crypto_guard();
        reset_kdf_state();

        let verification = generate_key_verification("fresh-password");

        assert!(is_kdf_envelope(&verification));
        assert!(verify_master_key("fresh-password", &verification));
        assert!(!verify_master_key("wrong-password", &verification));
        assert!(matches!(
            parse_verification_data(&verification),
            Some(ParsedVerification::Argon2 { .. })
        ));
        reset_kdf_state();
    }

    #[test]
    fn legacy_verification_upgrades_without_losing_read_access() {
        let _guard = crypto_guard();
        reset_kdf_state();
        let master_key = "legacy-password";

        let legacy = generate_key_verification_with_spec(master_key, &KdfSpec::LegacySha256)
            .expect("legacy verification");
        assert!(!is_kdf_envelope(&legacy));
        assert!(verification_needs_kdf_upgrade(&legacy));
        assert!(verify_master_key(master_key, &legacy));

        // 旧安装写出的密文
        set_active_kdf_spec(KdfSpec::LegacySha256);
        let legacy_ciphertext = encrypt_with_key("old-secret", master_key);

        // 生成新信封并落盘（这里只验证内存行为）
        let upgraded = create_upgraded_verification(master_key).expect("upgraded envelope");
        assert!(is_kdf_envelope(&upgraded));
        assert!(!verification_needs_kdf_upgrade(&upgraded));
        assert!(verify_master_key(master_key, &upgraded));

        // 采用新规范后旧密文仍可读（legacy 作为候选）
        assert!(activate_kdf_from_verification(&upgraded));
        assert_eq!(
            "old-secret",
            decrypt_with_key(&legacy_ciphertext, master_key).expect("旧密文仍可解密")
        );

        // 新写入使用新规范，新旧密文互不影响
        let new_ciphertext = encrypt_with_key("new-secret", master_key);
        assert_eq!(
            "new-secret",
            decrypt_with_key(&new_ciphertext, master_key).expect("新密文可解密")
        );
        assert_eq!(
            "old-secret",
            decrypt_with_key(&legacy_ciphertext, master_key).expect("旧密文仍可解密")
        );

        reset_kdf_state();
    }

    #[test]
    fn kdf_envelope_rejects_tampered_parameters() {
        let _guard = crypto_guard();
        reset_kdf_state();

        let base = KdfEnvelopeRecord {
            version: KDF_ENVELOPE_VERSION,
            kdf: KDF_NAME.to_string(),
            memory_kib: 32,
            iterations: 1,
            parallelism: 1,
            salt: BASE64.encode([7u8; KDF_SALT_LENGTH]),
            nonce: BASE64.encode([9u8; NONCE_LENGTH]),
            verifier: BASE64.encode([3u8; AUTH_TAG_LENGTH]),
        };

        let encoded = serde_json::to_vec(&base).expect("serialize envelope");
        let envelope = format!("{KDF_ENVELOPE_PREFIX}{}", BASE64.encode(&encoded));
        assert!(parse_verification_data(&envelope).is_some());

        let mut oversized = base.clone();
        oversized.memory_kib = u32::MAX;
        let encoded = serde_json::to_vec(&oversized).expect("serialize envelope");
        let envelope = format!("{KDF_ENVELOPE_PREFIX}{}", BASE64.encode(&encoded));
        assert!(parse_verification_data(&envelope).is_none());

        let mut wrong_version = base;
        wrong_version.version = 99;
        let encoded = serde_json::to_vec(&wrong_version).expect("serialize envelope");
        let envelope = format!("{KDF_ENVELOPE_PREFIX}{}", BASE64.encode(&encoded));
        assert!(parse_verification_data(&envelope).is_none());

        assert!(parse_verification_data("not-base64!!").is_none());
        assert!(!verify_master_key("any", "not-base64!!"));
    }
}
