use anyhow::{Result, anyhow, bail};
use rusqlite::OptionalExtension;

use crate::crypto;
use crate::storage::SshAccountExpect;
use crate::storage::connection::SqliteConnection;
use crate::storage::manager::now;
use crate::storage::traits::Repository;

use super::{CredentialEntry, CredentialSummary, DeleteCredentialOutcome};

#[derive(Clone)]
pub struct CredentialRepository {
    pub(crate) conn: SqliteConnection,
}

impl CredentialRepository {
    pub fn new(conn: SqliteConnection) -> Self {
        Self { conn }
    }

    pub fn get_plaintext(&self, id: i64) -> Result<Option<CredentialEntry>> {
        self.get(id)
    }

    pub fn get_by_cloud_id(&self, cloud_id: &str) -> Result<Option<CredentialEntry>> {
        self.conn.with_connection(|conn| {
            let sql = format!("{} WHERE cloud_id = ?1", Self::select_sql());
            conn.query_row(&sql, [cloud_id], Self::read_row)
                .optional()?
                .map(Self::decrypt_entry)
                .transpose()
        })
    }

    pub fn update_sync_status(
        &self,
        id: i64,
        cloud_id: Option<&str>,
        last_synced_at: Option<i64>,
    ) -> Result<()> {
        self.conn.with_connection(|conn| {
            let updated = conn.execute(
                "UPDATE credential_entries SET cloud_id = ?1, last_synced_at = ?2 WHERE id = ?3",
                rusqlite::params![cloud_id, last_synced_at, id],
            )?;
            anyhow::ensure!(updated == 1, "Credential {id} not found");
            Ok(())
        })
    }

    pub fn update_sync_status_with_updated_at(
        &self,
        id: i64,
        cloud_id: Option<&str>,
        last_synced_at: Option<i64>,
        updated_at: i64,
    ) -> Result<()> {
        self.conn.with_connection(|conn| {
            let updated = conn.execute(
                "UPDATE credential_entries
                 SET cloud_id = ?1, last_synced_at = ?2, updated_at = ?3
                 WHERE id = ?4",
                rusqlite::params![cloud_id, last_synced_at, updated_at, id],
            )?;
            anyhow::ensure!(updated == 1, "Credential {id} not found");
            Ok(())
        })
    }

    pub fn get_summary(&self, id: i64) -> Result<Option<CredentialSummary>> {
        self.conn.with_connection(|conn| {
            let sql = format!("{} WHERE id = ?1", Self::summary_select_sql());
            Ok(conn
                .query_row(&sql, [id], Self::read_summary_row)
                .optional()?)
        })
    }

    pub fn list_summaries(&self) -> Result<Vec<CredentialSummary>> {
        self.conn.with_connection(|conn| {
            let sql = format!(
                "{} ORDER BY updated_at DESC, id DESC",
                Self::summary_select_sql()
            );
            let mut statement = conn.prepare(&sql)?;
            let rows = statement.query_map([], Self::read_summary_row)?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })
    }

    fn select_sql() -> &'static str {
        "SELECT id, name, kind, username, password, private_key_path,
                private_key_content, passphrase, ssh_expect, sync_enabled, cloud_id,
                last_synced_at, team_id, owner_id, created_at, updated_at
         FROM credential_entries"
    }

    fn summary_select_sql() -> &'static str {
        "SELECT id, name, kind, username,
                password IS NOT NULL AND password != '' AS has_password,
                private_key_path IS NOT NULL AND private_key_path != ''
                    AS has_private_key_path,
                private_key_content IS NOT NULL AND private_key_content != ''
                    AS has_private_key_content,
                passphrase IS NOT NULL AND passphrase != '' AS has_passphrase,
                ssh_expect IS NOT NULL AND ssh_expect != '' AS has_ssh_expect,
                sync_enabled, cloud_id, last_synced_at, team_id, owner_id,
                created_at, updated_at
         FROM credential_entries"
    }

    fn read_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<CredentialEntry> {
        Ok(CredentialEntry {
            id: row.get("id")?,
            name: row.get("name")?,
            username: row.get("username")?,
            password: row.get("password")?,
            private_key_path: row.get("private_key_path")?,
            private_key_content: row.get("private_key_content")?,
            passphrase: row.get("passphrase")?,
            ssh_expect: decrypt_ssh_expect(row.get("ssh_expect")?).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    Box::new(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        error.to_string(),
                    )),
                )
            })?,
            sync_enabled: row.get::<_, i64>("sync_enabled").unwrap_or_default() != 0,
            cloud_id: row.get("cloud_id")?,
            last_synced_at: row.get("last_synced_at")?,
            team_id: row.get("team_id")?,
            owner_id: row.get("owner_id")?,
            created_at: row.get("created_at")?,
            updated_at: row.get("updated_at")?,
        })
    }

    fn read_summary_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<CredentialSummary> {
        Ok(CredentialSummary {
            id: row.get("id")?,
            name: row.get("name")?,
            kind: row.get("kind")?,
            username: row.get("username")?,
            has_password: row.get::<_, i64>("has_password")? != 0,
            has_private_key_path: row.get::<_, i64>("has_private_key_path")? != 0,
            has_private_key_content: row.get::<_, i64>("has_private_key_content")? != 0,
            has_passphrase: row.get::<_, i64>("has_passphrase")? != 0,
            has_ssh_expect: row.get::<_, i64>("has_ssh_expect")? != 0,
            sync_enabled: row.get::<_, i64>("sync_enabled")? != 0,
            cloud_id: row.get("cloud_id")?,
            last_synced_at: row.get("last_synced_at")?,
            team_id: row.get("team_id")?,
            owner_id: row.get("owner_id")?,
            created_at: row.get("created_at")?,
            updated_at: row.get("updated_at")?,
        })
    }

    fn decrypt_entry(mut entry: CredentialEntry) -> Result<CredentialEntry> {
        entry.password = decrypt_secret(entry.password)?;
        entry.private_key_content = decrypt_secret(entry.private_key_content)?;
        entry.passphrase = decrypt_secret(entry.passphrase)?;
        Ok(entry)
    }

    fn encrypted_values(
        item: &CredentialEntry,
    ) -> Result<(
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    )> {
        if item.has_secrets() && !crypto::has_master_key() {
            bail!("Cannot persist credential secrets without a master key");
        }
        Ok((
            encrypt_secret(item.password.as_deref())?,
            encrypt_secret(item.private_key_content.as_deref())?,
            encrypt_secret(item.passphrase.as_deref())?,
            encrypt_ssh_expect(&item.ssh_expect)?,
        ))
    }
}

fn encrypt_ssh_expect(value: &SshAccountExpect) -> Result<Option<String>> {
    if value.is_empty() {
        return Ok(None);
    }
    let serialized = serde_json::to_string(value)?;
    encrypt_secret(Some(&serialized))
}

fn decrypt_ssh_expect(value: Option<String>) -> Result<SshAccountExpect> {
    let Some(serialized) = decrypt_secret(value)? else {
        return Ok(SshAccountExpect::default());
    };
    serde_json::from_str(&serialized)
        .map_err(|error| anyhow!("Credential automatic login data is invalid: {error}"))
}

fn encrypt_secret(value: Option<&str>) -> Result<Option<String>> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_empty() {
        return Ok(Some(String::new()));
    }
    if !crypto::has_master_key() {
        bail!("Cannot persist credential secrets without a master key");
    }
    let encrypted = crypto::encrypt_password(value);
    if encrypted == value || !encrypted.starts_with("ENC:") {
        bail!("Credential secret encryption failed");
    }
    Ok(Some(encrypted))
}

fn decrypt_secret(value: Option<String>) -> Result<Option<String>> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_empty() {
        return Ok(Some(String::new()));
    }
    if !crypto::has_master_key() || !value.starts_with("ENC:") {
        bail!("Credential secret cannot be decrypted safely");
    }
    let decrypted = crypto::decrypt_password(&value);
    if decrypted.is_empty() {
        bail!("Credential secret decryption failed");
    }
    Ok(Some(decrypted))
}

impl Repository for CredentialRepository {
    type Entity = CredentialEntry;

    fn entity_type(&self) -> gpui::SharedString {
        "CredentialEntry".into()
    }

    fn insert(&self, item: &mut Self::Entity) -> Result<i64> {
        let (password, key, passphrase, ssh_expect) = Self::encrypted_values(item)?;
        let ts = now();
        let id = self.conn.with_connection(|conn| {
            conn.execute(
                "INSERT INTO credential_entries
                 (name, kind, username, password, private_key_path, private_key_content,
                  passphrase, ssh_expect, sync_enabled, cloud_id, last_synced_at, team_id,
                  owner_id, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
                rusqlite::params![
                    item.name,
                    item.username,
                    password,
                    item.private_key_path,
                    key,
                    passphrase,
                    ssh_expect,
                    item.sync_enabled,
                    item.cloud_id,
                    item.last_synced_at,
                    item.team_id,
                    item.owner_id,
                    ts,
                    ts
                ],
            )?;
            Ok(conn.last_insert_rowid())
        })?;
        item.id = Some(id);
        item.created_at = Some(ts);
        item.updated_at = Some(ts);
        Ok(id)
    }

    fn update(&self, item: &Self::Entity) -> Result<()> {
        let id = item.id.ok_or_else(|| anyhow!("Cannot update without ID"))?;
        let (password, key, passphrase, ssh_expect) = Self::encrypted_values(item)?;
        let ts = now();
        self.conn.with_connection(|conn| {
            let updated = conn.execute(
                "UPDATE credential_entries
                 SET name = ?1, kind = ?2, username = ?3, password = ?4,
                     private_key_path = ?5, private_key_content = ?6, passphrase = ?7,
                     ssh_expect = ?8, sync_enabled = ?9, cloud_id = ?10,
                     last_synced_at = ?11, team_id = ?12, owner_id = ?13,
                     updated_at = ?14 WHERE id = ?15",
                rusqlite::params![
                    item.name,
                    item.username,
                    password,
                    item.private_key_path,
                    key,
                    passphrase,
                    ssh_expect,
                    item.sync_enabled,
                    item.cloud_id,
                    item.last_synced_at,
                    item.team_id,
                    item.owner_id,
                    ts,
                    id
                ],
            )?;
            anyhow::ensure!(updated == 1, "Credential {id} not found");
            Ok(())
        })
    }

    fn delete(&self, id: i64) -> Result<()> {
        match self.delete_checked(id)? {
            DeleteCredentialOutcome::Deleted | DeleteCredentialOutcome::NotFound => Ok(()),
            DeleteCredentialOutcome::Referenced(references) => bail!(
                "Credential {id} is still referenced by {} connection location(s)",
                references.len()
            ),
        }
    }

    fn get(&self, id: i64) -> Result<Option<Self::Entity>> {
        self.conn.with_connection(|conn| {
            let sql = format!("{} WHERE id = ?1", Self::select_sql());
            conn.query_row(&sql, [id], Self::read_row)
                .optional()?
                .map(Self::decrypt_entry)
                .transpose()
        })
    }

    fn list(&self) -> Result<Vec<Self::Entity>> {
        self.conn.with_connection(|conn| {
            let sql = format!("{} ORDER BY updated_at DESC, id DESC", Self::select_sql());
            let mut statement = conn.prepare(&sql)?;
            let rows = statement.query_map([], Self::read_row)?;
            rows.map(|row| row.map_err(Into::into).and_then(Self::decrypt_entry))
                .collect()
        })
    }

    fn count(&self) -> Result<i64> {
        self.conn.with_connection(|conn| {
            Ok(
                conn.query_row("SELECT COUNT(*) FROM credential_entries", [], |row| {
                    row.get(0)
                })?,
            )
        })
    }

    fn exists(&self, id: i64) -> Result<bool> {
        self.conn.with_connection(|conn| {
            Ok(conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM credential_entries WHERE id = ?1)",
                [id],
                |row| row.get::<_, i64>(0),
            )? == 1)
        })
    }
}
