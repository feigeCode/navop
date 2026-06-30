use connection_importer::{
    ImportSourceKind, ImportedConnection, ImportedSshAuthMethod, ImportedSshConnection,
    to_db_connection_config, to_ssh_params,
};
use one_core::storage::StoredConnection;

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
    Database(ImportedConnection),
    Ssh(ImportedSshConnection),
}

impl EditableImportDraft {
    pub(crate) fn database(imported: ImportedConnection) -> Self {
        Self {
            selected: true,
            name: imported.name.clone(),
            host: imported.host.clone(),
            port: imported
                .port
                .map(|port| port.to_string())
                .unwrap_or_default(),
            username: imported.username.clone(),
            password: imported.password.clone().unwrap_or_default(),
            database: imported.database.clone().unwrap_or_default(),
            private_key_path: String::new(),
            payload: ImportDraftPayload::Database(imported),
        }
    }

    pub(crate) fn ssh(imported: ImportedSshConnection) -> Self {
        let (password, private_key_path) = ssh_auth_edit_values(&imported.auth_method);
        Self {
            selected: true,
            name: imported.name.clone(),
            host: imported.host.clone(),
            port: imported.port.to_string(),
            username: imported.username.clone(),
            password,
            database: String::new(),
            private_key_path,
            payload: ImportDraftPayload::Ssh(imported),
        }
    }

    pub(crate) fn kind(&self) -> ImportDraftKind {
        match self.payload {
            ImportDraftPayload::Database(_) => ImportDraftKind::Database,
            ImportDraftPayload::Ssh(_) => ImportDraftKind::Ssh,
        }
    }

    pub(crate) fn source_name(&self) -> &'static str {
        match &self.payload {
            ImportDraftPayload::Database(imported) => imported.source.display_name(),
            ImportDraftPayload::Ssh(imported) => imported.source.display_name(),
        }
    }

    pub(crate) fn source_kind(&self) -> ImportSourceKind {
        match &self.payload {
            ImportDraftPayload::Database(imported) => imported.source,
            ImportDraftPayload::Ssh(imported) => imported.source,
        }
    }

    pub(crate) fn source_id(&self) -> &str {
        match &self.payload {
            ImportDraftPayload::Database(imported) => &imported.source_id,
            ImportDraftPayload::Ssh(imported) => &imported.source_id,
        }
    }

    pub(crate) fn supports_database_edit(&self) -> bool {
        matches!(self.payload, ImportDraftPayload::Database(_))
    }

    pub(crate) fn supports_password_edit(&self) -> bool {
        match &self.payload {
            ImportDraftPayload::Database(_) => true,
            ImportDraftPayload::Ssh(imported) => {
                matches!(imported.auth_method, ImportedSshAuthMethod::Password { .. })
            }
        }
    }

    pub(crate) fn supports_private_key_edit(&self) -> bool {
        match &self.payload {
            ImportDraftPayload::Database(_) => false,
            ImportDraftPayload::Ssh(imported) => {
                matches!(
                    imported.auth_method,
                    ImportedSshAuthMethod::PrivateKey { .. }
                )
            }
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
            ImportDraftPayload::Database(imported) => self.to_database_connection(imported.clone()),
            ImportDraftPayload::Ssh(imported) => self.to_ssh_connection(imported.clone()),
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

    fn to_database_connection(
        &self,
        mut imported: ImportedConnection,
    ) -> Result<StoredConnection, String> {
        imported.name = required_text(&self.name, "连接名称")?;
        imported.host = self.host.trim().to_string();
        imported.port = optional_port(&self.port)?;
        imported.username = self.username.trim().to_string();
        imported.password = optional_text(&self.password);
        imported.database = optional_text(&self.database);
        let config = to_db_connection_config(imported).map_err(|error| error.to_string())?;
        Ok(StoredConnection::from_db_connection(config))
    }

    fn to_ssh_connection(
        &self,
        mut imported: ImportedSshConnection,
    ) -> Result<StoredConnection, String> {
        imported.name = required_text(&self.name, "连接名称")?;
        imported.host = required_text(&self.host, "主机")?;
        imported.port = required_port(&self.port)?;
        imported.username = self.username.trim().to_string();
        imported.auth_method = self.edited_ssh_auth_method(imported.auth_method);
        let name = imported.name.clone();
        let params = to_ssh_params(imported).map_err(|error| error.to_string())?;
        Ok(StoredConnection::new_ssh(name, params, None))
    }

    fn edited_ssh_auth_method(&self, auth_method: ImportedSshAuthMethod) -> ImportedSshAuthMethod {
        match auth_method {
            ImportedSshAuthMethod::Password { .. } => ImportedSshAuthMethod::Password {
                password: optional_text(&self.password),
            },
            ImportedSshAuthMethod::PrivateKey { passphrase, .. } => {
                ImportedSshAuthMethod::PrivateKey {
                    key_path: self.private_key_path.trim().to_string(),
                    passphrase,
                }
            }
            ImportedSshAuthMethod::Agent => ImportedSshAuthMethod::Agent,
            ImportedSshAuthMethod::AutoPublicKey => ImportedSshAuthMethod::AutoPublicKey,
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

fn ssh_auth_edit_values(auth_method: &ImportedSshAuthMethod) -> (String, String) {
    match auth_method {
        ImportedSshAuthMethod::Password { password } => {
            (password.clone().unwrap_or_default(), String::new())
        }
        ImportedSshAuthMethod::PrivateKey { key_path, .. } => (String::new(), key_path.clone()),
        ImportedSshAuthMethod::Agent | ImportedSshAuthMethod::AutoPublicKey => {
            (String::new(), String::new())
        }
    }
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
