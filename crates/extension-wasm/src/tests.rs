use super::*;
#[test]
fn default_config_uses_safe_limits() {
    let config = WasmRuntimeConfig::default();
    assert_eq!(64, config.max_memory_mb);
    assert_eq!(5_000, config.timeout_ms);
    assert_eq!(100_000_000, config.fuel_per_call);
}

#[test]
fn config_resolves_relative_module_path() {
    let root = std::path::Path::new("/tmp/ext");
    let path = WasmRuntimeConfig::default().resolve_module_path(root, "./wasm/plugin.wasm");
    assert_eq!(std::path::PathBuf::from("/tmp/ext/wasm/plugin.wasm"), path);
}

#[test]
fn config_keeps_absolute_module_path() {
    let root = std::path::Path::new("/tmp/ext");
    let path = WasmRuntimeConfig::default().resolve_module_path(root, "/opt/plugin.wasm");
    assert_eq!(std::path::PathBuf::from("/opt/plugin.wasm"), path);
}
