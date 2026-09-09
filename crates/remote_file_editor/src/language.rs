use std::path::Path;

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
    extension_runtime::language_extensions::registered_language_name(extension)
        .or_else(|| local_language_name(extension).map(str::to_string))
}

fn local_language_name(extension: &str) -> Option<&'static str> {
    match extension
        .trim()
        .trim_start_matches('.')
        .to_ascii_lowercase()
        .as_str()
    {
        "json" | "jsonc" => Some("json"),
        _ => None,
    }
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
    fn language_for_path_maps_local_aliases() {
        assert_eq!(language_for_path("/tmp/settings.jsonc", false), "json");
    }

    #[test]
    fn language_for_path_ignores_query_string() {
        assert_eq!(
            language_for_path("/tmp/settings.jsonc?token=Nwiw70H2Gs", false),
            "json"
        );
    }
}
