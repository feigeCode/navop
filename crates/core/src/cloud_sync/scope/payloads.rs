//! 新增可同步类型的明文载荷与加解密
//!
//! 沿用现有信封：`明文 JSON → AES-256-GCM（主密钥）→ base64(nonce+ciphertext)`，
//! checksum 取明文的 SHA-256。所有载荷都带 `format_version`，便于后续演进。

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::cloud_sync::models::{CloudSyncData, data_type};
use crate::cloud_sync::service::{CloudSyncService, SyncError};

/// 载荷格式版本
pub const PAYLOAD_FORMAT_VERSION: u32 = 1;

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 加密一个明文载荷为云端记录
pub fn build_record(
    service: &CloudSyncService,
    record_type: &str,
    plain: &impl Serialize,
) -> Result<CloudSyncData, SyncError> {
    build_record_with_id(
        service,
        record_type,
        plain,
        uuid::Uuid::new_v4().to_string(),
    )
}

/// 加密并指定云端记录 ID（Singleton 与「重新上传已有记录」场景使用）
pub fn build_record_with_id(
    service: &CloudSyncService,
    record_type: &str,
    plain: &impl Serialize,
    id: String,
) -> Result<CloudSyncData, SyncError> {
    let plaintext =
        serde_json::to_string(plain).map_err(|e| SyncError::DataFormatError(e.to_string()))?;
    let checksum = CloudSyncService::calculate_blob_checksum(&plaintext);
    let encrypted_data = service.encrypt_blob(&plaintext, None)?;

    Ok(CloudSyncData {
        id,
        owner_id: service.user_id().unwrap_or_default().to_string(),
        team_id: None,
        data_type: record_type.to_string(),
        encrypted_data,
        key_version: service.key_version(),
        checksum,
        version: 1,
        updated_at: now_millis(),
        deleted_at: None,
    })
}

/// 解密云端记录为明文载荷
pub fn parse_record<T: DeserializeOwned>(
    service: &CloudSyncService,
    data: &CloudSyncData,
) -> Result<T, SyncError> {
    let plaintext = service.decrypt_blob(&data.encrypted_data, None)?;
    serde_json::from_str(&plaintext).map_err(|e| SyncError::DataFormatError(e.to_string()))
}

// ============================================================================
// 快捷命令
// ============================================================================

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuickCommandPlainData {
    pub format_version: u32,
    pub name: Option<String>,
    pub command: String,
    pub description: Option<String>,
    pub shortcut: Option<String>,
    pub pinned: bool,
    pub sort_order: i32,
    pub group_name: Option<String>,
    pub group_color: Option<String>,
    /// 关联连接的云端 ID；本地整数 ID 跨设备无意义
    pub connection_cloud_id: Option<String>,
}

impl QuickCommandPlainData {
    pub fn data_type() -> &'static str {
        data_type::QUICK_COMMAND
    }
}

// ============================================================================
// SFTP 收藏
// ============================================================================

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SftpFavoritePlainData {
    pub format_version: u32,
    pub path: String,
    pub sort_order: i32,
    /// 关联连接的云端 ID
    pub connection_cloud_id: Option<String>,
    /// 业务唯一键的兜底：连接不可同步时按连接名匹配
    pub connection_name: Option<String>,
}

impl SftpFavoritePlainData {
    pub fn data_type() -> &'static str {
        data_type::SFTP_FAVORITE
    }
}

// ============================================================================
// 已保存的 SQL
// ============================================================================

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedSqlPlainData {
    pub format_version: u32,
    /// 稳定身份：跨重命名/移动保持不变，同步用它当键
    pub document_id: String,
    /// 数据库类型（复合键 part 1）
    pub db_type: String,
    /// 连接名（复合键 part 2，跨设备稳定）
    pub connection_cloud_id: Option<String>,
    /// 库名（复合键 part 3）
    pub database: Option<String>,
    /// 文件名（不含扩展名，复合键 part 4）
    pub name: String,
    pub content: String,
}

impl SavedSqlPlainData {
    pub fn data_type() -> &'static str {
        data_type::SAVED_SQL
    }

    /// 跨设备稳定的业务键
    pub fn business_key(&self) -> String {
        format!(
            "{}|{}|{}|{}",
            self.db_type,
            self.connection_cloud_id.clone().unwrap_or_default(),
            self.database.clone().unwrap_or_default(),
            self.name
        )
    }
}

// ============================================================================
// 笔记
// ============================================================================

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotePlainData {
    pub format_version: u32,
    pub notebook_id: String,
    pub notebook_name: String,
    pub document_id: String,
    /// 笔记本内的相对路径，用于还原树形结构
    pub relative_path: String,
    pub title: String,
    pub content: String,
}

impl NotePlainData {
    pub fn data_type() -> &'static str {
        data_type::NOTE
    }
}

/// 单篇笔记正文上限（Q4）：超过则跳过不同步
pub const NOTE_MAX_CONTENT_BYTES: usize = 1024 * 1024;

// ============================================================================
// LLM 提供商
// ============================================================================

/// ⚠️ 含 api_key，**不派生 Debug**，避免密钥被调试日志输出
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LlmProviderPlainData {
    pub format_version: u32,
    pub name: String,
    pub provider_type: String,
    pub api_key: Option<String>,
    pub api_base: Option<String>,
    pub api_version: Option<String>,
    pub model: String,
    pub models: Vec<String>,
    pub max_tokens: Option<i32>,
    pub temperature_milli: Option<i64>,
    pub enabled: bool,
    pub is_default: bool,
}

impl LlmProviderPlainData {
    pub fn data_type() -> &'static str {
        data_type::LLM_PROVIDER
    }

    /// 内置提供商（Navop AI）由每台设备本地自建，不参与同步
    pub fn is_builtin(&self) -> bool {
        self.provider_type == "onetcli" || self.provider_type == "OnetCli"
    }
}

// ============================================================================
// 设置类 Singleton
// ============================================================================

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SettingsPlainData {
    pub format_version: u32,
    /// 分组名，与 `SettingsGroup` 对应
    pub group: String,
    /// 白名单字段的载荷
    pub payload: serde_json::Value,
}

impl SettingsPlainData {
    pub fn data_type_for_group(group: &str) -> Option<&'static str> {
        super::settings_payload::SettingsGroup::ALL
            .iter()
            .find(|g| g.type_name() == group)
            .map(|g| g.data_type())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_sql_business_key_is_stable_across_field_order() {
        let a = SavedSqlPlainData {
            format_version: PAYLOAD_FORMAT_VERSION,
            document_id: "doc-1".to_string(),
            db_type: "mysql".to_string(),
            connection_cloud_id: Some("cloud-1".to_string()),
            database: Some("shop".to_string()),
            name: "daily".to_string(),
            content: "select 1".to_string(),
        };
        let b = SavedSqlPlainData { ..a.clone() };

        assert_eq!(a.business_key(), b.business_key());
        assert!(a.business_key().contains("mysql"));
    }

    #[test]
    fn builtin_llm_provider_is_detected() {
        let builtin = LlmProviderPlainData {
            format_version: PAYLOAD_FORMAT_VERSION,
            name: "Navop AI".to_string(),
            provider_type: "onetcli".to_string(),
            api_key: None,
            api_base: None,
            api_version: None,
            model: String::new(),
            models: Vec::new(),
            max_tokens: None,
            temperature_milli: None,
            enabled: true,
            is_default: false,
        };
        assert!(builtin.is_builtin());

        let custom = LlmProviderPlainData {
            provider_type: "openai".to_string(),
            api_key: Some("sk-test".to_string()),
            ..builtin.clone()
        };
        assert!(!custom.is_builtin());
    }

    #[test]
    fn settings_data_type_resolves_by_group_name() {
        assert_eq!(
            Some(data_type::SETTINGS_KEYBINDINGS),
            SettingsPlainData::data_type_for_group("keybindings")
        );
        assert_eq!(
            Some(data_type::SETTINGS_GENERAL),
            SettingsPlainData::data_type_for_group("general")
        );
        assert_eq!(None, SettingsPlainData::data_type_for_group("nope"));
    }
}
