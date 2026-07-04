use connection_import_protocol::{ImportRecord, ImportRecordKind, PasswordImportStatus};
use one_core::storage::StoredConnection;

use super::connection_import_draft_conversion::{
    import_draft_duplicate_identity, import_draft_to_stored_connection, ssh_auth_edit_values,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ImportDraftKind {
    Database,
    Ssh,
    Unsupported,
}

#[cfg(test)]
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

#[cfg(test)]
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
            ImportRecordKind::PortForwarding => Self::unsupported(record),
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

    fn unsupported(record: ImportRecord) -> Self {
        Self {
            selected: true,
            name: record.display_name.clone(),
            host: String::new(),
            port: String::new(),
            username: String::new(),
            password: String::new(),
            database: String::new(),
            private_key_path: String::new(),
            payload: ImportDraftPayload::Record(record),
        }
    }

    pub(crate) fn kind(&self) -> ImportDraftKind {
        match &self.payload {
            ImportDraftPayload::Record(record) => match record.kind {
                ImportRecordKind::Database => ImportDraftKind::Database,
                ImportRecordKind::Ssh => ImportDraftKind::Ssh,
                ImportRecordKind::PortForwarding => ImportDraftKind::Unsupported,
            },
        }
    }

    pub(crate) fn source_name(&self) -> &str {
        match &self.payload {
            ImportDraftPayload::Record(record) => &record.source_label,
        }
    }

    pub(crate) fn password_status_text(&self) -> &'static str {
        match &self.payload {
            ImportDraftPayload::Record(record) => match record.password_status {
                PasswordImportStatus::Included => "密码已导入",
                PasswordImportStatus::Missing => "密码缺失",
                PasswordImportStatus::Unsupported => "密码不支持导入",
                PasswordImportStatus::PermissionDenied => "密码导入被拒绝",
            },
        }
    }

    pub(crate) fn warning_text(&self) -> Option<String> {
        match &self.payload {
            ImportDraftPayload::Record(record) if record.warnings.is_empty() => None,
            ImportDraftPayload::Record(record) => Some(
                record
                    .warnings
                    .iter()
                    .map(|warning| warning.message.as_str())
                    .collect::<Vec<_>>()
                    .join("；"),
            ),
        }
    }

    pub(crate) fn source_id(&self) -> &str {
        match &self.payload {
            ImportDraftPayload::Record(record) => &record.id,
        }
    }

    #[cfg(test)]
    pub(crate) fn apply_edit(&mut self, edit: ImportDraftEdit) -> Result<(), String> {
        match edit {
            ImportDraftEdit::Selected(selected) => self.selected = selected,
            ImportDraftEdit::Text { field, value } => self.set_text(field, value),
        }
        Ok(())
    }

    pub(crate) fn to_stored_connection(&self) -> Result<StoredConnection, String> {
        match &self.payload {
            ImportDraftPayload::Record(record) => import_draft_to_stored_connection(self, record),
        }
    }

    pub(crate) fn duplicate_identity(&self) -> Result<String, String> {
        match &self.payload {
            ImportDraftPayload::Record(record) => import_draft_duplicate_identity(self, record),
        }
    }

    #[cfg(test)]
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
}
#[cfg(test)]
pub(crate) fn selected_import_count(drafts: &[EditableImportDraft]) -> usize {
    drafts.iter().filter(|draft| draft.selected).count()
}

#[cfg(test)]
pub(crate) fn selected_import_drafts_to_connections(
    drafts: &[EditableImportDraft],
) -> Result<Vec<StoredConnection>, String> {
    drafts
        .iter()
        .filter(|draft| draft.selected)
        .map(EditableImportDraft::to_stored_connection)
        .collect()
}
