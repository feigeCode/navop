use std::collections::HashMap;

use connection_import_protocol::{
    DatabaseImportRecord, ImportDatabaseType, ImportRecord, ImportRecordKind, SshImportAuthMethod,
};
use one_core::storage::{
    DatabaseType, DbConnectionConfig, SshAuthMethod, SshParams, StoredConnection,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ImportDraftKind {
    Database,
    Ssh,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ImportDraftField {
    Name,
    Host,
    Port,
    Username,
    Password,
    Database,
    PrivateKeyPath,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ImportDraftEdit {
    Selected(bool),
    Text {
        field: ImportDraftField,
        value: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EditableImportDraft {
    pub(crate) selected: bool,
    pub(crate) name: String,
    pub(crate) host: String,
    pub(crate) port: String,
    pub(crate) username: String,
    pub(crate) password: String,
    pub(crate) database: String,
    pub(crate) private_key_path: String,
    payload: ImportDraftPayload,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ImportDraftPayload {
    Record(ImportRecord),
}

impl EditableImportDraft {
    pub(crate) fn new(record: ImportRecord) -> Self {
        match record.kind {
            ImportRecordKind::Database => Self::database(record),
            ImportRecordKind::Ssh => Self::ssh(record),
        }
    }

    fn database(record: ImportRecord) -> Self {
        let imported = record.database.as_ref();
        Self {
            selected: true,
            name: imported
                .map(|record| record.name.clone())
                .unwrap_or_default(),
            host: imported
                .map(|record| record.host.clone())
                .unwrap_or_default(),
            port: imported
                .and_then(|record| record.port)
                .map(|port| port.to_string())
                .unwrap_or_default(),
            username: imported
                .map(|record| record.username.clone())
                .unwrap_or_default(),
            password: imported
                .and_then(|record| record.password.clone())
                .unwrap_or_default(),
            database: imported
                .and_then(|record| record.database.clone())
                .unwrap_or_default(),
            private_key_path: String::new(),
            payload: ImportDraftPayload::Record(record),
        }
    }

    fn ssh(record: ImportRecord) -> Self {
        let imported = record.ssh.as_ref();
        let (password, private_key_path) = imported
            .map(|record| ssh_auth_edit_values(&record.auth_method))
            .unwrap_or_default();
        Self {
            selected: true,
            name: imported
                .map(|record| record.name.clone())
                .unwrap_or_default(),
            host: imported
                .map(|record| record.host.clone())
                .unwrap_or_default(),
            port: imported
                .and_then(|record| record.port)
                .map(|port| port.to_string())
                .unwrap_or_default(),
            username: imported
                .map(|record| record.username.clone())
                .unwrap_or_default(),
            password,
            database: String::new(),
            private_key_path,
            payload: ImportDraftPayload::Record(record),
        }
    }

    pub(crate) fn kind(&self) -> ImportDraftKind {
        match &self.payload {
            ImportDraftPayload::Record(record) => match record.kind {
                ImportRecordKind::Database => ImportDraftKind::Database,
                ImportRecordKind::Ssh => ImportDraftKind::Ssh,
            },
        }
    }

    pub(crate) fn source_name(&self) -> &str {
        match &self.payload {
            ImportDraftPayload::Record(record) => &record.source_label,
        }
    }

    pub(crate) fn source_icon_hint(&self) -> &str {
        match &self.payload {
            ImportDraftPayload::Record(record) => &record.importer_id,
        }
    }

    pub(crate) fn source_id(&self) -> &str {
        match &self.payload {
            ImportDraftPayload::Record(record) => &record.id,
        }
    }

    pub(crate) fn supports_database_edit(&self) -> bool {
        self.kind() == ImportDraftKind::Database
    }

    pub(crate) fn supports_password_edit(&self) -> bool {
        match &self.payload {
            ImportDraftPayload::Record(record) if record.kind == ImportRecordKind::Database => true,
            ImportDraftPayload::Record(record) => record
                .ssh
                .as_ref()
                .map(|ssh| matches!(ssh.auth_method, SshImportAuthMethod::Password { .. }))
                .unwrap_or(false),
        }
    }

    pub(crate) fn supports_private_key_edit(&self) -> bool {
        match &self.payload {
            ImportDraftPayload::Record(record) => record
                .ssh
                .as_ref()
                .map(|ssh| matches!(ssh.auth_method, SshImportAuthMethod::PrivateKey { .. }))
                .unwrap_or(false),
        }
    }

    pub(crate) fn apply_edit(&mut self, edit: ImportDraftEdit) -> Result<(), String> {
        match edit {
            ImportDraftEdit::Selected(selected) => self.selected = selected,
            ImportDraftEdit::Text { field, value } => self.set_text(field, value),
        }
        Ok(())
    }

    pub(crate) fn to_stored_connection(&self) -> Result<StoredConnection, String> {
        match &self.payload {
            ImportDraftPayload::Record(record) => match record.kind {
                ImportRecordKind::Database => self.to_database_connection(record),
                ImportRecordKind::Ssh => self.to_ssh_connection(record),
            },
        }
    }

    fn set_text(&mut self, field: ImportDraftField, value: String) {
        match field {
            ImportDraftField::Name => self.name = value,
            ImportDraftField::Host => self.host = value,
            ImportDraftField::Port => self.port = value,
            ImportDraftField::Username => self.username = value,
            ImportDraftField::Password => self.password = value,
            ImportDraftField::Database => self.database = value,
            ImportDraftField::PrivateKeyPath => self.private_key_path = value,
        }
    }

    fn to_database_connection(&self, record: &ImportRecord) -> Result<StoredConnection, String> {
        let imported = record
            .database
            .as_ref()
            .ok_or_else(|| "数据库导入记录缺少数据库配置".to_string())?;
        let name = required_text(&self.name, "连接名称")?;
        let database_type = database_type(&imported.database_type);
        let port = optional_port(&self.port)?
            .or_else(|| default_database_port(&database_type))
            .unwrap_or_default();
        let config = DbConnectionConfig {
            id: String::new(),
            database_type,
            name: name.clone(),
            host: required_text(&self.host, "主机")?,
            port,
            username: self.username.trim().to_string(),
            password: optional_text(&self.password).unwrap_or_default(),
            database: optional_text(&self.database),
            service_name: None,
            sid: None,
            workspace_id: None,
            extra_params: extra_params(imported),
        };
        Ok(StoredConnection::from_db_connection(config))
    }

    fn to_ssh_connection(&self, record: &ImportRecord) -> Result<StoredConnection, String> {
        let imported = record
            .ssh
            .as_ref()
            .ok_or_else(|| "SSH 导入记录缺少 SSH 配置".to_string())?;
        let name = required_text(&self.name, "连接名称")?;
        let params = SshParams {
            host: required_text(&self.host, "主机")?,
            port: required_port(&self.port)?,
            username: self.username.trim().to_string(),
            auth_method: self.edited_ssh_auth_method(&imported.auth_method),
            connect_timeout: None,
            keepalive_interval: None,
            keepalive_max: None,
            default_directory: None,
            init_script: None,
            disable_shell_integration: None,
            jump_server: None,
            proxy: None,
        };
        Ok(StoredConnection::new_ssh(name, params, None))
    }

    fn edited_ssh_auth_method(&self, auth_method: &SshImportAuthMethod) -> SshAuthMethod {
        match auth_method {
            SshImportAuthMethod::Password { .. } => SshAuthMethod::Password {
                password: optional_text(&self.password).unwrap_or_default(),
            },
            SshImportAuthMethod::PrivateKey { passphrase, .. } => SshAuthMethod::PrivateKey {
                key_path: self.private_key_path.trim().to_string(),
                passphrase: passphrase.clone(),
            },
            SshImportAuthMethod::Agent => SshAuthMethod::Agent,
            SshImportAuthMethod::AutoPublicKey => SshAuthMethod::AutoPublicKey,
        }
    }
}

fn database_type(database_type: &ImportDatabaseType) -> DatabaseType {
    match database_type {
        ImportDatabaseType::MySql => DatabaseType::MySQL,
        ImportDatabaseType::PostgreSql => DatabaseType::PostgreSQL,
        ImportDatabaseType::Sqlite => DatabaseType::SQLite,
        ImportDatabaseType::DuckDb => DatabaseType::DuckDB,
        ImportDatabaseType::SqlServer => DatabaseType::MSSQL,
        ImportDatabaseType::Oracle => DatabaseType::Oracle,
        ImportDatabaseType::ClickHouse => DatabaseType::ClickHouse,
        ImportDatabaseType::External { id } => DatabaseType::External {
            driver_id: id.clone(),
        },
    }
}

fn default_database_port(database_type: &DatabaseType) -> Option<u16> {
    match database_type {
        DatabaseType::MySQL => Some(3306),
        DatabaseType::PostgreSQL => Some(5432),
        DatabaseType::MSSQL => Some(1433),
        DatabaseType::Oracle => Some(1521),
        DatabaseType::ClickHouse => Some(8123),
        DatabaseType::SQLite | DatabaseType::DuckDB | DatabaseType::External { .. } => None,
    }
}

fn extra_params(imported: &DatabaseImportRecord) -> HashMap<String, String> {
    imported
        .extra_params
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

fn ssh_auth_edit_values(auth_method: &SshImportAuthMethod) -> (String, String) {
    match auth_method {
        SshImportAuthMethod::Password { password } => {
            (password.clone().unwrap_or_default(), String::new())
        }
        SshImportAuthMethod::PrivateKey { key_path, .. } => (String::new(), key_path.clone()),
        SshImportAuthMethod::Agent | SshImportAuthMethod::AutoPublicKey => {
            (String::new(), String::new())
        }
    }
}
pub(crate) fn selected_import_count(drafts: &[EditableImportDraft]) -> usize {
    drafts.iter().filter(|draft| draft.selected).count()
}

pub(crate) fn selected_import_drafts_to_connections(
    drafts: &[EditableImportDraft],
) -> Result<Vec<StoredConnection>, String> {
    drafts
        .iter()
        .filter(|draft| draft.selected)
        .map(EditableImportDraft::to_stored_connection)
        .collect()
}

fn required_text(value: &str, label: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(format!("{}不能为空", label));
    }
    Ok(trimmed.to_string())
}

fn optional_text(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn optional_port(value: &str) -> Result<Option<u16>, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    trimmed
        .parse::<u16>()
        .map(Some)
        .map_err(|_| "端口必须是 1-65535".to_string())
}

fn required_port(value: &str) -> Result<u16, String> {
    optional_port(value)?.ok_or_else(|| "端口不能为空".to_string())
}
