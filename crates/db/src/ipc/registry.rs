use crate::connection::DbError;
use crate::plugin_manifest::{DatabaseCapabilities, DatabaseUiManifest};
use extension_protocol::method;
use one_core::storage::DatabaseType;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use tracing::warn;

mod discovery;
mod entry;

const DRIVER_MANIFEST_FILE: &str = "driver.json";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IpcDriverManifest {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub version: String,
    pub entry: IpcDriverEntry,
    pub transport: IpcDriverTransport,
    #[serde(default)]
    pub dialect: IpcDriverDialect,
    #[serde(default)]
    pub capabilities: Option<DatabaseCapabilities>,
    #[serde(default)]
    pub connection: IpcDriverConnection,
    #[serde(default)]
    pub methods: Vec<String>,
    #[serde(default)]
    pub ui: IpcDriverUi,
    #[serde(skip)]
    pub manifest_dir: PathBuf,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct IpcDriverConnection {
    pub single_file: bool,
    pub single_connection: bool,
    pub close_on_release: bool,
    pub path_fields: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IpcDriverEntry {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub working_dir: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IpcDriverTransport {
    pub name: String,
    #[serde(default)]
    pub connect_timeout_ms: Option<u64>,
}

impl IpcDriverTransport {
    const DEFAULT_CONNECT_TIMEOUT_MS: u64 = 5_000;

    pub fn local_socket(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            connect_timeout_ms: None,
        }
    }

    pub fn connect_timeout_ms(&self) -> u64 {
        self.connect_timeout_ms
            .unwrap_or(Self::DEFAULT_CONNECT_TIMEOUT_MS)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IpcDriverDialect {
    #[serde(default = "default_identifier_quote_left")]
    pub identifier_quote_left: String,
    #[serde(default)]
    pub identifier_quote_right: Option<String>,
    #[serde(default)]
    pub limit_style: LimitStyle,
    #[serde(default = "default_bool_true")]
    pub bool_true: String,
    #[serde(default = "default_bool_false")]
    pub bool_false: String,
    #[serde(default)]
    pub explain_template: Option<String>,
    #[serde(default)]
    pub compatible_database_type: Option<DatabaseType>,
    #[serde(default)]
    pub supports_schema: bool,
    #[serde(default)]
    pub supports_sequences: bool,
    #[serde(default)]
    pub uses_schema_as_database: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct IpcDriverUi {
    #[serde(default)]
    pub icon: String,
    #[serde(default)]
    pub icon_color: Option<String>,
    #[serde(default)]
    pub locales_dir: Option<String>,
    #[serde(default)]
    pub default_port: Option<u16>,
    #[serde(default)]
    pub form: Option<DatabaseUiManifest>,
}

impl Default for IpcDriverDialect {
    fn default() -> Self {
        Self {
            identifier_quote_left: default_identifier_quote_left(),
            identifier_quote_right: None,
            limit_style: LimitStyle::default(),
            bool_true: default_bool_true(),
            bool_false: default_bool_false(),
            explain_template: None,
            compatible_database_type: None,
            supports_schema: false,
            supports_sequences: false,
            uses_schema_as_database: false,
        }
    }
}

impl IpcDriverDialect {
    pub fn identifier_quote_pair(&self) -> (&str, &str) {
        let left = self.identifier_quote_left.as_str();
        let right = match self.identifier_quote_right.as_deref() {
            Some(right) => right,
            None if left == "[" => "]",
            None => left,
        };
        (left, right)
    }

    pub fn format_explain_sql(&self, sql: &str) -> Option<String> {
        let template = self.explain_template.as_deref().unwrap_or("EXPLAIN {sql}");
        let template = template.trim();
        if template.is_empty() {
            return None;
        }
        if template.contains("{sql}") {
            Some(template.replace("{sql}", sql))
        } else {
            Some(format!("{template} {sql}"))
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LimitStyle {
    #[default]
    LimitOffset,
    OffsetFetch,
}

fn default_identifier_quote_left() -> String {
    "\"".to_string()
}

fn default_bool_true() -> String {
    "TRUE".to_string()
}

fn default_bool_false() -> String {
    "FALSE".to_string()
}

impl IpcDriverManifest {
    pub fn command_working_dir(&self) -> PathBuf {
        self.entry
            .working_dir
            .as_deref()
            .map(|dir| self.manifest_dir.join(dir))
            .unwrap_or_else(|| self.manifest_dir.clone())
    }

    pub fn effective_capabilities(&self) -> DatabaseCapabilities {
        let mut capabilities = self
            .ui
            .form
            .as_ref()
            .map(|manifest| manifest.capabilities.clone())
            .unwrap_or_else(|| DatabaseCapabilities {
                supports_functions: true,
                supports_procedures: true,
                ..DatabaseCapabilities::default()
            });
        capabilities.supports_schema |= self.dialect.supports_schema;
        capabilities.supports_sequences |= self.dialect.supports_sequences;
        capabilities.uses_schema_as_database |= self.dialect.uses_schema_as_database;
        self.capabilities.clone().unwrap_or(capabilities)
    }

    pub fn icon_path(&self) -> Option<PathBuf> {
        if self.ui.icon.trim().is_empty() {
            return None;
        }
        Some(self.manifest_dir.join(&self.ui.icon))
    }

    pub fn icon_color_path(&self) -> Option<PathBuf> {
        self.ui
            .icon_color
            .as_ref()
            .filter(|path| !path.trim().is_empty())
            .map(|path| self.manifest_dir.join(path))
    }

    pub fn locales_dir(&self) -> Option<PathBuf> {
        self.ui
            .locales_dir
            .as_ref()
            .filter(|path| !path.trim().is_empty())
            .map(|path| self.manifest_dir.join(path))
    }

    pub fn load_locale(&self, locale: &str) -> Result<serde_yaml::Value, DbError> {
        let locales_dir = self.locales_dir().ok_or_else(|| {
            DbError::connection(format!("driver '{}' has no locales directory", self.id))
        })?;

        let locale_file = locales_dir.join(format!("{locale}.yml"));
        if locale_file.exists() {
            return load_yaml_file(&locale_file);
        }

        let en_file = locales_dir.join("en.yml");
        if en_file.exists() {
            return load_yaml_file(&en_file);
        }

        Err(DbError::connection(format!(
            "driver '{}' has no locale file for '{}'",
            self.id, locale
        )))
    }

    fn validate(&self) -> Result<(), DbError> {
        if self.id.trim().is_empty() || self.name.trim().is_empty() {
            return Err(DbError::connection(
                "external driver id and name are required",
            ));
        }
        if self.entry.command.trim().is_empty() {
            return Err(DbError::connection(format!(
                "external driver '{}' command is required",
                self.id
            )));
        }
        if self.transport.name.trim().is_empty() {
            return Err(DbError::connection(format!(
                "external driver '{}' local socket name is required",
                self.id
            )));
        }
        for method_name in &self.methods {
            if !is_allowed_manifest_method(method_name) {
                return Err(DbError::connection(format!(
                    "external driver '{}' declares unknown IPC method '{}'",
                    self.id, method_name
                )));
            }
        }
        Ok(())
    }
}

fn load_yaml_file(path: &Path) -> Result<serde_yaml::Value, DbError> {
    let content = std::fs::read_to_string(path).map_err(|error| {
        DbError::connection_with_source("failed to read driver locale file", error)
    })?;
    serde_yaml::from_str(&content)
        .map_err(|error| DbError::connection_with_source("invalid driver locale file", error))
}

fn is_allowed_manifest_method(method_name: &str) -> bool {
    method::is_allowed_declaration(method_name)
}

#[derive(Clone, Debug)]
pub struct IpcDriverRegistry {
    drivers: Vec<IpcDriverManifest>,
}

impl IpcDriverRegistry {
    pub fn load_default() -> Self {
        Self::load_from_dirs(&discovery::default_driver_dirs()).unwrap_or_else(|_| Self::empty())
    }

    pub fn from_drivers(mut drivers: Vec<IpcDriverManifest>) -> Self {
        sort_drivers(&mut drivers);
        Self { drivers }
    }

    pub fn load_from_dirs(dirs: &[PathBuf]) -> Result<Self, DbError> {
        let mut drivers = Vec::new();
        let mut seen = HashSet::new();
        for dir in dirs {
            let registry = match Self::load_from_dir(dir) {
                Ok(registry) => registry,
                Err(error) => {
                    warn!(
                        path = %dir.display(),
                        error = %error,
                        "skipping external driver directory"
                    );
                    continue;
                }
            };
            for driver in registry.drivers {
                if seen.insert(driver.id.clone()) {
                    drivers.push(driver);
                }
            }
        }
        sort_drivers(&mut drivers);
        Ok(Self { drivers })
    }

    pub fn load_from_dir(dir: &Path) -> Result<Self, DbError> {
        if !dir.exists() {
            return Ok(Self::empty());
        }

        let mut drivers = Vec::new();
        if dir.join(DRIVER_MANIFEST_FILE).is_file() {
            if let Ok(driver) = load_manifest(dir) {
                drivers.push(driver);
            }
        }

        for entry in std::fs::read_dir(dir).map_err(read_dir_error)? {
            let entry = entry.map_err(read_dir_error)?;
            if entry.file_type().map_err(read_dir_error)?.is_dir() {
                if let Ok(driver) = load_manifest(&entry.path()) {
                    drivers.push(driver);
                }
            }
        }
        sort_drivers(&mut drivers);
        Ok(Self { drivers })
    }

    pub fn empty() -> Self {
        Self {
            drivers: Vec::new(),
        }
    }

    pub fn drivers(&self) -> &[IpcDriverManifest] {
        &self.drivers
    }

    pub fn find(&self, driver_id: &str) -> Option<IpcDriverManifest> {
        self.drivers
            .iter()
            .find(|driver| driver.id == driver_id)
            .cloned()
    }
}

pub fn default_driver_dir() -> PathBuf {
    discovery::default_user_driver_dir()
}

pub fn default_driver_dirs() -> Vec<PathBuf> {
    discovery::default_driver_dirs()
}

fn load_manifest(driver_dir: &Path) -> Result<IpcDriverManifest, DbError> {
    let path = driver_dir.join(DRIVER_MANIFEST_FILE);
    let content = std::fs::read_to_string(&path).map_err(|error| {
        DbError::connection_with_source("failed to read driver manifest", error)
    })?;
    let mut manifest: IpcDriverManifest = serde_json::from_str(&content)
        .map_err(|error| DbError::connection_with_source("invalid driver manifest", error))?;
    manifest.manifest_dir = driver_dir.to_path_buf();
    manifest.validate()?;
    entry::resolve_entry_command(&mut manifest);
    Ok(manifest)
}

fn read_dir_error(error: std::io::Error) -> DbError {
    DbError::connection_with_source("failed to scan external driver directory", error)
}

fn sort_drivers(drivers: &mut [IpcDriverManifest]) {
    drivers.sort_by(|left, right| left.name.cmp(&right.name).then(left.id.cmp(&right.id)));
}

#[cfg(test)]
mod tests;
