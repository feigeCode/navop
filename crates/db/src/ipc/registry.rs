use crate::connection::DbError;
use crate::plugin_manifest::{DatabaseCapabilities, DatabaseUiManifest};
use extension_protocol::method;
use one_core::storage::DatabaseType;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use tracing::{debug, info, warn};

mod discovery;
mod entry;

const DRIVER_MANIFEST_FILE: &str = "driver.json";
static IPC_DRIVER_LOG_KEYS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

fn default_driver_api() -> String {
    "database".to_string()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IpcDriverManifest {
    pub id: String,
    pub name: String,
    /// 旧 SQL driver manifest 未声明时默认归类为 database。
    #[serde(default = "default_driver_api")]
    pub api: String,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub version: String,
    /// Sidecar/server compatibility metadata. SQL registry keeps this opaque so
    /// non-SQL selectors can evolve without coupling `db` to their schemas.
    #[serde(default)]
    pub compatibility: serde_json::Value,
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
    pub commands: HashMap<String, String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub working_dir: Option<String>,
    #[serde(default)]
    pub env_from_config: HashMap<String, String>,
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
    pub table_reference_schema_mode: TableReferenceSchemaMode,
    #[serde(default)]
    pub row_id_column: Option<String>,
    #[serde(default)]
    pub row_id_alias: Option<String>,
    #[serde(default)]
    pub default_order_by: Option<String>,
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
            table_reference_schema_mode: TableReferenceSchemaMode::default(),
            row_id_column: None,
            row_id_alias: None,
            default_order_by: None,
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TableReferenceSchemaMode {
    #[default]
    Auto,
    PreferSchema,
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
        let mut capabilities = self.capabilities.clone().unwrap_or(capabilities);
        if !self.methods.is_empty()
            && !self
                .methods
                .iter()
                .any(|driver_method| driver_method == method::SCHEMA_VIEWS)
        {
            capabilities.supports_views = false;
        }
        if !self.methods.is_empty()
            && !self
                .methods
                .iter()
                .any(|driver_method| driver_method == method::SCHEMA_INDEXES)
        {
            capabilities.supports_indexes = false;
        }
        capabilities
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
        if self.api.trim().is_empty() {
            return Err(DbError::connection(format!(
                "external driver '{}' api is required",
                self.id
            )));
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

#[derive(Clone, Debug)]
pub struct IpcDriverRegistryLoadReport {
    pub registry: IpcDriverRegistry,
    pub loaded: Vec<IpcDriverLoadedEntry>,
    pub skipped: Vec<IpcDriverSkippedEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IpcDriverLoadedEntry {
    pub id: String,
    pub name: String,
    pub category: Option<String>,
    pub version: String,
    pub dir: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IpcDriverSkippedEntry {
    pub dir: PathBuf,
    pub error: String,
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
            let report = match Self::load_from_dir_with_report(dir) {
                Ok(report) => report,
                Err(error) => {
                    warn!(
                        path = %dir.display(),
                        error = %error,
                        "skipping external driver directory"
                    );
                    continue;
                }
            };
            for driver in report.registry.drivers {
                if seen.insert(driver.id.clone()) {
                    drivers.push(driver);
                } else {
                    let key = format!("duplicate:{}:{}", driver.id, driver.manifest_dir.display());
                    if should_log_ipc_driver_once(&key) {
                        warn!(
                            target: "extension_loader",
                            kind = "ipc",
                            driver_id = %driver.id,
                            name = %driver.name,
                            path = %driver.manifest_dir.display(),
                            "skipped duplicate ipc driver manifest"
                        );
                    }
                }
            }
        }
        sort_drivers(&mut drivers);
        let loaded_ids: Vec<_> = drivers.iter().map(|driver| driver.id.as_str()).collect();
        let summary_key = format!("registry:{loaded_ids:?}");
        if should_log_ipc_driver_once(&summary_key) {
            info!(
                target: "extension_loader",
                kind = "ipc",
                loaded = loaded_ids.len(),
                drivers = ?loaded_ids,
                "loaded ipc driver registry"
            );
        }
        Ok(Self { drivers })
    }

    pub fn load_from_dir(dir: &Path) -> Result<Self, DbError> {
        Ok(Self::load_from_dir_with_report(dir)?.registry)
    }

    pub fn load_from_dir_with_report(dir: &Path) -> Result<IpcDriverRegistryLoadReport, DbError> {
        if !dir.exists() {
            debug!(
                target: "extension_loader",
                kind = "ipc",
                path = %dir.display(),
                "ipc driver directory does not exist"
            );
            return Ok(IpcDriverRegistryLoadReport {
                registry: Self::empty(),
                loaded: Vec::new(),
                skipped: Vec::new(),
            });
        }

        let mut drivers = Vec::new();
        let mut loaded = Vec::new();
        let mut skipped = Vec::new();
        let mut root_is_wrapped_driver = false;
        if let Some(driver_dir) = driver_manifest_dir_for(dir)? {
            root_is_wrapped_driver = driver_dir != dir;
            load_manifest_into_report(&driver_dir, &mut drivers, &mut loaded, &mut skipped);
        }

        if !root_is_wrapped_driver {
            for entry in std::fs::read_dir(dir).map_err(read_dir_error)? {
                let entry = entry.map_err(read_dir_error)?;
                if entry.file_type().map_err(read_dir_error)?.is_dir() {
                    if let Some(driver_dir) = driver_manifest_dir_for(&entry.path())? {
                        load_manifest_into_report(
                            &driver_dir,
                            &mut drivers,
                            &mut loaded,
                            &mut skipped,
                        );
                    }
                }
            }
        }
        sort_drivers(&mut drivers);
        let loaded_ids: Vec<_> = loaded.iter().map(|entry| entry.id.as_str()).collect();
        let skipped_dirs: Vec<_> = skipped
            .iter()
            .map(|entry| entry.dir.display().to_string())
            .collect();
        let summary_key = format!("dir:{}:{loaded_ids:?}:{skipped_dirs:?}", dir.display());
        if should_log_ipc_driver_once(&summary_key) {
            debug!(
                target: "extension_loader",
                kind = "ipc",
                root = %dir.display(),
                loaded = loaded.len(),
                skipped = skipped.len(),
                drivers = ?loaded_ids,
                skipped_paths = ?skipped_dirs,
                "scanned ipc driver directory"
            );
        }
        Ok(IpcDriverRegistryLoadReport {
            registry: Self { drivers },
            loaded,
            skipped,
        })
    }

    pub fn load_driver_from_dir(dir: &Path) -> Result<Option<IpcDriverManifest>, DbError> {
        if !dir.exists() {
            return Ok(None);
        }
        let Some(driver_dir) = driver_manifest_dir_for(dir)? else {
            return Ok(None);
        };
        load_manifest(&driver_dir).map(Some)
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

    pub fn find_by_api(&self, api: &str, driver_id: &str) -> Option<IpcDriverManifest> {
        self.drivers
            .iter()
            .find(|driver| driver.api == api && driver.id == driver_id)
            .cloned()
    }

    pub fn drivers_for_api(&self, api: &str) -> Vec<IpcDriverManifest> {
        self.drivers
            .iter()
            .filter(|driver| driver.api == api)
            .cloned()
            .collect()
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

fn load_manifest_into_report(
    driver_dir: &Path,
    drivers: &mut Vec<IpcDriverManifest>,
    loaded: &mut Vec<IpcDriverLoadedEntry>,
    skipped: &mut Vec<IpcDriverSkippedEntry>,
) {
    match load_manifest(driver_dir) {
        Ok(driver) => {
            loaded.push(IpcDriverLoadedEntry {
                id: driver.id.clone(),
                name: driver.name.clone(),
                category: driver.category.clone(),
                version: driver.version.clone(),
                dir: driver.manifest_dir.clone(),
            });
            drivers.push(driver);
        }
        Err(error) => {
            let error = error.to_string();
            let key = format!("skip:{}:{error}", driver_dir.display());
            if should_log_ipc_driver_once(&key) {
                warn!(
                    target: "extension_loader",
                    kind = "ipc",
                    path = %driver_dir.display(),
                    error = %error,
                    "skipped ipc driver manifest"
                );
            }
            skipped.push(IpcDriverSkippedEntry {
                dir: driver_dir.to_path_buf(),
                error,
            });
        }
    }
}

fn should_log_ipc_driver_once(key: &str) -> bool {
    let seen = IPC_DRIVER_LOG_KEYS.get_or_init(|| Mutex::new(HashSet::new()));
    seen.lock()
        .map(|mut seen| seen.insert(key.to_string()))
        .unwrap_or(true)
}

fn driver_manifest_dir_for(dir: &Path) -> Result<Option<PathBuf>, DbError> {
    if dir.join(DRIVER_MANIFEST_FILE).is_file() {
        return Ok(Some(dir.to_path_buf()));
    }
    single_wrapped_driver_dir(dir)
}

fn single_wrapped_driver_dir(dir: &Path) -> Result<Option<PathBuf>, DbError> {
    let mut found_dir = None;
    for entry in std::fs::read_dir(dir).map_err(read_dir_error)? {
        let entry = entry.map_err(read_dir_error)?;
        if ignored_archive_metadata(&entry.file_name()) {
            continue;
        }
        if !entry.file_type().map_err(read_dir_error)?.is_dir() {
            return Ok(None);
        }
        if found_dir.replace(entry.path()).is_some() {
            return Ok(None);
        }
    }
    let Some(driver_dir) = found_dir else {
        return Ok(None);
    };
    if driver_dir.join(DRIVER_MANIFEST_FILE).is_file() {
        Ok(Some(driver_dir))
    } else {
        Ok(None)
    }
}

fn ignored_archive_metadata(name: &OsStr) -> bool {
    let Some(name) = name.to_str() else {
        return false;
    };
    name == ".DS_Store" || name == "__MACOSX" || name.starts_with("._")
}

fn read_dir_error(error: std::io::Error) -> DbError {
    DbError::connection_with_source("failed to scan external driver directory", error)
}

fn sort_drivers(drivers: &mut [IpcDriverManifest]) {
    drivers.sort_by(|left, right| left.name.cmp(&right.name).then(left.id.cmp(&right.id)));
}

#[cfg(test)]
mod tests;
