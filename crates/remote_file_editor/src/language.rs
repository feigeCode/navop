pub fn language_for_path(path: &str, plain_text_mode: bool) -> &'static str {
    if plain_text_mode {
        return "text";
    }

    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".rs") {
        "rust"
    } else if lower.ends_with(".json") {
        "json"
    } else if lower.ends_with(".yaml") || lower.ends_with(".yml") {
        "yaml"
    } else if lower.ends_with(".js") {
        "javascript"
    } else if lower.ends_with(".ts") {
        "typescript"
    } else if lower.ends_with(".md") {
        "markdown"
    } else if lower.ends_with(".sh") {
        "bash"
    } else {
        "text"
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
        assert_eq!(language_for_path("/tmp/main.rs", false), "rust");
        assert_eq!(language_for_path("/tmp/config.yaml", false), "yaml");
        assert_eq!(language_for_path("/tmp/index.json", false), "json");
    }

    #[test]
    fn language_for_path_falls_back_to_text() {
        assert_eq!(language_for_path("/tmp/README.unknown", false), "text");
    }
}
