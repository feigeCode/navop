use gpui::{App, FontWeight, HighlightStyle, Hsla, SharedString};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_repr::{Deserialize_repr, Serialize_repr};
use std::{
    collections::HashMap,
    ops::Deref,
    path::PathBuf,
    sync::{Arc, LazyLock, Mutex},
};

use crate::{
    ActiveTheme, DEFAULT_THEME_COLORS, ThemeMode,
    highlighter::{InstalledExtension, Language, LanguageManifest, languages, wasm_store},
};

pub(super) const HIGHLIGHT_NAMES: [&str; 40] = [
    "attribute",
    "boolean",
    "comment",
    "comment.doc",
    "constant",
    "constructor",
    "embedded",
    "emphasis",
    "emphasis.strong",
    "enum",
    "function",
    "hint",
    "keyword",
    "label",
    "link_text",
    "link_uri",
    "number",
    "operator",
    "predictive",
    "preproc",
    "primary",
    "property",
    "punctuation",
    "punctuation.bracket",
    "punctuation.delimiter",
    "punctuation.list_marker",
    "punctuation.special",
    "string",
    "string.escape",
    "string.regex",
    "string.special",
    "string.special.symbol",
    "tag",
    "tag.doctype",
    "text.literal",
    "title",
    "type",
    "variable",
    "variable.special",
    "variant",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanguageConfig {
    pub name: SharedString,
    pub language: tree_sitter::Language,
    pub kind: LanguageKind,
    pub injection_languages: Vec<SharedString>,
    pub highlights: SharedString,
    pub injections: SharedString,
    pub locals: SharedString,
}

/// 区分语言是静态链接的 native parser 还是通过 wasm 扩展加载的。
#[derive(Debug, Clone)]
pub enum LanguageKind {
    Native,
    Wasm { wasm_bytes: Arc<[u8]> },
}

impl PartialEq for LanguageKind {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Native, Self::Native) => true,
            (Self::Wasm { wasm_bytes: a }, Self::Wasm { wasm_bytes: b }) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}

impl Eq for LanguageKind {}

impl LanguageConfig {
    pub fn new(
        name: impl Into<SharedString>,
        language: tree_sitter::Language,
        injection_languages: Vec<SharedString>,
        highlights: &str,
        injections: &str,
        locals: &str,
    ) -> Self {
        Self {
            name: name.into(),
            language,
            kind: LanguageKind::Native,
            injection_languages,
            highlights: SharedString::from(highlights.to_string()),
            injections: SharedString::from(injections.to_string()),
            locals: SharedString::from(locals.to_string()),
        }
    }

    /// 使用 wasm parser 构造 `LanguageConfig`。
    pub fn new_wasm(
        name: impl Into<SharedString>,
        language: tree_sitter::Language,
        wasm_bytes: Arc<[u8]>,
        injection_languages: Vec<SharedString>,
        highlights: &str,
        injections: &str,
        locals: &str,
    ) -> Self {
        Self {
            name: name.into(),
            language,
            kind: LanguageKind::Wasm { wasm_bytes },
            injection_languages,
            highlights: SharedString::from(highlights.to_string()),
            injections: SharedString::from(injections.to_string()),
            locals: SharedString::from(locals.to_string()),
        }
    }
}

/// Theme for Tree-sitter Highlight
///
/// https://docs.rs/tree-sitter-highlight/0.26.8/tree_sitter_highlight/
#[derive(Debug, Default, Clone, PartialEq, Eq, Hash, JsonSchema, Serialize, Deserialize)]
pub struct SyntaxColors {
    pub attribute: Option<ThemeStyle>,
    pub boolean: Option<ThemeStyle>,
    pub comment: Option<ThemeStyle>,
    pub comment_doc: Option<ThemeStyle>,
    pub constant: Option<ThemeStyle>,
    pub constructor: Option<ThemeStyle>,
    pub embedded: Option<ThemeStyle>,
    pub emphasis: Option<ThemeStyle>,
    #[serde(rename = "emphasis.strong")]
    pub emphasis_strong: Option<ThemeStyle>,
    #[serde(rename = "enum")]
    pub enum_: Option<ThemeStyle>,
    pub function: Option<ThemeStyle>,
    pub hint: Option<ThemeStyle>,
    pub keyword: Option<ThemeStyle>,
    pub label: Option<ThemeStyle>,
    #[serde(rename = "link_text")]
    pub link_text: Option<ThemeStyle>,
    #[serde(rename = "link_uri")]
    pub link_uri: Option<ThemeStyle>,
    pub number: Option<ThemeStyle>,
    pub operator: Option<ThemeStyle>,
    pub predictive: Option<ThemeStyle>,
    pub preproc: Option<ThemeStyle>,
    pub primary: Option<ThemeStyle>,
    pub property: Option<ThemeStyle>,
    pub punctuation: Option<ThemeStyle>,
    #[serde(rename = "punctuation.bracket")]
    pub punctuation_bracket: Option<ThemeStyle>,
    #[serde(rename = "punctuation.delimiter")]
    pub punctuation_delimiter: Option<ThemeStyle>,
    #[serde(rename = "punctuation.list_marker")]
    pub punctuation_list_marker: Option<ThemeStyle>,
    #[serde(rename = "punctuation.special")]
    pub punctuation_special: Option<ThemeStyle>,
    pub string: Option<ThemeStyle>,
    #[serde(rename = "string.escape")]
    pub string_escape: Option<ThemeStyle>,
    #[serde(rename = "string.regex")]
    pub string_regex: Option<ThemeStyle>,
    #[serde(rename = "string.special")]
    pub string_special: Option<ThemeStyle>,
    #[serde(rename = "string.special.symbol")]
    pub string_special_symbol: Option<ThemeStyle>,
    pub tag: Option<ThemeStyle>,
    #[serde(rename = "tag.doctype")]
    pub tag_doctype: Option<ThemeStyle>,
    #[serde(rename = "text.literal")]
    pub text_literal: Option<ThemeStyle>,
    pub title: Option<ThemeStyle>,
    #[serde(rename = "type")]
    pub type_: Option<ThemeStyle>,
    pub variable: Option<ThemeStyle>,
    #[serde(rename = "variable.special")]
    pub variable_special: Option<ThemeStyle>,
    pub variant: Option<ThemeStyle>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, JsonSchema, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FontStyle {
    Normal,
    Italic,
    Underline,
}

impl From<FontStyle> for gpui::FontStyle {
    fn from(style: FontStyle) -> Self {
        match style {
            FontStyle::Normal => gpui::FontStyle::Normal,
            FontStyle::Italic => gpui::FontStyle::Italic,
            FontStyle::Underline => gpui::FontStyle::Normal,
        }
    }
}

#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, Serialize_repr, Deserialize_repr, JsonSchema)]
#[repr(u16)]
pub enum FontWeightContent {
    Thin = 100,
    ExtraLight = 200,
    Light = 300,
    Normal = 400,
    Medium = 500,
    Semibold = 600,
    Bold = 700,
    ExtraBold = 800,
    Black = 900,
}

impl From<FontWeightContent> for FontWeight {
    fn from(value: FontWeightContent) -> Self {
        match value {
            FontWeightContent::Thin => FontWeight::THIN,
            FontWeightContent::ExtraLight => FontWeight::EXTRA_LIGHT,
            FontWeightContent::Light => FontWeight::LIGHT,
            FontWeightContent::Normal => FontWeight::NORMAL,
            FontWeightContent::Medium => FontWeight::MEDIUM,
            FontWeightContent::Semibold => FontWeight::SEMIBOLD,
            FontWeightContent::Bold => FontWeight::BOLD,
            FontWeightContent::ExtraBold => FontWeight::EXTRA_BOLD,
            FontWeightContent::Black => FontWeight::BLACK,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, JsonSchema, Serialize, Deserialize)]
pub struct ThemeStyle {
    color: Option<Hsla>,
    font_style: Option<FontStyle>,
    font_weight: Option<FontWeightContent>,
}

impl From<ThemeStyle> for HighlightStyle {
    fn from(style: ThemeStyle) -> Self {
        HighlightStyle {
            color: style.color,
            font_weight: style.font_weight.map(Into::into),
            font_style: style.font_style.map(Into::into),
            ..Default::default()
        }
    }
}

impl SyntaxColors {
    pub fn style(&self, name: &str) -> Option<HighlightStyle> {
        if name.is_empty() {
            return None;
        }

        let style = match name {
            "attribute" => self.attribute,
            "boolean" => self.boolean,
            "comment" => self.comment,
            "comment.doc" => self.comment_doc,
            "constant" => self.constant,
            "constructor" => self.constructor,
            "embedded" => self.embedded,
            "emphasis" => self.emphasis,
            "emphasis.strong" => self.emphasis_strong,
            "enum" => self.enum_,
            "function" => self.function,
            "hint" => self.hint,
            "keyword" => self.keyword,
            "label" => self.label,
            "link_text" => self.link_text,
            "link_uri" => self.link_uri,
            "number" => self.number,
            "operator" => self.operator,
            "predictive" => self.predictive,
            "preproc" => self.preproc,
            "primary" => self.primary,
            "property" => self.property,
            "punctuation" => self.punctuation,
            "punctuation.bracket" => self.punctuation_bracket,
            "punctuation.delimiter" => self.punctuation_delimiter,
            "punctuation.list_marker" => self.punctuation_list_marker,
            "punctuation.special" => self.punctuation_special,
            "string" => self.string,
            "string.escape" => self.string_escape,
            "string.regex" => self.string_regex,
            "string.special" => self.string_special,
            "string.special.symbol" => self.string_special_symbol,
            "tag" => self.tag,
            "tag.doctype" => self.tag_doctype,
            "text.literal" => self.text_literal,
            "title" => self.title,
            "type" => self.type_,
            "variable" => self.variable,
            "variable.special" => self.variable_special,
            "variant" => self.variant,
            _ => None,
        }
        .map(|s| s.into());

        if style.is_some() {
            style
        } else {
            // Fallback `keyword.modifier` to `keyword`
            if name.contains(".") {
                if let Some(prefix) = name.split(".").next() {
                    return self.style(prefix);
                }

                None
            } else {
                None
            }
        }
    }

    #[inline]
    pub fn style_for_index(&self, index: usize) -> Option<HighlightStyle> {
        HIGHLIGHT_NAMES.get(index).and_then(|name| self.style(name))
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Hash, JsonSchema, Serialize, Deserialize)]
pub struct StatusColors {
    #[serde(rename = "error")]
    error: Option<Hsla>,
    #[serde(rename = "error.background")]
    error_background: Option<Hsla>,
    #[serde(rename = "error.border")]
    error_border: Option<Hsla>,
    #[serde(rename = "warning")]
    warning: Option<Hsla>,
    #[serde(rename = "warning.background")]
    warning_background: Option<Hsla>,
    #[serde(rename = "warning.border")]
    warning_border: Option<Hsla>,
    #[serde(rename = "info")]
    info: Option<Hsla>,
    #[serde(rename = "info.background")]
    info_background: Option<Hsla>,
    #[serde(rename = "info.border")]
    info_border: Option<Hsla>,
    #[serde(rename = "success")]
    success: Option<Hsla>,
    #[serde(rename = "success.background")]
    success_background: Option<Hsla>,
    #[serde(rename = "success.border")]
    success_border: Option<Hsla>,
    #[serde(rename = "hint")]
    hint: Option<Hsla>,
    #[serde(rename = "hint.background")]
    hint_background: Option<Hsla>,
    #[serde(rename = "hint.border")]
    hint_border: Option<Hsla>,
}

impl StatusColors {
    #[inline]
    pub fn error(&self, cx: &App) -> Hsla {
        self.error.unwrap_or(cx.theme().red)
    }

    #[inline]
    pub fn error_background(&self, cx: &App) -> Hsla {
        let bg = cx.theme().background;
        self.error_background
            .unwrap_or(bg.blend(self.error(cx).alpha(0.2)))
    }

    #[inline]
    pub fn error_border(&self, cx: &App) -> Hsla {
        self.error_border.unwrap_or(self.error(cx))
    }

    #[inline]
    pub fn warning(&self, cx: &App) -> Hsla {
        self.warning.unwrap_or(cx.theme().yellow)
    }

    #[inline]
    pub fn warning_background(&self, cx: &App) -> Hsla {
        let bg = cx.theme().background;
        self.warning_background
            .unwrap_or(bg.blend(self.warning(cx).alpha(0.2)))
    }

    #[inline]
    pub fn warning_border(&self, cx: &App) -> Hsla {
        self.warning_border.unwrap_or(self.warning(cx))
    }

    #[inline]
    pub fn info(&self, cx: &App) -> Hsla {
        self.info.unwrap_or(cx.theme().blue)
    }

    #[inline]
    pub fn info_background(&self, cx: &App) -> Hsla {
        let bg = cx.theme().background;
        self.info_background
            .unwrap_or(bg.blend(self.info(cx).alpha(0.2)))
    }

    #[inline]
    pub fn info_border(&self, cx: &App) -> Hsla {
        self.info_border.unwrap_or(self.info(cx))
    }

    #[inline]
    pub fn success(&self, cx: &App) -> Hsla {
        self.success.unwrap_or(cx.theme().green)
    }

    #[inline]
    pub fn success_background(&self, cx: &App) -> Hsla {
        let bg = cx.theme().background;
        self.success_background
            .unwrap_or(bg.blend(self.success(cx).alpha(0.2)))
    }

    #[inline]
    pub fn success_border(&self, cx: &App) -> Hsla {
        self.success_border.unwrap_or(self.success(cx))
    }

    #[inline]
    pub fn hint(&self, cx: &App) -> Hsla {
        self.hint.unwrap_or(cx.theme().cyan)
    }

    #[inline]
    pub fn hint_background(&self, cx: &App) -> Hsla {
        let bg = cx.theme().background;
        self.hint_background
            .unwrap_or(bg.blend(self.hint(cx).alpha(0.2)))
    }

    #[inline]
    pub fn hint_border(&self, cx: &App) -> Hsla {
        self.hint_border.unwrap_or(self.hint(cx))
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Hash, JsonSchema, Serialize, Deserialize)]
pub struct HighlightThemeStyle {
    #[serde(rename = "editor.background")]
    pub editor_background: Option<Hsla>,
    #[serde(rename = "editor.foreground")]
    pub editor_foreground: Option<Hsla>,
    #[serde(rename = "editor.active_line.background")]
    pub editor_active_line: Option<Hsla>,
    #[serde(rename = "editor.line_number")]
    pub editor_line_number: Option<Hsla>,
    #[serde(rename = "editor.active_line_number")]
    pub editor_active_line_number: Option<Hsla>,
    #[serde(rename = "editor.invisible")]
    pub editor_invisible: Option<Hsla>,
    #[serde(flatten)]
    pub status: StatusColors,
    #[serde(rename = "syntax")]
    pub syntax: SyntaxColors,
}

/// Theme for Tree-sitter Highlight from JSON theme file.
///
/// This json is compatible with the Zed theme format.
///
/// https://zed.dev/docs/extensions/languages#syntax-highlighting
#[derive(Debug, Clone, PartialEq, Eq, Hash, JsonSchema, Serialize, Deserialize)]
pub struct HighlightTheme {
    pub name: String,
    #[serde(default)]
    pub appearance: ThemeMode,
    pub style: HighlightThemeStyle,
}

impl Deref for HighlightTheme {
    type Target = SyntaxColors;

    fn deref(&self) -> &Self::Target {
        &self.style.syntax
    }
}

impl HighlightTheme {
    pub fn default_dark() -> Arc<Self> {
        DEFAULT_THEME_COLORS[&ThemeMode::Dark].1.clone()
    }

    pub fn default_light() -> Arc<Self> {
        DEFAULT_THEME_COLORS[&ThemeMode::Light].1.clone()
    }
}

/// Registry for code highlighter languages.
pub struct LanguageRegistry {
    languages: Mutex<HashMap<SharedString, LanguageConfig>>,
    file_extensions: Mutex<HashMap<SharedString, SharedString>>,
    wasm_extensions: Mutex<HashMap<SharedString, LazyWasmExtension>>,
}

#[derive(Debug, Clone)]
struct LazyWasmExtension {
    source_path: PathBuf,
}

impl LanguageRegistry {
    fn with_default_languages() -> Self {
        Self {
            languages: Mutex::new(HashMap::new()),
            file_extensions: Mutex::new(HashMap::new()),
            wasm_extensions: Mutex::new(HashMap::new()),
        }
    }

    /// Returns the singleton instance of the `LanguageRegistry` with default languages and themes.
    pub fn singleton() -> &'static LazyLock<LanguageRegistry> {
        static INSTANCE: LazyLock<LanguageRegistry> =
            LazyLock::new(LanguageRegistry::with_default_languages);
        &INSTANCE
    }

    /// Registers a new language configuration to the registry.
    pub fn register(&self, lang: &str, config: &LanguageConfig) {
        self.register_with_file_extensions(lang, config, &[]);
    }

    /// Registers a language configuration and associates it with file extensions.
    pub fn register_with_file_extensions(
        &self,
        lang: &str,
        config: &LanguageConfig,
        file_extensions: &[String],
    ) {
        let lang = SharedString::from(lang.to_string());
        self.languages
            .lock()
            .unwrap()
            .insert(lang.clone(), config.clone());
        self.register_file_extensions(&lang, file_extensions);
    }

    /// Registers language extension metadata without loading the wasm parser yet.
    pub fn register_wasm_manifest(&self, manifest: LanguageManifest, source_path: PathBuf) {
        let lang = SharedString::from(manifest.name.clone());
        self.register_file_extensions(&lang, &manifest.file_extensions);
        self.wasm_extensions
            .lock()
            .unwrap()
            .insert(lang, LazyWasmExtension { source_path });
    }

    /// 注册一个通过 wasm 加载的语言扩展。
    pub fn register_wasm(
        &self,
        name: &str,
        wasm_bytes: impl Into<Arc<[u8]>>,
        file_extensions: &[String],
        injection_languages: Vec<SharedString>,
        highlights: &str,
        injections: &str,
        locals: &str,
    ) -> anyhow::Result<()> {
        let bytes: Arc<[u8]> = wasm_bytes.into();
        let language = wasm_store::with_registry_store(|store| store.load_language(name, &bytes))
            .map_err(|e| anyhow::anyhow!("load wasm language {name}: {e}"))?;

        let config = LanguageConfig::new_wasm(
            name.to_string(),
            language,
            bytes,
            injection_languages,
            highlights,
            injections,
            locals,
        );

        self.languages
            .lock()
            .unwrap()
            .insert(name.to_string().into(), config);
        self.wasm_extensions
            .lock()
            .unwrap()
            .remove(&SharedString::from(name.to_string()));
        self.register_file_extensions(&SharedString::from(name.to_string()), file_extensions);
        Ok(())
    }

    /// Returns a list of all registered language names.
    pub fn languages(&self) -> Vec<SharedString> {
        let mut languages: Vec<_> = languages::Language::all()
            .map(|language| SharedString::from(language.name()))
            .collect();
        languages.extend(self.languages.lock().unwrap().keys().cloned());
        languages.extend(self.wasm_extensions.lock().unwrap().keys().cloned());
        languages.sort();
        languages.dedup();
        languages
    }

    /// Returns the language configuration for the given language name.
    pub fn language(&self, name: &str) -> Option<LanguageConfig> {
        // Try to get by name first, there may have a custom language registered
        // Then try to get built-in language to support short language names, e.g. "js" for "javascript"
        let languages = self.languages.lock().unwrap();
        if let Some(config) = languages.get(name).cloned() {
            return Some(config);
        }
        if let Some(language) = Language::from_name(name) {
            return Some(
                languages
                    .get(language.name())
                    .cloned()
                    .unwrap_or_else(|| language.config()),
            );
        }
        drop(languages);

        self.load_lazy_wasm_language(name)
    }

    /// Returns the registered language name for a file extension, without the leading dot.
    pub fn language_name_for_extension(&self, extension: &str) -> Option<String> {
        let extension = normalize_file_extension(extension)?;
        self.file_extensions
            .lock()
            .unwrap()
            .get(extension.as_str())
            .map(|language| language.to_string())
            .or_else(|| Language::from_name(&extension).map(|language| language.name().to_string()))
    }

    /// 移除一个已注册的语言。返回是否真的有条目被删除。
    pub fn unregister(&self, name: &str) -> bool {
        let builtin = builtin_language_config(name);
        let key = SharedString::from(name.to_string());
        let mut languages = self.languages.lock().unwrap();

        match languages.get(&key) {
            Some(current) if builtin.as_ref().is_some_and(|config| current == config) => false,
            Some(_) => {
                languages.remove(&key);
                self.unregister_file_extensions(name);
                self.wasm_extensions.lock().unwrap().remove(&key);
                true
            }
            None => {
                let removed = self.wasm_extensions.lock().unwrap().remove(&key).is_some();
                if removed {
                    self.unregister_file_extensions(name);
                }
                removed
            }
        }
    }

    /// 查询指定语言是否是通过 wasm 扩展加载的。
    pub fn is_wasm(&self, name: &str) -> Option<bool> {
        self.language(name)
            .map(|config| matches!(config.kind, LanguageKind::Wasm { .. }))
    }
}

impl LanguageRegistry {
    fn load_lazy_wasm_language(&self, name: &str) -> Option<LanguageConfig> {
        let extension = self.wasm_extensions.lock().unwrap().get(name).cloned()?;
        match InstalledExtension::load_from_dir(&extension.source_path)
            .and_then(|extension| extension.register(self))
        {
            Ok(()) => self.languages.lock().unwrap().get(name).cloned(),
            Err(error) => {
                tracing::warn!("failed to lazy load language extension {name:?}: {error:?}");
                None
            }
        }
    }

    fn register_file_extensions(&self, lang: &SharedString, file_extensions: &[String]) {
        let mut extensions = self.file_extensions.lock().unwrap();
        for extension in file_extensions {
            if let Some(extension) = normalize_file_extension(extension) {
                extensions.insert(extension.into(), lang.clone());
            }
        }
    }

    fn unregister_file_extensions(&self, lang: &str) {
        self.file_extensions
            .lock()
            .unwrap()
            .retain(|_, language| language.as_ref() != lang);
    }
}

fn normalize_file_extension(extension: &str) -> Option<String> {
    let normalized = extension
        .trim()
        .trim_start_matches('.')
        .to_ascii_lowercase();
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

fn builtin_language_config(name: &str) -> Option<LanguageConfig> {
    languages::Language::all()
        .find(|language| language.name() == name)
        .map(|language| language.config())
}

#[cfg(test)]
mod tests {
    use crate::highlighter::{LanguageConfig, LanguageRegistry};

    #[test]
    fn test_registry() {
        let registry = LanguageRegistry::with_default_languages();

        registry.register(
            "foo",
            &LanguageConfig::new("foo", tree_sitter_json::LANGUAGE.into(), vec![], "", "", ""),
        );

        assert!(registry.language("foo").is_some());
        assert!(registry.language("json").is_some());
        assert!(registry.language("text").is_some());
        assert!(registry.language("unknown").is_none());

        #[cfg(feature = "tree-sitter-rust")]
        {
            assert!(registry.language("rust").is_some());
            assert!(registry.language("rs").is_some());
        }
        #[cfg(not(feature = "tree-sitter-rust"))]
        {
            assert!(registry.language("rust").is_none());
            assert!(registry.language("rs").is_none());
        }

        #[cfg(feature = "tree-sitter-javascript")]
        {
            assert!(registry.language("javascript").is_some());
            assert!(registry.language("js").is_some());
        }
        #[cfg(not(feature = "tree-sitter-javascript"))]
        {
            assert!(registry.language("javascript").is_none());
            assert!(registry.language("js").is_none());
        }
    }

    #[test]
    fn registry_resolves_registered_file_extensions() {
        let registry = LanguageRegistry::with_default_languages();
        registry.register_with_file_extensions(
            "__test_custom_language__",
            &LanguageConfig::new(
                "__test_custom_language__",
                tree_sitter_json::LANGUAGE.into(),
                vec![],
                "",
                "",
                "",
            ),
            &["custom_ext".to_string()],
        );

        assert_eq!(
            registry
                .language_name_for_extension("custom_ext")
                .as_deref(),
            Some("__test_custom_language__")
        );
        assert_eq!(
            registry
                .language_name_for_extension(".CUSTOM_EXT")
                .as_deref(),
            Some("__test_custom_language__")
        );
    }

    #[test]
    fn unregister_removes_registered_file_extensions() {
        let registry = LanguageRegistry::with_default_languages();
        registry.register_with_file_extensions(
            "__test_extension_cleanup__",
            &LanguageConfig::new(
                "__test_extension_cleanup__",
                tree_sitter_json::LANGUAGE.into(),
                vec![],
                "",
                "",
                "",
            ),
            &["cleanup_ext".to_string()],
        );

        assert!(registry.unregister("__test_extension_cleanup__"));
        assert_eq!(registry.language_name_for_extension("cleanup_ext"), None);
    }

    #[test]
    fn register_wasm_rejects_invalid_bytes() {
        let registry = LanguageRegistry::with_default_languages();
        let invalid = vec![0u8, 1, 2, 3, 4];
        let result =
            registry.register_wasm("__test_invalid_wasm__", invalid, &[], vec![], "", "", "");

        assert!(result.is_err(), "expected error for non-wasm bytes");
        assert!(
            !registry
                .languages()
                .iter()
                .any(|name| name.as_ref() == "__test_invalid_wasm__")
        );
    }

    #[test]
    fn unregister_removes_existing_entry() {
        let registry = LanguageRegistry::with_default_languages();
        registry.register(
            "__test_unregister__",
            &LanguageConfig::new(
                "__test_unregister__",
                tree_sitter_json::LANGUAGE.into(),
                vec![],
                "",
                "",
                "",
            ),
        );

        assert!(registry.unregister("__test_unregister__"));
        assert!(!registry.unregister("__test_unregister__"));
    }

    #[test]
    fn unregister_restores_builtin_language_after_override() {
        let registry = LanguageRegistry::with_default_languages();
        let builtin = registry.language("json").unwrap();
        registry.register(
            "json",
            &LanguageConfig::new(
                "json",
                tree_sitter_json::LANGUAGE.into(),
                vec![],
                "override",
                "",
                "",
            ),
        );
        assert_ne!(builtin, registry.language("json").unwrap());

        assert!(registry.unregister("json"));
        assert_eq!(builtin, registry.language("json").unwrap());
        assert!(!registry.unregister("json"));
        assert_eq!(builtin, registry.language("json").unwrap());
    }
}
