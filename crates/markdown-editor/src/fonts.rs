//! User-configurable system font stacks shared by editor rendering paths.
//!
//! Ported from velotype `src/fonts.rs` (#121). navop's fork does not ship the
//! upstream export pipeline or preferences window, so this module keeps the
//! native resolution path and exposes the resolved stacks as GPUI globals that
//! the host can configure through [`set_font_stacks`](crate::set_font_stacks).

use gpui::{App, Font, FontFallbacks, Global, font};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// GPUI's platform system-font alias.
pub const SYSTEM_UI_FONT: &str = ".SystemUIFont";
/// Default family list for document text.
pub const DEFAULT_BODY_STACK: &str = ".SystemUIFont";
/// Default family list for application controls.
pub const DEFAULT_UI_STACK: &str = ".SystemUIFont";
/// Cross-platform default family list for code text.
pub const DEFAULT_CODE_STACK: &str =
    "SFMono-Regular, Consolas, Liberation Mono, Menlo, monospace";

/// Case-insensitive lookup of available font families with their original spelling.
#[derive(Clone, Debug)]
pub struct FontCatalog {
    /// Lowercased family names mapped to canonical names, shared across clones.
    families: Arc<HashMap<String, String>>,
}

impl Global for FontCatalog {}

impl FontCatalog {
    /// Indexes families, retaining the first spelling and always adding the system alias.
    pub fn from_names(names: impl IntoIterator<Item = String>) -> Self {
        let mut families = HashMap::new();
        for name in names {
            families.entry(name.to_lowercase()).or_insert(name);
        }
        families
            .entry(SYSTEM_UI_FONT.to_lowercase())
            .or_insert_with(|| SYSTEM_UI_FONT.to_string());
        Self {
            families: Arc::new(families),
        }
    }

    /// Looks up an available family's canonical spelling without regard to case.
    fn canonical_name(&self, family: &str) -> Option<&str> {
        self.families
            .get(&family.to_lowercase())
            .map(String::as_str)
    }

    /// Clones the installed catalog, or returns one containing only the system alias.
    pub fn current(cx: &App) -> Self {
        cx.try_global::<Self>()
            .cloned()
            .unwrap_or_else(|| Self::from_names(std::iter::once(SYSTEM_UI_FONT.to_string())))
    }
}

/// Editable family lists persisted by the host before local resolution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FontPreferences {
    /// Family list for document body text.
    pub body_stack: String,
    /// Family list for inline and block code.
    pub code_stack: String,
    /// Family list for application controls.
    pub ui_stack: String,
}

impl Default for FontPreferences {
    /// Uses system fonts for body/UI text and the cross-platform code stack.
    fn default() -> Self {
        Self {
            body_stack: DEFAULT_BODY_STACK.into(),
            code_stack: DEFAULT_CODE_STACK.into(),
            ui_stack: DEFAULT_UI_STACK.into(),
        }
    }
}

impl FontPreferences {
    /// Builds a preference set from three user-editable family lists.
    pub fn new(
        body_stack: impl Into<String>,
        code_stack: impl Into<String>,
        ui_stack: impl Into<String>,
    ) -> Self {
        Self {
            body_stack: body_stack.into(),
            code_stack: code_stack.into(),
            ui_stack: ui_stack.into(),
        }
    }
}

/// Syntax errors that prevent a family list from being resolved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FontStackParseError {
    /// No nonblank family remains after parsing.
    Empty,
    /// A single- or double-quoted family has no closing quote.
    UnterminatedQuote,
}

impl std::fmt::Display for FontStackParseError {
    /// Formats a short English validation message.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => write!(f, "enter at least one font family"),
            Self::UnterminatedQuote => write!(f, "font family has an unterminated quote"),
        }
    }
}

impl std::error::Error for FontStackParseError {}

/// Parses a CSS-like comma-separated family list. Quotes group a family but
/// are not retained; duplicate matching is case-insensitive.
pub fn parse_font_stack(input: &str) -> Result<Vec<String>, FontStackParseError> {
    let mut families = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut escaped = false;

    for ch in input.chars() {
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }
        if quote.is_some() && ch == '\\' {
            escaped = true;
            continue;
        }
        if matches!(ch, '\'' | '"') {
            match quote {
                Some(open) if open == ch => quote = None,
                None => quote = Some(ch),
                Some(_) => current.push(ch),
            }
            continue;
        }
        if ch == ',' && quote.is_none() {
            push_family(&mut families, &mut current);
        } else {
            current.push(ch);
        }
    }
    if quote.is_some() {
        return Err(FontStackParseError::UnterminatedQuote);
    }
    if escaped {
        current.push('\\');
    }
    push_family(&mut families, &mut current);

    let mut seen = HashSet::new();
    families.retain(|family| seen.insert(family.to_lowercase()));
    if families.is_empty() {
        Err(FontStackParseError::Empty)
    } else {
        Ok(families)
    }
}

/// Appends a trimmed nonblank family and clears the parsing buffer.
fn push_family(families: &mut Vec<String>, current: &mut String) {
    let family = current.trim();
    if !family.is_empty() {
        families.push(family.to_string());
    }
    current.clear();
}

/// Maps supported generic families to ordered native candidates for an OS.
fn generic_families(name: &str, target_os: &str) -> Option<&'static [&'static str]> {
    match (name.to_ascii_lowercase().as_str(), target_os) {
        ("sans-serif", "windows") => Some(&["Segoe UI", "Arial"]),
        ("sans-serif", "macos") => Some(&["Helvetica Neue", "Helvetica"]),
        ("sans-serif", _) => Some(&["Noto Sans", "DejaVu Sans", "Liberation Sans"]),
        ("serif", "windows") => Some(&["Times New Roman", "Georgia"]),
        ("serif", "macos") => Some(&["Times", "Georgia"]),
        ("serif", _) => Some(&["Noto Serif", "DejaVu Serif", "Liberation Serif"]),
        ("monospace", "windows") => Some(&["Cascadia Mono", "Consolas", "Courier New"]),
        ("monospace", "macos") => Some(&["SFMono-Regular", "Menlo", "Monaco"]),
        ("monospace", _) => Some(&["Noto Sans Mono", "DejaVu Sans Mono", "Liberation Mono"]),
        _ => None,
    }
}

/// Returns Tibetan fallback candidates in platform-preferred order.
fn tibetan_font_families(target_os: &str) -> &'static [&'static str] {
    match target_os {
        "windows" => &[
            "Microsoft Himalaya",
            "Noto Serif Tibetan",
            "Noto Sans Tibetan",
            "BabelStone Tibetan",
        ],
        "macos" => &["Kailasa", "Noto Serif Tibetan", "Noto Sans Tibetan"],
        _ => &[
            "Noto Serif Tibetan",
            "Noto Sans Tibetan",
            "Microsoft Himalaya",
            "Kailasa",
            "BabelStone Tibetan",
        ],
    }
}

/// Expands supported generic families and deduplicates candidates in encounter order.
fn expand_generics(families: &[String], target_os: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut seen = HashSet::new();
    for family in families {
        if let Some(expanded) = generic_families(family, target_os) {
            for candidate in expanded {
                if seen.insert(candidate.to_lowercase()) {
                    result.push((*candidate).to_string());
                }
            }
        } else if seen.insert(family.to_lowercase()) {
            result.push(family.clone());
        }
    }
    result
}

/// A native font with its fallback chain resolved against the installed catalog.
#[derive(Clone, Debug)]
pub struct ResolvedFontStack {
    #[cfg(test)]
    native_families: Vec<String>,
    /// True when no requested family or expanded generic candidate is available.
    /// In that case the native stack starts with the system UI font.
    pub uses_system_default: bool,
    /// Native font with its fallback chain constructed once during resolution.
    native_font: Font,
}

impl ResolvedFontStack {
    /// Resolves installed candidates, optional Tibetan fallbacks, and the system alias.
    fn resolve(
        input: &str,
        catalog: &FontCatalog,
        target_os: &str,
        tibetan: bool,
    ) -> Result<Self, FontStackParseError> {
        let requested = parse_font_stack(input)?;
        let expanded = expand_generics(&requested, target_os);
        let mut native_families = Vec::new();
        let mut has_available_requested_family = false;
        for family in &expanded {
            if let Some(canonical) = catalog.canonical_name(family) {
                push_unique(&mut native_families, canonical);
                has_available_requested_family = true;
            }
            // A missing entry is normal for a cross-platform font stack: it is
            // skipped for native rendering but still usable on other machines.
        }
        if !has_available_requested_family {
            push_unique(&mut native_families, SYSTEM_UI_FONT);
        }
        if tibetan {
            for family in tibetan_font_families(target_os) {
                if let Some(canonical) = catalog.canonical_name(family) {
                    push_unique(&mut native_families, canonical);
                }
            }
        }
        push_unique(&mut native_families, SYSTEM_UI_FONT);
        let native_font = native_font(&native_families);
        Ok(Self {
            #[cfg(test)]
            native_families,
            uses_system_default: !has_available_requested_family,
            native_font,
        })
    }

    /// Clones the cached native font, including its fallback list.
    pub fn font(&self) -> Font {
        self.native_font.clone()
    }
}

/// Uses the first family as primary and attaches remaining families as fallbacks.
fn native_font(families: &[String]) -> Font {
    let mut result = font(
        families
            .first()
            .cloned()
            .unwrap_or_else(|| SYSTEM_UI_FONT.into()),
    );
    if families.len() > 1 {
        result.fallbacks = Some(FontFallbacks::from_fonts(families[1..].to_vec()));
    }
    result
}

/// Appends a family only if no ASCII-case-insensitive match already exists.
fn push_unique(values: &mut Vec<String>, value: &str) {
    if !values
        .iter()
        .any(|existing| existing.eq_ignore_ascii_case(value))
    {
        values.push(value.to_string());
    }
}

/// Resolved body, code, and UI fonts shared through GPUI global state.
#[derive(Clone, Debug)]
pub struct FontSettings {
    /// Native families for document body text.
    pub body: ResolvedFontStack,
    /// Native families for code text.
    pub code: ResolvedFontStack,
    /// Native families for application controls.
    pub ui: ResolvedFontStack,
}

impl Global for FontSettings {}

impl FontSettings {
    /// Resolves all three stacks with Tibetan fallbacks, failing on invalid syntax.
    pub fn resolve(
        preferences: FontPreferences,
        catalog: &FontCatalog,
        target_os: &str,
    ) -> Result<Self, FontStackParseError> {
        Ok(Self {
            body: ResolvedFontStack::resolve(&preferences.body_stack, catalog, target_os, true)?,
            code: ResolvedFontStack::resolve(&preferences.code_stack, catalog, target_os, true)?,
            ui: ResolvedFontStack::resolve(&preferences.ui_stack, catalog, target_os, true)?,
        })
    }

    /// Installs the local catalog and resolved fonts, using defaults on a parse error.
    pub fn init(cx: &mut App, preferences: FontPreferences) {
        let catalog = FontCatalog::from_names(cx.text_system().all_font_names());
        let settings = Self::resolve(preferences, &catalog, std::env::consts::OS)
            .unwrap_or_else(|_| {
                Self::resolve(FontPreferences::default(), &catalog, std::env::consts::OS)
                    .expect("default font stacks are valid")
            });
        cx.set_global(catalog);
        cx.set_global(settings);
    }

    /// Replaces global fonts only after all supplied stacks resolve successfully.
    pub fn update(
        cx: &mut App,
        preferences: FontPreferences,
    ) -> Result<(), FontStackParseError> {
        let catalog = FontCatalog::current(cx);
        let settings = Self::resolve(preferences, &catalog, std::env::consts::OS)?;
        cx.set_global(settings);
        Ok(())
    }

    /// Clones global settings or resolves defaults against the current catalog.
    pub fn current(cx: &App) -> Self {
        cx.try_global::<Self>().cloned().unwrap_or_else(|| {
            Self::resolve(
                FontPreferences::default(),
                &FontCatalog::current(cx),
                std::env::consts::OS,
            )
            .expect("default font stacks are valid")
        })
    }

    /// Returns the current body font, resolving defaults if globals are absent.
    pub fn body_font(cx: &App) -> Font {
        Self::current_stack_font(cx, |settings| &settings.body)
    }

    /// Returns the current code font, resolving defaults if globals are absent.
    pub fn code_font(cx: &App) -> Font {
        Self::current_stack_font(cx, |settings| &settings.code)
    }

    /// Returns the current UI font, resolving defaults if globals are absent.
    pub fn ui_font(cx: &App) -> Font {
        Self::current_stack_font(cx, |settings| &settings.ui)
    }

    /// Selects a cached font directly from global settings, with a default fallback.
    fn current_stack_font(cx: &App, select: impl FnOnce(&Self) -> &ResolvedFontStack) -> Font {
        if let Some(settings) = cx.try_global::<Self>() {
            select(settings).font()
        } else {
            let fallback = Self::current(cx);
            select(&fallback).font()
        }
    }
}

/// Applies host-configured font stacks, keeping the previous settings on error.
///
/// The installed-family catalog is rebuilt on every call so fonts registered
/// after [`crate::init`] (host-imported custom fonts) resolve correctly.
pub fn set_font_stacks(
    preferences: FontPreferences,
    cx: &mut App,
) -> Result<(), FontStackParseError> {
    let catalog = FontCatalog::from_names(cx.text_system().all_font_names());
    let settings = FontSettings::resolve(preferences, &catalog, std::env::consts::OS)?;
    cx.set_global(catalog);
    cx.set_global(settings);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_quotes_whitespace_empty_items_and_duplicates() {
        assert_eq!(
            parse_font_stack("  'Noto Sans', Arial, , \"Noto Sans\", serif").unwrap(),
            vec!["Noto Sans", "Arial", "serif"]
        );
    }

    #[test]
    fn rejects_empty_and_unterminated_stacks() {
        assert_eq!(parse_font_stack(" ,  "), Err(FontStackParseError::Empty));
        assert_eq!(
            parse_font_stack("\"Noto Sans"),
            Err(FontStackParseError::UnterminatedQuote)
        );
    }

    #[test]
    fn resolves_generics_and_only_uses_system_default_when_none_are_available() {
        let catalog = FontCatalog::from_names(vec![
            "Segoe UI".into(),
            "Arial".into(),
            "Microsoft Himalaya".into(),
            SYSTEM_UI_FONT.into(),
        ]);
        let stack =
            ResolvedFontStack::resolve("Missing, sans-serif", &catalog, "windows", true).unwrap();
        assert!(!stack.uses_system_default);
        assert_eq!(&stack.native_families[..2], &["Segoe UI", "Arial"]);
        assert_eq!(stack.native_families.last().unwrap(), SYSTEM_UI_FONT);

        let missing_only =
            ResolvedFontStack::resolve("Missing", &catalog, "windows", true).unwrap();
        assert!(missing_only.uses_system_default);
        assert_eq!(missing_only.native_families.first().unwrap(), SYSTEM_UI_FONT);
        assert_eq!(
            missing_only.native_families.get(1).unwrap(),
            "Microsoft Himalaya"
        );
    }

    #[test]
    fn catalog_restores_the_installed_fonts_canonical_name() {
        let stack = ResolvedFontStack::resolve(
            "iNtEr",
            &FontCatalog::from_names(vec!["Inter".into()]),
            "linux",
            false,
        )
        .unwrap();
        assert_eq!(
            stack.native_families.first().map(String::as_str),
            Some("Inter")
        );
        assert_eq!(stack.font().family.to_string(), "Inter");
    }
}
