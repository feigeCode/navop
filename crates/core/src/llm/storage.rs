use anyhow::Result;
use gpui::{App, SharedString};
use rusqlite::params;

use crate::llm::chat_history::{AgentSessionRepository, MessageRepository, SessionRepository};
use crate::storage::connection::SqliteConnection;
use crate::storage::row_mapping::FromSqliteRow;
use crate::storage::traits::{Entity, Repository};
use crate::storage::{GlobalStorageState, now};

use super::types::{ProviderConfig, ProviderType};

struct ProviderConfigRow {
    id: i64,
    name: String,
    provider_type: String,
    api_key: Option<String>,
    api_base: Option<String>,
    api_version: Option<String>,
    model: String,
    models: Option<String>,
    max_tokens: Option<i32>,
    temperature: Option<f64>,
    enabled: i32,
    is_default: i32,
    created_at: i64,
    updated_at: i64,
}

impl FromSqliteRow for ProviderConfigRow {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(ProviderConfigRow {
            id: row.get("id")?,
            name: row.get("name")?,
            provider_type: row.get("provider_type")?,
            api_key: row.get("api_key")?,
            api_base: row.get("api_base")?,
            api_version: row.get("api_version")?,
            model: row.get("model")?,
            models: row.get("models")?,
            max_tokens: row.get("max_tokens")?,
            temperature: row.get("temperature")?,
            enabled: row.get("enabled")?,
            is_default: row.get("is_default")?,
            created_at: row.get("created_at")?,
            updated_at: row.get("updated_at")?,
        })
    }
}

impl TryFrom<ProviderConfigRow> for ProviderConfig {
    type Error = anyhow::Error;

    fn try_from(row: ProviderConfigRow) -> Result<Self> {
        let provider_type = ProviderType::from_str(&row.provider_type)
            .ok_or_else(|| anyhow::anyhow!("Invalid provider type: {}", row.provider_type))?;

        let models = match row.models.as_deref() {
            Some(json) => serde_json::from_str::<Vec<String>>(json).unwrap_or_default(),
            None => Vec::new(),
        };
        let mut models = models;
        if !row.model.is_empty() {
            if models.is_empty() {
                models.push(row.model.clone());
            } else if !models.iter().any(|m| m == &row.model) {
                models.insert(0, row.model.clone());
            }
        }

        Ok(ProviderConfig {
            id: row.id,
            name: row.name,
            provider_type,
            api_key: row.api_key,
            api_base: row.api_base,
            api_version: row.api_version,
            model: row.model,
            models,
            max_tokens: row.max_tokens,
            temperature: row.temperature.map(|t| t as f32),
            enabled: row.enabled != 0,
            is_default: row.is_default != 0,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }
}

#[derive(Clone)]
pub struct ProviderRepository {
    conn: SqliteConnection,
}

impl ProviderRepository {
    pub fn new(conn: SqliteConnection) -> Self {
        Self { conn }
    }
}

impl Entity for ProviderConfig {
    fn id(&self) -> Option<i64> {
        Some(self.id)
    }

    fn created_at(&self) -> i64 {
        self.created_at
    }

    fn updated_at(&self) -> i64 {
        self.updated_at
    }
}

impl Repository for ProviderRepository {
    type Entity = ProviderConfig;

    fn entity_type(&self) -> SharedString {
        SharedString::from("ProviderConfig")
    }

    fn insert(&self, item: &mut Self::Entity) -> Result<i64> {
        // `id == 0` 表示「尚未落库、等待分配行号」：必须绑定 NULL 让 SQLite
        // 自动分配 rowid，再把 `last_insert_rowid` 回写到 `item.id` 并返回。
        // 显式绑定 0 会占掉 rowid 0——第二条 id=0 的记录立刻撞
        // `UNIQUE constraint failed: llm_providers.id`。同步拉取路径
        // （personal/cloud 两个 apply_provider）对新设备上的每条提供商记录
        // 都以 0 调用这里，多提供商用户一拉取整个轮次就失败。
        // 非 0 的显式 id（如 UI 用时间戳生成的 id）保持原样写入。
        let assign_rowid = item.id == 0;
        let id = item.id;
        let name = item.name.clone();
        let provider_type = item.provider_type.as_str().to_string();
        let api_key = item.api_key.clone();
        let api_base = item.api_base.clone();
        let api_version = item.api_version.clone();
        let model = item.model.clone();
        let models = if item.models.is_empty() && !item.model.is_empty() {
            vec![item.model.clone()]
        } else {
            item.models.clone()
        };
        let models_json = serde_json::to_string(&models).unwrap_or_else(|_| "[]".to_string());
        let max_tokens = item.max_tokens;
        let temperature = item.temperature.map(|t| t as f64);
        let enabled = if item.enabled { 1i32 } else { 0i32 };
        let is_default = if item.is_default { 1i32 } else { 0i32 };
        let created_at = item.created_at;
        let updated_at = item.updated_at;

        self.conn.with_connection(|conn| {
            if assign_rowid {
                conn.execute(
                    "INSERT INTO llm_providers (name, provider_type, api_key, api_base, api_version, model, models, max_tokens, temperature, enabled, is_default, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                    params![name, provider_type, api_key, api_base, api_version, model, models_json, max_tokens, temperature, enabled, is_default, created_at, updated_at],
                )?;
                let rowid = conn.last_insert_rowid();
                item.id = rowid;
                Ok(rowid)
            } else {
                conn.execute(
                    "INSERT INTO llm_providers (id, name, provider_type, api_key, api_base, api_version, model, models, max_tokens, temperature, enabled, is_default, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
                    params![id, name, provider_type, api_key, api_base, api_version, model, models_json, max_tokens, temperature, enabled, is_default, created_at, updated_at],
                )?;
                Ok(id)
            }
        })
    }

    fn update(&self, item: &Self::Entity) -> Result<()> {
        let id = item.id;
        let name = item.name.clone();
        let provider_type = item.provider_type.as_str().to_string();
        let api_key = item.api_key.clone();
        let api_base = item.api_base.clone();
        let api_version = item.api_version.clone();
        let model = item.model.clone();
        let models = if item.models.is_empty() && !item.model.is_empty() {
            vec![item.model.clone()]
        } else {
            item.models.clone()
        };
        let models_json = serde_json::to_string(&models).unwrap_or_else(|_| "[]".to_string());
        let max_tokens = item.max_tokens;
        let temperature = item.temperature.map(|t| t as f64);
        let enabled = if item.enabled { 1i32 } else { 0i32 };
        let is_default = if item.is_default { 1i32 } else { 0i32 };
        let updated_at = now();

        self.conn.with_connection(|conn| {
            conn.execute(
                "UPDATE llm_providers SET name = ?1, provider_type = ?2, api_key = ?3, api_base = ?4, api_version = ?5, model = ?6, models = ?7, max_tokens = ?8, temperature = ?9, enabled = ?10, is_default = ?11, updated_at = ?12 WHERE id = ?13",
                params![name, provider_type, api_key, api_base, api_version, model, models_json, max_tokens, temperature, enabled, is_default, updated_at, id],
            )?;
            Ok(())
        })
    }

    fn delete(&self, id: i64) -> Result<()> {
        self.conn.with_connection(|conn| {
            conn.execute("DELETE FROM llm_providers WHERE id = ?1", params![id])?;
            Ok(())
        })
    }

    fn get(&self, id: i64) -> Result<Option<Self::Entity>> {
        self.conn.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, name, provider_type, api_key, api_base, api_version, model, models, max_tokens, temperature, enabled, is_default, created_at, updated_at FROM llm_providers WHERE id = ?1",
            )?;
            let mut rows = stmt.query(params![id])?;
            if let Some(row) = rows.next()? {
                let config_row = ProviderConfigRow::from_row(row)?;
                Ok(Some(config_row.try_into()?))
            } else {
                Ok(None)
            }
        })
    }

    fn list(&self) -> Result<Vec<Self::Entity>> {
        self.conn.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, name, provider_type, api_key, api_base, api_version, model, models, max_tokens, temperature, enabled, is_default, created_at, updated_at FROM llm_providers ORDER BY created_at DESC",
            )?;
            let rows = stmt.query_map([], |row| ProviderConfigRow::from_row(row))?;
            let mut results = Vec::new();
            for row in rows {
                results.push(row?.try_into()?);
            }
            Ok(results)
        })
    }

    fn count(&self) -> Result<i64> {
        self.conn.with_connection(|conn| {
            let count: i64 =
                conn.query_row("SELECT COUNT(*) FROM llm_providers", [], |row| row.get(0))?;
            Ok(count)
        })
    }

    fn exists(&self, id: i64) -> Result<bool> {
        self.conn.with_connection(|conn| {
            let exists: i64 = conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM llm_providers WHERE id = ?1)",
                params![id],
                |row| row.get(0),
            )?;
            Ok(exists == 1)
        })
    }
}

pub fn init(cx: &mut App) {
    let storage_state = cx.global::<GlobalStorageState>();
    let storage = storage_state.storage.clone();

    let conn = storage.connection();
    let provider_repo = ProviderRepository::new(conn.clone());
    let session_repo = SessionRepository::new(conn.clone());
    let message_repo = MessageRepository::new(conn.clone());
    let agent_session_repo = AgentSessionRepository::new(conn);

    storage.register(provider_repo);
    storage.register(session_repo);
    storage.register(message_repo);
    storage.register(agent_session_repo);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::migration::run_migrations;

    fn sample_provider(name: &str) -> ProviderConfig {
        ProviderConfig {
            name: name.to_string(),
            provider_type: ProviderType::OpenAI,
            model: "gpt-4o-mini".to_string(),
            models: vec!["gpt-4o-mini".to_string()],
            created_at: 1_700_000_000,
            updated_at: 1_700_000_000,
            ..ProviderConfig::default()
        }
    }

    fn open_provider_repo(temp: &tempfile::TempDir) -> ProviderRepository {
        let conn = SqliteConnection::open(temp.path().join("test.db")).expect("sqlite");
        conn.with_connection(|conn| run_migrations(conn))
            .expect("migrations run");
        ProviderRepository::new(conn)
    }

    #[test]
    fn provider_insert_assigns_rowid_when_id_is_zero() {
        // 回归：同步拉取路径（personal / cloud 的 apply_provider）对新设备上的
        // 每条 LLM 提供商记录都以 id=0 调 insert。显式绑定 0 会占掉 rowid 0，
        // 第二条记录立刻 UNIQUE constraint failed: llm_providers.id，整个拉取
        // 轮次失败（新设备拉取多个提供商时必现）。
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = open_provider_repo(&temp);

        let mut first = sample_provider("first");
        let first_id = repo.insert(&mut first).expect("first insert");
        assert!(first_id > 0, "insert 应返回自动分配的行号，实际 {first_id}");
        assert_eq!(first_id, first.id, "insert 应把行号回写到 item.id");

        let mut second = sample_provider("second");
        let second_id = repo.insert(&mut second).expect("second insert");
        assert!(second_id > 0);
        assert_ne!(first_id, second_id, "两条记录的行号必须不同");
        assert_eq!(second_id, second.id);

        let listed = repo.list().expect("list");
        assert_eq!(2, listed.len());
        assert!(listed
            .iter()
            .any(|provider| provider.name == "first" && provider.id == first_id));
        assert!(listed
            .iter()
            .any(|provider| provider.name == "second" && provider.id == second_id));
    }

    #[test]
    fn provider_insert_keeps_explicit_nonzero_id() {
        // UI 新增提供商用时间戳当 id，必须按显式值写入，不能被自动分配改写。
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = open_provider_repo(&temp);

        let mut config = sample_provider("explicit");
        config.id = 1_700_000_000;
        let id = repo.insert(&mut config).expect("insert");
        assert_eq!(1_700_000_000, id);
        assert_eq!(1_700_000_000, config.id);
        let loaded = repo
            .get(1_700_000_000)
            .expect("get")
            .expect("row with explicit id exists");
        assert_eq!("explicit", loaded.name);
    }

    #[test]
    fn provider_config_row_with_empty_model_keeps_models_untouched() {
        let row = ProviderConfigRow {
            id: 1,
            name: "my-openai".into(),
            provider_type: ProviderType::OpenAI.as_str().to_string(),
            api_key: None,
            api_base: None,
            api_version: None,
            model: String::new(),
            models: Some(serde_json::to_string(&["qwen-max", "deepseek-v3"]).unwrap()),
            max_tokens: None,
            temperature: None,
            enabled: 1,
            is_default: 0,
            created_at: 0,
            updated_at: 0,
        };
        let config = ProviderConfig::try_from(row).unwrap();
        assert!(config.model.is_empty());
        assert_eq!(config.models, vec!["qwen-max", "deepseek-v3"]);
    }

    #[test]
    fn provider_config_row_with_empty_model_and_no_models_stays_empty() {
        let row = ProviderConfigRow {
            id: 1,
            name: "my-openai".into(),
            provider_type: ProviderType::OpenAI.as_str().to_string(),
            api_key: None,
            api_base: None,
            api_version: None,
            model: String::new(),
            models: None,
            max_tokens: None,
            temperature: None,
            enabled: 1,
            is_default: 0,
            created_at: 0,
            updated_at: 0,
        };
        let config = ProviderConfig::try_from(row).unwrap();
        assert!(config.model.is_empty());
        assert!(config.models.is_empty());
    }
}
