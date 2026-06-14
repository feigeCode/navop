use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WasmRuntimeConfig {
    pub max_memory_mb: u32,
    pub fuel_per_call: u64,
    pub timeout_ms: u64,
}

impl Default for WasmRuntimeConfig {
    fn default() -> Self {
        Self {
            max_memory_mb: 64,
            fuel_per_call: 100_000_000,
            timeout_ms: 5_000,
        }
    }
}

impl WasmRuntimeConfig {
    pub fn resolve_module_path(&self, manifest_dir: &Path, module: &str) -> PathBuf {
        let path = Path::new(module);
        if path.is_absolute() {
            return path.to_path_buf();
        }
        manifest_dir.join(path).components().collect()
    }
}
