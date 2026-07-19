use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use cditor_app::{
    SyntaxHighlightError, SyntaxHighlightLanguage, SyntaxHighlightPalette, SyntaxHighlightProvider,
    SyntaxHighlightRun, SyntaxHighlightStyle,
};
use gpui::{FontStyle, FontWeight, HighlightStyle, Hsla, Rgba};
use gpui_component::highlighter::{
    HighlightTheme, LanguageConfig, LanguageRegistry, SyntaxHighlighter,
};
use ropey::Rope;

struct ProviderState {
    theme: Arc<HighlightTheme>,
    palette: SyntaxHighlightPalette,
}

struct SharedHighlighter {
    config: LanguageConfig,
    highlighter: Arc<Mutex<SyntaxHighlighter>>,
}

pub(crate) struct NavopSyntaxHighlightProvider {
    state: RwLock<ProviderState>,
    theme_revision: AtomicU64,
    highlighters: Mutex<HashMap<String, SharedHighlighter>>,
}

impl NavopSyntaxHighlightProvider {
    pub(crate) fn new(theme: Arc<HighlightTheme>, background: Hsla, foreground: Hsla) -> Self {
        Self {
            state: RwLock::new(ProviderState {
                theme,
                palette: palette(background, foreground),
            }),
            theme_revision: AtomicU64::new(0),
            highlighters: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) fn refresh_theme(
        &self,
        theme: Arc<HighlightTheme>,
        background: Hsla,
        foreground: Hsla,
    ) {
        let next_palette = palette(background, foreground);
        let mut state = self.state.write().expect("syntax highlight state poisoned");
        if state.theme == theme && state.palette == next_palette {
            return;
        }
        state.theme = theme;
        state.palette = next_palette;
        self.theme_revision.fetch_add(1, Ordering::AcqRel);
    }
}

impl SyntaxHighlightProvider for NavopSyntaxHighlightProvider {
    fn id(&self) -> &str {
        "navop-tree-sitter"
    }

    fn revision(&self) -> u64 {
        let theme = self.theme_revision.load(Ordering::Acquire);
        let registry = LanguageRegistry::singleton().revision();
        theme.wrapping_add(registry.rotate_left(32))
    }

    fn palette(&self) -> SyntaxHighlightPalette {
        self.state
            .read()
            .expect("syntax highlight state poisoned")
            .palette
    }

    fn languages(&self) -> Vec<SyntaxHighlightLanguage> {
        LanguageRegistry::singleton()
            .languages()
            .into_iter()
            .map(|language| {
                let id = language.to_string();
                SyntaxHighlightLanguage::new(&id, language_label(&id))
            })
            .collect()
    }

    fn highlight(
        &self,
        language: &str,
        source: &str,
    ) -> Result<Vec<SyntaxHighlightRun>, SyntaxHighlightError> {
        let registry = LanguageRegistry::singleton();
        let Some(config) = registry.language(language) else {
            return Err(SyntaxHighlightError::new(format!(
                "language {language:?} is not registered"
            )));
        };
        let text = Rope::from_str(source);
        let highlighter = {
            let mut highlighters = self
                .highlighters
                .lock()
                .expect("syntax highlighter pool poisoned");
            let entry =
                highlighters
                    .entry(language.to_owned())
                    .or_insert_with(|| SharedHighlighter {
                        config: config.clone(),
                        highlighter: Arc::new(Mutex::new(SyntaxHighlighter::new(language))),
                    });
            if entry.config != config {
                *entry = SharedHighlighter {
                    config,
                    highlighter: Arc::new(Mutex::new(SyntaxHighlighter::new(language))),
                };
            }
            entry.highlighter.clone()
        };
        let mut highlighter = highlighter
            .lock()
            .expect("shared syntax highlighter poisoned");
        if !highlighter.update(None, &text, None) {
            return Err(SyntaxHighlightError::new(format!(
                "tree-sitter parse failed for {language:?}"
            )));
        }
        let theme = self
            .state
            .read()
            .expect("syntax highlight state poisoned")
            .theme
            .clone();
        Ok(highlighter
            .styles(&(0..source.len()), &theme)
            .into_iter()
            .map(|(range, style)| SyntaxHighlightRun {
                range,
                style: convert_style(style),
            })
            .collect())
    }
}

fn palette(background: Hsla, foreground: Hsla) -> SyntaxHighlightPalette {
    SyntaxHighlightPalette {
        background: rgb24(background),
        foreground: rgb24(foreground),
    }
}

fn language_label(id: &str) -> String {
    match id {
        "csharp" => "C#".to_owned(),
        "css" => "CSS".to_owned(),
        "html" => "HTML".to_owned(),
        "javascript" => "JavaScript".to_owned(),
        "json" => "JSON".to_owned(),
        "sql" => "SQL".to_owned(),
        "tsx" => "TSX".to_owned(),
        "typescript" => "TypeScript".to_owned(),
        "yaml" => "YAML".to_owned(),
        other => other
            .split(['-', '_'])
            .filter(|part| !part.is_empty())
            .map(capitalize)
            .collect::<Vec<_>>()
            .join(" "),
    }
}

fn capitalize(value: &str) -> String {
    let mut characters = value.chars();
    let Some(first) = characters.next() else {
        return String::new();
    };
    first.to_uppercase().chain(characters).collect()
}

fn convert_style(style: HighlightStyle) -> SyntaxHighlightStyle {
    SyntaxHighlightStyle {
        foreground: style.color.map(rgb24),
        background: style.background_color.map(rgb24),
        bold: style
            .font_weight
            .is_some_and(|weight| weight >= FontWeight::BOLD),
        italic: matches!(
            style.font_style,
            Some(FontStyle::Italic | FontStyle::Oblique)
        ),
        underline: style.underline.is_some(),
        strikethrough: style.strikethrough.is_some(),
    }
}

fn rgb24(color: Hsla) -> u32 {
    let color = Rgba::from(color);
    let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u32;
    (channel(color.r) << 16) | (channel(color.g) << 8) | channel(color.b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::rgb;

    #[test]
    fn color_conversion_discards_alpha_without_reordering_rgb() {
        assert_eq!(rgb24(Hsla::from(rgb(0x123456))), 0x123456);
    }

    #[test]
    fn provider_highlights_registered_markdown_without_changing_ranges() {
        let provider = NavopSyntaxHighlightProvider::new(
            HighlightTheme::default_light(),
            Hsla::from(rgb(0xffffff)),
            Hsla::from(rgb(0x111111)),
        );
        let source = "**bold**";
        let runs = provider
            .highlight("markdown", source)
            .expect("markdown is registered by the notes feature");
        assert!(runs.iter().all(|run| run.range.end <= source.len()));
        assert!(!runs.is_empty());
        assert!(
            provider
                .languages()
                .iter()
                .any(|language| { language.id == "markdown" && language.label == "Markdown" })
        );
    }

    #[test]
    fn provider_reuses_one_highlighter_per_language() {
        let provider = NavopSyntaxHighlightProvider::new(
            HighlightTheme::default_light(),
            Hsla::from(rgb(0xffffff)),
            Hsla::from(rgb(0x111111)),
        );
        provider.highlight("markdown", "# First").unwrap();
        let first = {
            let highlighters = provider.highlighters.lock().unwrap();
            highlighters
                .get("markdown")
                .map(|entry| Arc::as_ptr(&entry.highlighter))
                .unwrap()
        };

        let second_source = "## Second\n\n`code`";
        provider.highlight("markdown", second_source).unwrap();
        let highlighters = provider.highlighters.lock().unwrap();
        let entry = highlighters.get("markdown").unwrap();
        assert_eq!(first, Arc::as_ptr(&entry.highlighter));
        assert_eq!(
            entry.highlighter.lock().unwrap().text().to_string(),
            second_source
        );
    }
}
