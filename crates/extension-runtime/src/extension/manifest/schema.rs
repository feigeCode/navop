use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::contributes::ContributesManifest;

pub const CURRENT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Manifest {
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub publisher: String,
    #[serde(default)]
    pub license: String,
    #[serde(default)]
    pub homepage: String,
    #[serde(default)]
    pub repository: String,
    #[serde(default)]
    pub icon: String,
    #[serde(default)]
    pub description_i18n: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub categories: Vec<String>,
    #[serde(default)]
    pub keywords: Vec<String>,
    pub engines: Engines,
    #[serde(default)]
    pub api: ApiVersions,
    #[serde(default)]
    pub activation: Vec<String>,
    #[serde(default)]
    pub permissions: Vec<String>,
    #[serde(default)]
    pub runtime: RuntimeSection,
    #[serde(default)]
    pub contributes: ContributesManifest,
    #[serde(skip)]
    pub manifest_dir: PathBuf,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct Engines {
    pub onetcli: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ApiVersions {
    #[serde(default = "default_api_version")]
    pub extension: String,
    #[serde(default = "default_api_version")]
    pub database: String,
    #[serde(default = "default_api_version")]
    pub ui: String,
    #[serde(default = "default_api_version")]
    pub task: String,
    #[serde(default = "default_api_version")]
    pub connection: String,
}

impl Default for ApiVersions {
    fn default() -> Self {
        Self {
            extension: default_api_version(),
            database: default_api_version(),
            ui: default_api_version(),
            task: default_api_version(),
            connection: default_api_version(),
        }
    }
}

impl ApiVersions {
    pub fn all_iter(&self) -> impl Iterator<Item = (&'static str, &str)> {
        [
            ("extension", self.extension.as_str()),
            ("database", self.database.as_str()),
            ("ui", self.ui.as_str()),
            ("task", self.task.as_str()),
            ("connection", self.connection.as_str()),
        ]
        .into_iter()
    }
}

fn default_api_version() -> String {
    "1.0".to_string()
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct RuntimeSection {
    #[serde(default)]
    pub ipc: Vec<IpcRuntime>,
    #[serde(default)]
    pub wasm: Vec<WasmRuntime>,
}

impl RuntimeSection {
    pub fn duplicated_ids(&self) -> Vec<String> {
        let mut seen = std::collections::HashMap::<&str, usize>::new();
        for runtime in &self.ipc {
            *seen.entry(runtime.id.as_str()).or_insert(0) += 1;
        }
        for runtime in &self.wasm {
            *seen.entry(runtime.id.as_str()).or_insert(0) += 1;
        }
        seen.into_iter()
            .filter_map(|(id, count)| (count > 1).then(|| id.to_string()))
            .collect()
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct IpcRuntime {
    pub id: String,
    pub entry: IpcEntry,
    #[serde(default)]
    pub transport: IpcTransport,
    #[serde(default = "default_auto_restart")]
    pub auto_restart: bool,
    #[serde(default = "default_max_restart_attempts")]
    pub max_restart_attempts: u32,
    #[serde(default = "default_shutdown_grace_ms")]
    pub shutdown_grace_ms: u64,
}

fn default_auto_restart() -> bool {
    true
}

fn default_max_restart_attempts() -> u32 {
    3
}

fn default_shutdown_grace_ms() -> u64 {
    30_000
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct IpcEntry {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub working_dir: Option<String>,
    #[serde(default)]
    pub env: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct IpcTransport {
    #[serde(default = "default_transport_kind")]
    pub kind: String,
    #[serde(default)]
    pub connect_timeout_ms: Option<u64>,
}

impl Default for IpcTransport {
    fn default() -> Self {
        Self {
            kind: default_transport_kind(),
            connect_timeout_ms: None,
        }
    }
}

fn default_transport_kind() -> String {
    "local_socket".to_string()
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WasmRuntime {
    pub id: String,
    pub module: String,
    pub kind: WasmRuntimeKind,
    #[serde(default = "default_wasm_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default = "default_wasm_max_memory_mb")]
    pub max_memory_mb: u32,
    #[serde(default = "default_wasm_fuel_per_call")]
    pub fuel_per_call: u64,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WasmRuntimeKind {
    Component,
}

fn default_wasm_timeout_ms() -> u64 {
    5_000
}

fn default_wasm_max_memory_mb() -> u32 {
    64
}

fn default_wasm_fuel_per_call() -> u64 {
    100_000_000
}
