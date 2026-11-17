//! 连接迁移包格式与导入规划。
//!
//! 迁移包是跨产品搬运连接的唯一载体：全量产品与两个独立产品读写同一份
//! `{version, exported_at, app_id, workspaces, connections, encrypted_credentials}`
//! JSON；`encrypted_credentials` 的明文载荷里还包含被连接引用的钥匙串条目。
//! 本模块只做解析与纯函数规划，真正落库（仓库插入、事件广播）在
//! `import/mod.rs`。

pub(crate) mod plan;

use one_core::storage::SshAccountExpect;
use serde::{Deserialize, Serialize};

pub use plan::ImportReport;

/// 迁移包格式版本；读取比它更高的版本时直接拒绝，避免误读未来字段。
///
/// v2 起工作区带有包内 `id` / `parent`，用来保留工作区层级；v1 包没有这两个字段，
/// 读进来是平铺的一层工作区，与升级前的行为一致。
/// v3 起连接凭据（密码/密钥等敏感字段）以及被连接引用的钥匙串条目不再以明文
/// 出现在包内，而是整体用导出密码加密后放在
/// [`ConnectionBundle::encrypted_credentials`]；v2 及更早的包没有该字段，仍按
/// 原行为导入（需要用户解锁源主密钥才能使用密码）。
pub const BUNDLE_VERSION: u32 = 3;

/// 迁移包加密凭据的格式标识；将来调整密文结构时据此兼容旧包。
pub const CREDENTIALS_FORMAT: &str = "navop-credentials-v1";

/// 加密凭据载荷的版本；只增不改，读到更高版本时拒绝。
///
/// v2 起载荷额外携带被连接引用的钥匙串条目（`credentials`）。旧版本读到 v2
/// 会直接拒绝，避免它忽略条目后把连接的 `credential_reference` 原样带到
/// 目标端、撞上无关的本地 id。
pub const CREDENTIALS_PAYLOAD_VERSION: u32 = 2;

/// 工作区在迁移包里的稳定标识；只在包内有效，导入后会重新分配。
pub(crate) fn workspace_bundle_id(local_id: i64) -> String {
    format!("w{local_id}")
}

/// 跨产品搬运的连接与工作区快照。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConnectionBundle {
    /// 格式版本，当前为 [`BUNDLE_VERSION`]。
    pub version: u32,
    /// 导出时间（Unix 秒），仅用于展示。
    #[serde(default)]
    pub exported_at: i64,
    /// 导出方产品 id（如 `navop` / `navop-shell`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_id: Option<String>,
    /// 导出方的工作区；导入时按名称匹配，缺失则新建。
    #[serde(default)]
    pub workspaces: Vec<BundleWorkspace>,
    /// 导出方的连接。
    #[serde(default)]
    pub connections: Vec<BundleConnection>,
    /// v3 起：连接敏感字段以及被引用钥匙串条目的密文，用导出时用户输入的
    /// 密码加密。当前导出始终写入该字段；读取 v2 及更早的包时为 `None`，
    /// 此时导入不需要密码。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encrypted_credentials: Option<EncryptedCredentials>,
}

/// 用导出密码加密后的连接凭据。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EncryptedCredentials {
    /// 密文格式标识，当前为 [`CREDENTIALS_FORMAT`]。
    pub format: String,
    /// `ENC:` 前缀的自描述密文，明文是 [`CredentialsPayload`] 的 JSON。
    pub ciphertext: String,
}

/// 加密凭据的明文载荷；整体作为一段 JSON 加密，避免逐个连接派生密钥。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CredentialsPayload {
    /// 载荷版本，当前为 [`CREDENTIALS_PAYLOAD_VERSION`]。
    pub version: u32,
    /// 每个连接的完整参数（含敏感字段）；按 `index` 对应迁移包中的连接。
    #[serde(default)]
    pub connections: Vec<ConnectionCredentials>,
    /// 被连接引用到的钥匙串条目；旧版 v3 包没有该字段。
    #[serde(default)]
    pub credentials: Vec<BundleCredential>,
}

/// 单个连接的完整参数，按数组下标对应 `ConnectionBundle::connections`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConnectionCredentials {
    /// `connections` 数组中的下标。
    pub index: usize,
    /// 已解密、含密码/密钥的完整 `params` JSON 字符串。
    pub params: String,
}

/// 迁移包里的钥匙串条目，只存在于加密载荷中。
///
/// 字段与 [`one_core::storage::CredentialEntry`] 对齐，但不携带 team/owner 与
/// 本地时间戳。`id` 保留源端本地 id，用于重映射连接里的
/// `credential_reference`；`cloud_id` 仅用于导入时复用目标端已有条目，命中
/// 不到会新建本地条目并清空云归属，避免把源账号的同步元数据带到目标账号。
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct BundleCredential {
    /// 源端本地 id，用于重映射连接参数。
    pub id: i64,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub private_key_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub private_key_content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub passphrase: Option<String>,
    #[serde(default, skip_serializing_if = "SshAccountExpect::is_empty")]
    pub ssh_expect: SshAccountExpect,
    #[serde(default)]
    pub sync_enabled: bool,
    #[serde(default)]
    pub forward_to_agent: bool,
    /// 源端云 id，仅用于导入时按云 id 复用目标端已有条目。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cloud_id: Option<String>,
}

impl std::fmt::Debug for BundleCredential {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BundleCredential")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("username", &self.username)
            .field("password", &self.password.as_ref().map(|_| "<redacted>"))
            .field(
                "private_key_path",
                &self.private_key_path.as_ref().map(|_| "<local-path>"),
            )
            .field(
                "private_key_content",
                &self.private_key_content.as_ref().map(|_| "<redacted>"),
            )
            .field(
                "passphrase",
                &self.passphrase.as_ref().map(|_| "<redacted>"),
            )
            .field(
                "ssh_expect",
                &(!self.ssh_expect.is_empty()).then_some("<redacted>"),
            )
            .field("sync_enabled", &self.sync_enabled)
            .field("forward_to_agent", &self.forward_to_agent)
            .field("cloud_id", &self.cloud_id)
            .finish()
    }
}

/// 迁移包中的工作区。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BundleWorkspace {
    /// 包内稳定标识（v2 起）；导入时按它还原父子关系，本地 id 会重新分配。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// 父工作区的包内标识；`None` 表示顶层工作区。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
}

/// 迁移包中的连接；`params` 已脱敏，只保留非敏感字段用于展示与匹配。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BundleConnection {
    pub name: String,
    pub connection_type: String,
    #[serde(default)]
    pub params: String,
    /// 所属工作区名称，不是 id：跨产品时 id 会重新分配。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remark: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_databases: Option<String>,
    #[serde(default = "default_sync_enabled")]
    pub sync_enabled: bool,
}

fn default_sync_enabled() -> bool {
    true
}

#[cfg(test)]
mod tests;
