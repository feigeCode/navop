use std::{path::Path, sync::Once};

use gpui_component::highlighter::{LanguageRegistry, register_extension_manifests_dir};

static LANGUAGE_MANIFEST_SCAN: Once = Once::new();

pub fn language_for_path(path: &str, plain_text_mode: bool) -> String {
    if plain_text_mode {
        return "text".to_string();
    }

    let path_without_query = path.split_once('?').map_or(path, |(path, _)| path);

    Path::new(path_without_query)
        .extension()
        .and_then(|extension| extension.to_str())
        .and_then(language_name_for_extension)
        .unwrap_or_else(|| "text".to_string())
}

fn language_name_for_extension(extension: &str) -> Option<String> {
    let registry = LanguageRegistry::singleton();
    registry.language_name_for_extension(extension).or_else(|| {
        scan_language_extension_manifests(registry);
        registry.language_name_for_extension(extension)
    })
}

fn scan_language_extension_manifests(registry: &LanguageRegistry) {
    LANGUAGE_MANIFEST_SCAN.call_once(|| {
        let Ok(config_dir) = one_core::storage::get_config_dir() else {
            return;
        };
        let root = config_dir.join("extensions").join("languages");
        if let Err(error) = register_extension_manifests_dir(&root, registry) {
            tracing::warn!("failed to scan language extension manifests: {error:?}");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::language_for_path;

    #[test]
    fn language_for_path_uses_plain_text_for_large_file_mode() {
        assert_eq!(language_for_path("/tmp/main.rs", true), "text");
    }

    #[test]
    fn language_for_path_maps_known_extensions() {
        assert_eq!(language_for_path("/tmp/index.json", false), "json");
    }

    #[test]
    fn language_for_path_falls_back_to_text() {
        assert_eq!(language_for_path("/tmp/README.unknown", false), "text");
    }

    #[test]
    fn language_for_path_uses_registry_extension_lookup() {
        assert_eq!(language_for_path("/tmp/settings.jsonc", false), "json");
    }

    #[test]
    fn language_for_path_ignores_query_string() {
        let registry = gpui_component::highlighter::LanguageRegistry::singleton();
        registry.register_wasm_manifest(
            gpui_component::highlighter::LanguageManifest {
                name: "__remote_html__".to_string(),
                version: "0.1.0".to_string(),
                file_extensions: vec!["html".to_string()],
                injection_languages: Vec::new(),
                requires: Vec::new(),
                sha256_wasm: None,
            },
            std::path::PathBuf::new(),
        );

        assert_eq!(
            language_for_path("/tmp/index.html?token=Nwiw70H2Gs", false),
            "__remote_html__"
        );

        registry.unregister("__remote_html__");
    }
}
