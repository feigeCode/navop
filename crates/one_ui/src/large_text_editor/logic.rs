use gpui_component::highlighter::Language;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LargeTextEditorTab {
    Text,
    Json,
}

impl LargeTextEditorTab {
    pub fn language(&self) -> Language {
        match self {
            LargeTextEditorTab::Text => Language::Plain,
            LargeTextEditorTab::Json => Language::Json,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum JsonEditorSyncMode {
    Pretty,
    Mirror,
}

fn canonicalize_large_text_value(value: &str) -> Option<String> {
    json5::from_str::<serde_json::Value>(value)
        .ok()
        .map(|json| json.to_string())
}

pub fn large_text_values_equivalent(original: &str, candidate: &str) -> bool {
    if original == candidate {
        return true;
    }

    match (
        canonicalize_large_text_value(original),
        canonicalize_large_text_value(candidate),
    ) {
        (Some(original_json), Some(candidate_json)) => original_json == candidate_json,
        _ => false,
    }
}

pub(super) fn active_editor_text(
    active_tab: LargeTextEditorTab,
    text_content: &str,
    json_content: &str,
) -> String {
    match active_tab {
        LargeTextEditorTab::Text => text_content.to_string(),
        LargeTextEditorTab::Json => json_content.to_string(),
    }
}

pub(super) fn editor_values_for_text(
    text: &str,
    json_sync_mode: JsonEditorSyncMode,
) -> (String, String) {
    let json_text = match json_sync_mode {
        JsonEditorSyncMode::Pretty => match json5::from_str::<serde_json::Value>(text) {
            Ok(value) => serde_json::to_string_pretty(&value).unwrap_or_else(|_| text.to_string()),
            Err(_) => text.to_string(),
        },
        JsonEditorSyncMode::Mirror => text.to_string(),
    };

    (text.to_string(), json_text)
}

pub(super) fn normalize_commit_text(
    active_tab: LargeTextEditorTab,
    raw_text: &str,
) -> Result<String, json5::Error> {
    if active_tab == LargeTextEditorTab::Json {
        return json5::from_str::<serde_json::Value>(raw_text).map(|value| value.to_string());
    }

    match json5::from_str::<serde_json::Value>(raw_text) {
        Ok(value) => Ok(value.to_string()),
        Err(_) => Ok(raw_text.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_commit_text_minifies_valid_json_from_text_tab() {
        let value = normalize_commit_text(LargeTextEditorTab::Text, "{\n  \"a\": 1\n}")
            .expect("text tab JSON should be minified");

        assert_eq!("{\"a\":1}", value);
    }

    #[test]
    fn normalize_commit_text_preserves_plain_text_from_text_tab() {
        let value = normalize_commit_text(LargeTextEditorTab::Text, "plain text")
            .expect("plain text should be preserved");

        assert_eq!("plain text", value);
    }

    #[test]
    fn normalize_commit_text_requires_valid_json_from_json_tab() {
        let err = normalize_commit_text(LargeTextEditorTab::Json, "{invalid json}")
            .expect_err("json tab should validate before commit");

        assert!(!err.to_string().is_empty());
    }

    #[test]
    fn large_text_values_equivalent_ignores_json_formatting() {
        assert!(large_text_values_equivalent(
            "{\n  \"name\": \"codex\",\n  \"enabled\": true\n}",
            "{\"name\":\"codex\",\"enabled\":true}",
        ));
    }

    #[test]
    fn active_editor_text_returns_active_tab_content() {
        assert_eq!(
            "plain text",
            active_editor_text(LargeTextEditorTab::Text, "plain text", "{\n  \"a\": 1\n}")
        );
        assert_eq!(
            "{\n  \"a\": 1\n}",
            active_editor_text(LargeTextEditorTab::Json, "plain text", "{\n  \"a\": 1\n}")
        );
    }

    #[test]
    fn editor_values_for_text_pretty_formats_json_for_json_editor() {
        let (_, json_value) = editor_values_for_text(
            "{\"name\":\"codex\",\"enabled\":true}",
            JsonEditorSyncMode::Pretty,
        );

        assert_eq!(
            "{\n  \"name\": \"codex\",\n  \"enabled\": true\n}",
            json_value
        );
    }

    #[test]
    fn editor_values_for_text_mirror_keeps_minified_json_visible() {
        let (text_value, json_value) = editor_values_for_text(
            "{\"name\":\"codex\",\"enabled\":true}",
            JsonEditorSyncMode::Mirror,
        );

        assert_eq!("{\"name\":\"codex\",\"enabled\":true}", text_value);
        assert_eq!("{\"name\":\"codex\",\"enabled\":true}", json_value);
    }
}
