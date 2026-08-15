use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize, Serializer, ser::SerializeStruct};

/// A locally stored credential entry.
///
/// Secret values are kept decrypted in memory only. The repository encrypts
/// them before writing them to SQLite and decrypts them on reads.
#[derive(Clone, PartialEq, Eq, Deserialize)]
pub struct CredentialEntry {
    pub id: Option<i64>,
    pub name: String,
    pub kind: String,
    pub username: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default)]
    pub private_key_path: Option<String>,
    #[serde(default)]
    pub private_key_content: Option<String>,
    #[serde(default)]
    pub passphrase: Option<String>,
    #[serde(default)]
    pub sync_enabled: bool,
    pub cloud_id: Option<String>,
    pub last_synced_at: Option<i64>,
    pub team_id: Option<String>,
    pub owner_id: Option<String>,
    pub created_at: Option<i64>,
    pub updated_at: Option<i64>,
}

/// Serializes only non-secret credential metadata.
///
/// A `CredentialEntry` returned by the repository contains decrypted secrets.
/// Keeping this explicit implementation prevents generic JSON/logging/sync
/// code from accidentally exporting passwords or private-key material.
impl Serialize for CredentialEntry {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("CredentialEntry", 11)?;
        state.serialize_field("id", &self.id)?;
        state.serialize_field("name", &self.name)?;
        state.serialize_field("kind", &self.kind)?;
        state.serialize_field("username", &self.username)?;
        state.serialize_field("sync_enabled", &self.sync_enabled)?;
        state.serialize_field("cloud_id", &self.cloud_id)?;
        state.serialize_field("last_synced_at", &self.last_synced_at)?;
        state.serialize_field("team_id", &self.team_id)?;
        state.serialize_field("owner_id", &self.owner_id)?;
        state.serialize_field("created_at", &self.created_at)?;
        state.serialize_field("updated_at", &self.updated_at)?;
        state.end()
    }
}

impl std::fmt::Debug for CredentialEntry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CredentialEntry")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("kind", &self.kind)
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
            .field("sync_enabled", &self.sync_enabled)
            .field("cloud_id", &self.cloud_id)
            .field("last_synced_at", &self.last_synced_at)
            .field("team_id", &self.team_id)
            .field("owner_id", &self.owner_id)
            .field("created_at", &self.created_at)
            .field("updated_at", &self.updated_at)
            .finish()
    }
}

impl CredentialEntry {
    pub fn new(name: impl Into<String>, kind: impl Into<String>) -> Self {
        Self {
            id: None,
            name: name.into(),
            kind: kind.into(),
            username: None,
            password: None,
            private_key_path: None,
            private_key_content: None,
            passphrase: None,
            sync_enabled: false,
            cloud_id: None,
            last_synced_at: None,
            team_id: None,
            owner_id: None,
            created_at: None,
            updated_at: None,
        }
    }

    pub fn private_key(&self) -> Option<&str> {
        self.private_key_content
            .as_deref()
            .filter(|value| !value.is_empty())
            .or_else(|| {
                self.private_key_path
                    .as_deref()
                    .filter(|value| !value.is_empty())
            })
    }

    pub(crate) fn has_secrets(&self) -> bool {
        [&self.password, &self.private_key_content, &self.passphrase]
            .into_iter()
            .flatten()
            .any(|value| !value.is_empty())
    }
}

impl crate::storage::traits::Entity for CredentialEntry {
    fn id(&self) -> Option<i64> {
        self.id
    }

    fn created_at(&self) -> i64 {
        self.created_at.expect("credential created_at must exist")
    }

    fn updated_at(&self) -> i64 {
        self.updated_at.expect("credential updated_at must exist")
    }
}

/// Metadata and field availability for credential lists and selectors.
///
/// This type intentionally contains neither encrypted nor decrypted secret
/// values. It can therefore be loaded while the credential vault is locked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialSummary {
    pub id: i64,
    pub name: String,
    pub kind: String,
    pub username: Option<String>,
    pub has_password: bool,
    pub has_private_key_path: bool,
    pub has_private_key_content: bool,
    pub has_passphrase: bool,
    pub sync_enabled: bool,
    pub cloud_id: Option<String>,
    pub last_synced_at: Option<i64>,
    pub team_id: Option<String>,
    pub owner_id: Option<String>,
    pub created_at: Option<i64>,
    pub updated_at: Option<i64>,
}

/// Selects which fields should be copied from a credential entry.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialReference {
    pub credential_id: i64,
    /// 跨设备稳定引用。新记录优先使用此字段，本地整数 ID 仅用于兼容旧记录。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_cloud_id: Option<String>,
    #[serde(default)]
    pub username: bool,
    #[serde(default)]
    pub password: bool,
    #[serde(default)]
    pub private_key: bool,
    #[serde(default)]
    pub passphrase: bool,
}

impl CredentialReference {
    pub fn new(credential_id: i64) -> Self {
        Self {
            credential_id,
            ..Default::default()
        }
    }

    pub fn all(credential_id: i64) -> Self {
        Self {
            credential_id,
            credential_cloud_id: None,
            username: true,
            password: true,
            private_key: true,
            passphrase: true,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReferencedCredentialFields {
    pub username: Option<String>,
    pub password: Option<String>,
    pub private_key: Option<String>,
    pub passphrase: Option<String>,
}

impl ReferencedCredentialFields {
    pub fn new(
        username: Option<String>,
        password: Option<String>,
        private_key: Option<String>,
        passphrase: Option<String>,
    ) -> Self {
        Self {
            username,
            password,
            private_key,
            passphrase,
        }
    }

    pub fn from_entry(entry: &CredentialEntry) -> Self {
        Self::new(
            entry.username.clone(),
            entry.password.clone(),
            entry.private_key().map(str::to_string),
            entry.passphrase.clone(),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CredentialResolutionError {
    MissingCredential(i64),
    EmptyField {
        credential_id: i64,
        field: &'static str,
    },
}

impl std::fmt::Display for CredentialResolutionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingCredential(id) => write!(formatter, "credential {id} was not found"),
            Self::EmptyField {
                credential_id,
                field,
            } => write!(formatter, "credential {credential_id} has no {field}"),
        }
    }
}

impl std::error::Error for CredentialResolutionError {}

pub fn resolve_reference(
    manual: ReferencedCredentialFields,
    credential: Option<&CredentialEntry>,
) -> ReferencedCredentialFields {
    let Some(credential) = credential else {
        return manual;
    };
    let values = ReferencedCredentialFields::from_entry(credential);
    ReferencedCredentialFields {
        username: non_empty_or(manual.username, values.username),
        password: non_empty_or(manual.password, values.password),
        private_key: non_empty_or(manual.private_key, values.private_key),
        passphrase: non_empty_or(manual.passphrase, values.passphrase),
    }
}

pub fn resolve_credential_reference(
    manual: ReferencedCredentialFields,
    reference: &CredentialReference,
    credential: Option<&CredentialEntry>,
) -> ReferencedCredentialFields {
    let Some(credential) = credential else {
        return manual;
    };
    resolve_selected(
        manual,
        reference,
        &ReferencedCredentialFields::from_entry(credential),
    )
}

pub fn resolve_credential_reference_strict(
    manual: ReferencedCredentialFields,
    reference: &CredentialReference,
    credential: Option<&CredentialEntry>,
) -> Result<ReferencedCredentialFields> {
    let credential = credential.ok_or_else(|| {
        anyhow!(CredentialResolutionError::MissingCredential(
            reference.credential_id,
        ))
    })?;
    let values = ReferencedCredentialFields::from_entry(credential);
    for (selected, value, field) in [
        (reference.username, &values.username, "username"),
        (reference.password, &values.password, "password"),
        (reference.private_key, &values.private_key, "private key"),
        (reference.passphrase, &values.passphrase, "passphrase"),
    ] {
        if selected && value.as_deref().is_none_or(str::is_empty) {
            return Err(anyhow!(CredentialResolutionError::EmptyField {
                credential_id: reference.credential_id,
                field,
            }));
        }
    }
    Ok(resolve_selected(manual, reference, &values))
}

fn resolve_selected(
    mut manual: ReferencedCredentialFields,
    reference: &CredentialReference,
    values: &ReferencedCredentialFields,
) -> ReferencedCredentialFields {
    if reference.username {
        manual.username = values.username.clone();
    }
    if reference.password {
        manual.password = values.password.clone();
    }
    if reference.private_key {
        manual.private_key = values.private_key.clone();
    }
    if reference.passphrase {
        manual.passphrase = values.passphrase.clone();
    }
    manual
}

fn non_empty_or(manual: Option<String>, referenced: Option<String>) -> Option<String> {
    referenced.filter(|value| !value.is_empty()).or(manual)
}
