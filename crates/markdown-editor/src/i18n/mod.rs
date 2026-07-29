//! Localised UI strings and runtime language selection.
//!
//! This module owns language packs, system-locale matching, and the global
//! manager used by the embedded editor UI. Visual styling remains in `theme`.

use std::sync::Arc;

use anyhow::{Context as _, bail};
use gpui::{App, Global};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};

use crate::config::{
    VelotypeConfigDirs, object_without_empty_values, prune_empty_json_values, read_json_or_jsonc,
};

/// Localisable strings used by the embedded editor surface.
#[derive(Debug, Clone, Serialize)]
pub struct I18nStrings {
    pub info_dialog_ok: String,
    pub image_paste_failed_title: String,
    pub open_link_title: String,
    pub open_link_open: String,
    pub open_link_cancel: String,
    pub context_menu_insert: String,
    pub context_menu_table: String,
    pub table_axis_align_column_left: String,
    pub table_axis_align_column_center: String,
    pub table_axis_align_column_right: String,
    pub table_axis_move_column_left: String,
    pub table_axis_move_column_right: String,
    pub table_axis_delete_column: String,
    pub table_axis_move_row_up: String,
    pub table_axis_move_row_down: String,
    pub table_axis_delete_row: String,
    pub table_header_row: String,
    pub table_insert_title: String,
    pub table_insert_description: String,
    pub table_insert_body_rows: String,
    pub table_insert_columns: String,
    pub table_insert_cancel: String,
    pub table_insert_confirm: String,
    pub image_placeholder: String,
    pub image_loading_without_alt: String,
    pub image_loading_with_alt_template: String,
}

/// Partial language-pack strings. Missing values inherit from the built-in
/// language matching the pack id, or English for custom language ids.
#[derive(Debug, Default, Deserialize)]
struct I18nStringsDe {
    info_dialog_ok: Option<String>,
    image_paste_failed_title: Option<String>,
    open_link_title: Option<String>,
    open_link_open: Option<String>,
    open_link_cancel: Option<String>,
    context_menu_insert: Option<String>,
    context_menu_table: Option<String>,
    table_axis_align_column_left: Option<String>,
    table_axis_align_column_center: Option<String>,
    table_axis_align_column_right: Option<String>,
    table_axis_move_column_left: Option<String>,
    table_axis_move_column_right: Option<String>,
    table_axis_delete_column: Option<String>,
    table_axis_move_row_up: Option<String>,
    table_axis_move_row_down: Option<String>,
    table_axis_delete_row: Option<String>,
    table_header_row: Option<String>,
    table_insert_title: Option<String>,
    table_insert_description: Option<String>,
    table_insert_body_rows: Option<String>,
    table_insert_columns: Option<String>,
    table_insert_cancel: Option<String>,
    table_insert_confirm: Option<String>,
    image_placeholder: Option<String>,
    image_loading_without_alt: Option<String>,
    image_loading_with_alt_template: Option<String>,
}

const I18N_STRING_KEYS: &[&str] = &[
    "info_dialog_ok",
    "image_paste_failed_title",
    "open_link_title",
    "open_link_open",
    "open_link_cancel",
    "context_menu_insert",
    "context_menu_table",
    "table_axis_align_column_left",
    "table_axis_align_column_center",
    "table_axis_align_column_right",
    "table_axis_move_column_left",
    "table_axis_move_column_right",
    "table_axis_delete_column",
    "table_axis_move_row_up",
    "table_axis_move_row_down",
    "table_axis_delete_row",
    "table_header_row",
    "table_insert_title",
    "table_insert_description",
    "table_insert_body_rows",
    "table_insert_columns",
    "table_insert_cancel",
    "table_insert_confirm",
    "image_placeholder",
    "image_loading_without_alt",
    "image_loading_with_alt_template",
];

impl I18nStringsDe {
    fn into_strings(self, defaults: I18nStrings) -> I18nStrings {
        I18nStrings {
            info_dialog_ok: self.info_dialog_ok.unwrap_or(defaults.info_dialog_ok),
            image_paste_failed_title: self
                .image_paste_failed_title
                .unwrap_or(defaults.image_paste_failed_title),
            open_link_title: self.open_link_title.unwrap_or(defaults.open_link_title),
            open_link_open: self.open_link_open.unwrap_or(defaults.open_link_open),
            open_link_cancel: self.open_link_cancel.unwrap_or(defaults.open_link_cancel),
            context_menu_insert: self
                .context_menu_insert
                .unwrap_or(defaults.context_menu_insert),
            context_menu_table: self
                .context_menu_table
                .unwrap_or(defaults.context_menu_table),
            table_axis_align_column_left: self
                .table_axis_align_column_left
                .unwrap_or(defaults.table_axis_align_column_left),
            table_axis_align_column_center: self
                .table_axis_align_column_center
                .unwrap_or(defaults.table_axis_align_column_center),
            table_axis_align_column_right: self
                .table_axis_align_column_right
                .unwrap_or(defaults.table_axis_align_column_right),
            table_axis_move_column_left: self
                .table_axis_move_column_left
                .unwrap_or(defaults.table_axis_move_column_left),
            table_axis_move_column_right: self
                .table_axis_move_column_right
                .unwrap_or(defaults.table_axis_move_column_right),
            table_axis_delete_column: self
                .table_axis_delete_column
                .unwrap_or(defaults.table_axis_delete_column),
            table_axis_move_row_up: self
                .table_axis_move_row_up
                .unwrap_or(defaults.table_axis_move_row_up),
            table_axis_move_row_down: self
                .table_axis_move_row_down
                .unwrap_or(defaults.table_axis_move_row_down),
            table_axis_delete_row: self
                .table_axis_delete_row
                .unwrap_or(defaults.table_axis_delete_row),
            table_header_row: self.table_header_row.unwrap_or(defaults.table_header_row),
            table_insert_title: self
                .table_insert_title
                .unwrap_or(defaults.table_insert_title),
            table_insert_description: self
                .table_insert_description
                .unwrap_or(defaults.table_insert_description),
            table_insert_body_rows: self
                .table_insert_body_rows
                .unwrap_or(defaults.table_insert_body_rows),
            table_insert_columns: self
                .table_insert_columns
                .unwrap_or(defaults.table_insert_columns),
            table_insert_cancel: self
                .table_insert_cancel
                .unwrap_or(defaults.table_insert_cancel),
            table_insert_confirm: self
                .table_insert_confirm
                .unwrap_or(defaults.table_insert_confirm),
            image_placeholder: self.image_placeholder.unwrap_or(defaults.image_placeholder),
            image_loading_without_alt: self
                .image_loading_without_alt
                .unwrap_or(defaults.image_loading_without_alt),
            image_loading_with_alt_template: self
                .image_loading_with_alt_template
                .unwrap_or(defaults.image_loading_with_alt_template),
        }
    }
}

impl<'de> Deserialize<'de> for I18nStrings {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = I18nStringsDe::deserialize(deserializer)?;
        Ok(raw.into_strings(I18nStrings::en_us()))
    }
}

impl I18nStrings {
    /// Built-in Simplified Chinese editor strings.
    pub fn zh_cn() -> Self {
        Self {
            info_dialog_ok: "确定".into(),
            image_paste_failed_title: "图片粘贴失败".into(),
            open_link_title: "打开链接？".into(),
            open_link_open: "打开".into(),
            open_link_cancel: "取消".into(),
            context_menu_insert: "插入".into(),
            context_menu_table: "表格".into(),
            table_axis_align_column_left: "左对齐此列".into(),
            table_axis_align_column_center: "居中此列".into(),
            table_axis_align_column_right: "右对齐此列".into(),
            table_axis_move_column_left: "向左移动此列".into(),
            table_axis_move_column_right: "向右移动此列".into(),
            table_axis_delete_column: "删除此列".into(),
            table_axis_move_row_up: "向上移动此行".into(),
            table_axis_move_row_down: "向下移动此行".into(),
            table_axis_delete_row: "删除此行".into(),
            table_header_row: "标题行".into(),
            table_insert_title: "插入表格".into(),
            table_insert_description: "创建 1 个表头行，并配置正文行数与列数。".into(),
            table_insert_body_rows: "正文行数".into(),
            table_insert_columns: "列数".into(),
            table_insert_cancel: "取消".into(),
            table_insert_confirm: "插入".into(),
            image_placeholder: "图片".into(),
            image_loading_without_alt: "正在加载图片...".into(),
            image_loading_with_alt_template: "正在加载 {alt}".into(),
        }
    }

    /// Built-in English editor strings.
    pub fn en_us() -> Self {
        Self {
            info_dialog_ok: "OK".into(),
            image_paste_failed_title: "Image Paste Failed".into(),
            open_link_title: "Open link?".into(),
            open_link_open: "Open".into(),
            open_link_cancel: "Cancel".into(),
            context_menu_insert: "Insert".into(),
            context_menu_table: "Table".into(),
            table_axis_align_column_left: "Align Column Left".into(),
            table_axis_align_column_center: "Align Column Center".into(),
            table_axis_align_column_right: "Align Column Right".into(),
            table_axis_move_column_left: "Move Column Left".into(),
            table_axis_move_column_right: "Move Column Right".into(),
            table_axis_delete_column: "Delete Column".into(),
            table_axis_move_row_up: "Move Row Up".into(),
            table_axis_move_row_down: "Move Row Down".into(),
            table_axis_delete_row: "Delete Row".into(),
            table_header_row: "Header Row".into(),
            table_insert_title: "Insert Table".into(),
            table_insert_description: "Create one header row and configure body rows and columns."
                .into(),
            table_insert_body_rows: "Body Rows".into(),
            table_insert_columns: "Columns".into(),
            table_insert_cancel: "Cancel".into(),
            table_insert_confirm: "Insert".into(),
            image_placeholder: "Image".into(),
            image_loading_without_alt: "Loading image...".into(),
            image_loading_with_alt_template: "Loading {alt}".into(),
        }
    }

    /// Returns a built-in string set for a supported language id.
    pub fn for_language_id(language_id: &str) -> Option<Self> {
        match language_id {
            "zh-CN" => Some(Self::zh_cn()),
            "en-US" => Some(Self::en_us()),
            _ => None,
        }
    }
}

/// Metadata for a selectable UI language.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LanguageCatalogEntry {
    pub id: String,
    pub name: String,
}

const BUILTIN_LANGUAGE_ZH_CN_ID: &str = "zh-CN";
const BUILTIN_LANGUAGE_ZH_CN_NAME: &str = "简体中文";
const BUILTIN_LANGUAGE_EN_US_ID: &str = "en-US";
const BUILTIN_LANGUAGE_EN_US_NAME: &str = "English";

fn builtin_language_catalog() -> Vec<LanguageCatalogEntry> {
    vec![
        LanguageCatalogEntry {
            id: BUILTIN_LANGUAGE_ZH_CN_ID.into(),
            name: BUILTIN_LANGUAGE_ZH_CN_NAME.into(),
        },
        LanguageCatalogEntry {
            id: BUILTIN_LANGUAGE_EN_US_ID.into(),
            name: BUILTIN_LANGUAGE_EN_US_NAME.into(),
        },
    ]
}

/// A JSON language pack with metadata and fallback-completed strings.
#[derive(Debug, Clone, Serialize)]
#[allow(dead_code)]
pub struct I18nLanguagePack {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    pub strings: I18nStrings,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct I18nLanguagePackDe {
    id: String,
    name: Option<String>,
    author: Option<String>,
    description: Option<String>,
    version: Option<String>,
    homepage: Option<String>,
    license: Option<String>,
    #[serde(default)]
    strings: I18nStringsDe,
}

#[allow(dead_code)]
impl I18nLanguagePack {
    /// Parses a language pack from JSON text.
    pub fn from_json(json: &str) -> anyhow::Result<Self> {
        let mut value: Value = serde_json::from_str(json)?;
        prune_empty_json_values(&mut value);
        Self::from_value(value)
    }

    fn from_value(value: Value) -> anyhow::Result<Self> {
        let raw: I18nLanguagePackDe = serde_json::from_value(value)?;
        Ok(Self::from_partial(raw))
    }

    fn from_partial(raw: I18nLanguagePackDe) -> Self {
        let fallback = I18nStrings::for_language_id(&raw.id).unwrap_or_else(I18nStrings::en_us);
        let name = raw
            .name
            .unwrap_or_else(|| language_name_for_id(&raw.id).unwrap_or(&raw.id).to_string());
        Self {
            id: raw.id,
            name,
            author: raw.author,
            description: raw.description,
            version: raw.version,
            homepage: raw.homepage,
            license: raw.license,
            strings: raw.strings.into_strings(fallback),
        }
    }
}

fn language_name_for_id(language_id: &str) -> Option<&'static str> {
    match language_id {
        BUILTIN_LANGUAGE_ZH_CN_ID => Some(BUILTIN_LANGUAGE_ZH_CN_NAME),
        BUILTIN_LANGUAGE_EN_US_ID => Some(BUILTIN_LANGUAGE_EN_US_NAME),
        _ => None,
    }
}

fn is_builtin_language_id(language_id: &str) -> bool {
    matches!(
        language_id,
        BUILTIN_LANGUAGE_ZH_CN_ID | BUILTIN_LANGUAGE_EN_US_ID
    )
}

fn is_valid_custom_language_id(language_id: &str) -> bool {
    !language_id.trim().is_empty()
        && language_id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
        && language_id.chars().any(|ch| ch.is_ascii_alphabetic())
}

/// Selects a built-in language id from preferred system locales.
#[cfg(test)]
pub fn language_id_for_locale_preferences<I, S>(locales: I) -> &'static str
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    locales
        .into_iter()
        .find_map(|locale| language_id_for_locale(locale.as_ref()))
        .unwrap_or(BUILTIN_LANGUAGE_EN_US_ID)
}

#[cfg(test)]
fn language_id_for_locale(locale: &str) -> Option<&'static str> {
    let locale = locale.trim();
    if locale.is_empty() {
        return None;
    }

    let no_encoding = locale
        .split_once('.')
        .map_or(locale, |(locale, _encoding)| locale);
    let no_modifier = no_encoding
        .split_once('@')
        .map_or(no_encoding, |(locale, _modifier)| locale);
    let locale = no_modifier.replace('_', "-");
    let language = locale.split('-').next()?.to_ascii_lowercase();
    if !language.chars().all(|ch| ch.is_ascii_alphabetic()) {
        return None;
    }

    match language.as_str() {
        "zh" => Some(BUILTIN_LANGUAGE_ZH_CN_ID),
        "en" => Some(BUILTIN_LANGUAGE_EN_US_ID),
        _ => None,
    }
}

/// Global singleton that holds the current UI language strings.
pub struct I18nManager {
    current_language_id: String,
    strings: Arc<I18nStrings>,
    custom_languages: Vec<I18nLanguagePack>,
    language_catalog: Vec<LanguageCatalogEntry>,
}

impl Global for I18nManager {}

impl Default for I18nManager {
    fn default() -> Self {
        Self::new_with_language_id(BUILTIN_LANGUAGE_EN_US_ID)
    }
}

impl I18nManager {
    /// Installs the configured UI language into GPUI's global state.
    #[allow(dead_code)]
    pub fn init(cx: &mut App) {
        let language_id = crate::config::read_app_preferences()
            .map(|preferences| preferences.default_language_id)
            .unwrap_or_else(|_| BUILTIN_LANGUAGE_EN_US_ID.into());
        Self::init_with_language_id(cx, &language_id);
    }

    /// Installs a specific UI language into GPUI's global state.
    pub fn init_with_language_id(cx: &mut App, language_id: &str) {
        let mut manager = Self::new_with_language_id(BUILTIN_LANGUAGE_EN_US_ID);
        if let Ok(dirs) = VelotypeConfigDirs::from_system()
            && let Err(err) = manager.load_custom_languages_from_dirs(&dirs)
        {
            eprintln!("failed to load custom languages: {err}");
        }
        let _ = manager.set_language_by_id(language_id);
        cx.set_global(manager);
    }

    /// Creates a manager with a known language id, falling back to English.
    pub fn new_with_language_id(language_id: &str) -> Self {
        let current_language_id = if I18nStrings::for_language_id(language_id).is_some() {
            language_id
        } else {
            BUILTIN_LANGUAGE_EN_US_ID
        };
        Self {
            current_language_id: current_language_id.into(),
            strings: Arc::new(
                I18nStrings::for_language_id(current_language_id)
                    .unwrap_or_else(I18nStrings::en_us),
            ),
            custom_languages: Vec::new(),
            language_catalog: builtin_language_catalog(),
        }
    }

    /// Returns the identifier of the currently active UI language.
    #[cfg(test)]
    pub fn current_language_id(&self) -> &str {
        &self.current_language_id
    }

    /// Returns the strings for the currently active UI language.
    pub fn strings(&self) -> &I18nStrings {
        &self.strings
    }

    /// Returns an `Arc` clone of the currently active strings — O(1), no
    /// per-field copy. Use this in hot render paths instead of cloning the
    /// whole string catalog.
    pub fn strings_arc(&self) -> Arc<I18nStrings> {
        self.strings.clone()
    }

    /// Returns all built-in and imported UI languages.
    #[cfg(test)]
    pub fn available_languages(&self) -> &[LanguageCatalogEntry] {
        &self.language_catalog
    }

    /// Activates a UI language by identifier.
    pub fn set_language_by_id(&mut self, language_id: &str) -> bool {
        let strings = if let Some(strings) = I18nStrings::for_language_id(language_id) {
            strings
        } else if let Some(pack) = self
            .custom_languages
            .iter()
            .find(|pack| pack.id == language_id)
        {
            pack.strings.clone()
        } else {
            return false;
        };
        let changed = self.current_language_id != language_id;
        self.current_language_id = language_id.into();
        self.strings = Arc::new(strings);
        changed
    }

    fn load_custom_languages_from_dirs(&mut self, dirs: &VelotypeConfigDirs) -> anyhow::Result<()> {
        let languages_dir = dirs.languages_dir();
        if !languages_dir.exists() {
            return Ok(());
        }

        let mut loaded = Vec::new();
        for entry in std::fs::read_dir(&languages_dir)? {
            let path = entry?.path();
            if path.is_file() {
                match read_json_or_jsonc(&path).and_then(|value| {
                    custom_language_pack_from_value(value).map(|(pack, _normalized)| pack)
                }) {
                    Ok(pack) => loaded.push(pack),
                    Err(err) => eprintln!(
                        "skipping custom language config '{}': {err}",
                        path.display()
                    ),
                }
            }
        }
        loaded.sort_by(|left, right| left.name.cmp(&right.name).then(left.id.cmp(&right.id)));
        for pack in loaded {
            self.upsert_custom_language(pack);
        }
        Ok(())
    }

    fn upsert_custom_language(&mut self, pack: I18nLanguagePack) {
        if let Some(existing) = self
            .custom_languages
            .iter_mut()
            .find(|existing| existing.id == pack.id)
        {
            *existing = pack;
        } else {
            self.custom_languages.push(pack);
        }
        self.rebuild_language_catalog();
    }

    fn rebuild_language_catalog(&mut self) {
        let mut catalog = builtin_language_catalog();
        catalog.extend(
            self.custom_languages
                .iter()
                .map(|pack| LanguageCatalogEntry {
                    id: pack.id.clone(),
                    name: pack.name.clone(),
                }),
        );
        self.language_catalog = catalog;
    }
}

fn custom_language_pack_from_value(mut value: Value) -> anyhow::Result<(I18nLanguagePack, Value)> {
    prune_empty_json_values(&mut value);
    let Value::Object(object) = value else {
        bail!("language config must be a JSON object");
    };
    let object = object_without_empty_values(object);
    let id = required_string(&object, "id")?;
    if is_builtin_language_id(&id) {
        bail!("custom language id '{id}' would override a built-in language");
    }
    if !is_valid_custom_language_id(&id) {
        bail!("custom language id '{id}' contains unsupported characters");
    }
    let name = required_string(&object, "name")?;
    let mut normalized_object = Map::new();
    normalized_object.insert("id".into(), Value::String(id.clone()));
    normalized_object.insert("name".into(), Value::String(name));
    for key in ["author", "description", "version", "homepage", "license"] {
        if let Some(value) = object.get(key) {
            normalized_object.insert(key.into(), value.clone());
        }
    }
    if let Some(strings) = object.get("strings").and_then(Value::as_object) {
        let mut normalized_strings = Map::new();
        for key in I18N_STRING_KEYS {
            if let Some(value) = strings.get(*key) {
                normalized_strings.insert((*key).into(), value.clone());
            }
        }
        if !normalized_strings.is_empty() {
            normalized_object.insert("strings".into(), Value::Object(normalized_strings));
        }
    }
    let normalized = Value::Object(normalized_object);
    let pack = I18nLanguagePack::from_value(normalized.clone())
        .with_context(|| format!("failed to parse language config '{id}'"))?;
    Ok((pack, normalized))
}

fn required_string(object: &Map<String, Value>, key: &str) -> anyhow::Result<String> {
    let Some(value) = object.get(key) else {
        bail!("missing required field '{key}'");
    };
    let Some(text) = value
        .as_str()
        .map(str::trim)
        .filter(|text| !text.is_empty())
    else {
        bail!("field '{key}' must be a non-empty string");
    };
    Ok(text.to_string())
}

#[cfg(test)]
mod tests {
    use super::{I18nLanguagePack, I18nManager, I18nStrings, language_id_for_locale_preferences};
    use crate::theme::ThemeManager;

    #[test]
    fn built_in_chinese_strings_cover_embedded_editor_ui() {
        let strings = I18nStrings::zh_cn();
        assert_eq!(strings.info_dialog_ok, "确定");
        assert_eq!(strings.image_paste_failed_title, "图片粘贴失败");
        assert_eq!(strings.open_link_title, "打开链接？");
        assert_eq!(strings.context_menu_insert, "插入");
        assert_eq!(strings.table_insert_title, "插入表格");
        assert_eq!(strings.image_loading_without_alt, "正在加载图片...");
        assert_eq!(strings.image_loading_with_alt_template, "正在加载 {alt}");
    }

    #[test]
    fn manager_switches_builtin_languages() {
        let mut manager = I18nManager::default();
        assert_eq!(manager.current_language_id(), "en-US");
        assert_eq!(manager.strings().context_menu_insert, "Insert");
        assert_eq!(manager.strings().table_insert_title, "Insert Table");

        assert!(manager.set_language_by_id("zh-CN"));
        assert_eq!(manager.current_language_id(), "zh-CN");
        assert_eq!(manager.strings().context_menu_insert, "插入");
        assert_eq!(manager.strings().table_insert_title, "插入表格");
        assert!(!manager.set_language_by_id("zh-CN"));
        assert!(!manager.set_language_by_id("missing"));
    }

    #[test]
    fn language_catalog_contains_chinese_and_english() {
        let manager = I18nManager::default();
        let ids = manager
            .available_languages()
            .iter()
            .map(|entry| (entry.id.as_str(), entry.name.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(ids, vec![("zh-CN", "简体中文"), ("en-US", "English")]);
    }

    #[test]
    fn manager_can_be_constructed_with_known_language() {
        let manager = I18nManager::new_with_language_id("zh-CN");
        assert_eq!(manager.current_language_id(), "zh-CN");
        assert_eq!(manager.strings().context_menu_table, "表格");

        let fallback = I18nManager::new_with_language_id("missing");
        assert_eq!(fallback.current_language_id(), "en-US");
        assert_eq!(fallback.strings().context_menu_table, "Table");
    }

    #[test]
    fn theme_switch_does_not_modify_selected_language() {
        let mut theme_manager = ThemeManager::default();
        let mut i18n_manager = I18nManager::new_with_language_id("zh-CN");

        assert!(theme_manager.set_theme_by_id("velotype"));
        assert!(!i18n_manager.set_language_by_id("missing"));

        assert_eq!(theme_manager.current_theme_id(), "velotype");
        assert_eq!(i18n_manager.current_language_id(), "zh-CN");
        assert_eq!(i18n_manager.strings().context_menu_insert, "插入");
    }

    #[test]
    fn locale_preferences_map_to_builtin_languages() {
        assert_eq!(language_id_for_locale_preferences(["zh-CN"]), "zh-CN");
        assert_eq!(language_id_for_locale_preferences(["zh-HK"]), "zh-CN");
        assert_eq!(language_id_for_locale_preferences(["zh-Hant-TW"]), "zh-CN");
        assert_eq!(language_id_for_locale_preferences(["zh_SG.UTF-8"]), "zh-CN");
        assert_eq!(language_id_for_locale_preferences(["en-US"]), "en-US");
        assert_eq!(language_id_for_locale_preferences(["en_GB.UTF-8"]), "en-US");
        assert_eq!(
            language_id_for_locale_preferences(["fr-FR", "zh-CN"]),
            "zh-CN"
        );
        assert_eq!(
            language_id_for_locale_preferences(Vec::<&str>::new()),
            "en-US"
        );
        assert_eq!(language_id_for_locale_preferences(["fr-FR"]), "en-US");
        assert_eq!(language_id_for_locale_preferences(["!!!"]), "en-US");
    }

    #[test]
    fn language_pack_json_falls_back_for_missing_strings() {
        let pack = I18nLanguagePack::from_json(
            r#"{
                "id": "zh-CN",
                "name": "简体中文",
                "strings": {
                    "context_menu_insert": "新增",
                    "table_insert_title": "新建表格",
                    "unknown_field": "ignored"
                }
            }"#,
        )
        .expect("language pack should load");

        assert_eq!(pack.id, "zh-CN");
        assert_eq!(pack.name, "简体中文");
        assert_eq!(pack.strings.context_menu_insert, "新增");
        assert_eq!(pack.strings.table_insert_title, "新建表格");
        assert_eq!(pack.strings.info_dialog_ok, "确定");
        assert_eq!(pack.strings.open_link_cancel, "取消");
        assert_eq!(pack.strings.image_placeholder, "图片");
    }

    #[test]
    fn unknown_language_pack_falls_back_to_english_strings() {
        let pack = I18nLanguagePack::from_json(
            r#"{
                "id": "fr-FR",
                "strings": {
                    "context_menu_insert": "Insérer"
                }
            }"#,
        )
        .expect("language pack should load");

        assert_eq!(pack.id, "fr-FR");
        assert_eq!(pack.name, "fr-FR");
        assert_eq!(pack.strings.context_menu_insert, "Insérer");
        assert_eq!(pack.strings.context_menu_table, "Table");
        assert_eq!(pack.strings.info_dialog_ok, "OK");
        assert_eq!(pack.strings.open_link_open, "Open");
        assert_eq!(pack.strings.image_placeholder, "Image");
    }
}
