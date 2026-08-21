use connection_import_protocol::{ImportRecord, ImportRecordKind, PasswordImportStatus};
use gpui_component::{Icon, IconName, IconSize, Sizable};
use one_core::storage::{ConnectionType, QuickCommand, StoredConnection};
use rust_i18n::t;

use super::connection_import_draft_conversion::{
    import_draft_duplicate_identity, import_draft_to_editor_connection,
    import_draft_to_quick_command, import_draft_to_stored_connection, ssh_auth_edit_values,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ImportDraftKind {
    Database,
    Ssh,
    QuickCommand,
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
    pub(crate) ssh_group_path: String,
    pub(crate) password: String,
    pub(crate) database: String,
    pub(crate) private_key_path: String,
    pub(crate) quick_command_group_name: String,
    pub(crate) quick_command_command: String,
    pub(crate) quick_command_shortcut: String,
    pub(crate) quick_command_description: String,
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
            ImportRecordKind::QuickCommand => Self::quick_command(record),
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
            ssh_group_path: String::new(),
            password: imported
                .and_then(|record| record.password.clone())
                .unwrap_or_default(),
            database: imported
                .and_then(|record| record.database.clone())
                .unwrap_or_default(),
            private_key_path: String::new(),
            quick_command_group_name: String::new(),
            quick_command_command: String::new(),
            quick_command_shortcut: String::new(),
            quick_command_description: String::new(),
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
            ssh_group_path: imported
                .and_then(|record| record.group_path.clone())
                .unwrap_or_default(),
            password,
            database: String::new(),
            private_key_path,
            quick_command_group_name: String::new(),
            quick_command_command: String::new(),
            quick_command_shortcut: String::new(),
            quick_command_description: String::new(),
            payload: ImportDraftPayload::Record(record),
        }
    }

    fn quick_command(record: ImportRecord) -> Self {
        let imported = record.quick_command.as_ref();
        Self {
            selected: true,
            name: imported
                .map(|record| record.name.clone())
                .unwrap_or_else(|| record.display_name.clone()),
            host: String::new(),
            port: String::new(),
            username: String::new(),
            ssh_group_path: String::new(),
            password: String::new(),
            database: String::new(),
            private_key_path: String::new(),
            quick_command_group_name: imported
                .and_then(|record| record.group_name.clone())
                .unwrap_or_default(),
            quick_command_command: imported
                .map(|record| record.command.clone())
                .unwrap_or_default(),
            quick_command_shortcut: imported
                .and_then(|record| record.shortcut.clone())
                .unwrap_or_default(),
            quick_command_description: imported
                .and_then(|record| record.description.clone())
                .unwrap_or_default(),
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
            ssh_group_path: String::new(),
            password: String::new(),
            database: String::new(),
            private_key_path: String::new(),
            quick_command_group_name: String::new(),
            quick_command_command: String::new(),
            quick_command_shortcut: String::new(),
            quick_command_description: String::new(),
            payload: ImportDraftPayload::Record(record),
        }
    }

    pub(crate) fn kind(&self) -> ImportDraftKind {
        match &self.payload {
            ImportDraftPayload::Record(record) => match record.kind {
                ImportRecordKind::Database => ImportDraftKind::Database,
                ImportRecordKind::Ssh => ImportDraftKind::Ssh,
                ImportRecordKind::QuickCommand => ImportDraftKind::QuickCommand,
                ImportRecordKind::PortForwarding => ImportDraftKind::Unsupported,
            },
        }
    }

    pub(crate) fn visual_connection_type(&self) -> ConnectionType {
        match &self.payload {
            ImportDraftPayload::Record(record) => match record.kind {
                ImportRecordKind::Database => ConnectionType::Database,
                ImportRecordKind::Ssh => ConnectionType::SshSftp,
                ImportRecordKind::QuickCommand => ConnectionType::SshSftp,
                ImportRecordKind::PortForwarding => ConnectionType::PortForwarding,
            },
        }
    }

    pub(crate) fn icon(&self) -> Icon {
        let name = match &self.payload {
            ImportDraftPayload::Record(record) => match record.kind {
                ImportRecordKind::QuickCommand => IconName::TerminalQuickCommandColor,
                _ => {
                    return crate::connection_visuals::connection_type_icon(
                        self.visual_connection_type(),
                        crate::connection_visuals::ConnectionVisualSize::Tree,
                    );
                }
            },
        };
        name.color().with_size(IconSize::Default)
    }

    pub(crate) fn source_name(&self) -> &str {
        match &self.payload {
            ImportDraftPayload::Record(record) => &record.source_label,
        }
    }

    pub(crate) fn password_status_text(&self) -> String {
        match &self.payload {
            ImportDraftPayload::Record(record) => match record.password_status {
                PasswordImportStatus::Included => {
                    t!("Home.ConnectionImport.password_included").to_string()
                }
                PasswordImportStatus::Missing => {
                    t!("Home.ConnectionImport.password_missing").to_string()
                }
                PasswordImportStatus::Unsupported => {
                    t!("Home.ConnectionImport.password_unsupported").to_string()
                }
                PasswordImportStatus::PermissionDenied => {
                    t!("Home.ConnectionImport.password_permission_denied").to_string()
                }
            },
        }
    }

    pub(crate) fn quick_command_detail_text(&self) -> String {
        let mut parts = Vec::new();
        if let Some(group) = optional_text(&self.quick_command_group_name) {
            parts.push(group);
        }
        if let Some(shortcut) = optional_text(&self.quick_command_shortcut) {
            parts.push(shortcut);
        }
        if let Some(description) = optional_text(&self.quick_command_description) {
            parts.push(description);
        }
        parts.push(self.quick_command_command.clone());
        parts.join(" · ")
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

    pub(crate) fn to_editor_connection(&self) -> Result<StoredConnection, String> {
        match &self.payload {
            ImportDraftPayload::Record(record) => import_draft_to_editor_connection(self, record),
        }
    }

    pub(crate) fn to_quick_command(&self) -> Result<QuickCommand, String> {
        match &self.payload {
            ImportDraftPayload::Record(record) => import_draft_to_quick_command(self, record),
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

fn optional_text(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
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
