use gpui::{
    App, AppContext, Context, Entity, EventEmitter, IntoElement, Render, Styled as _, Subscription,
    Window,
};
use gpui_component::highlighter::Language;
use gpui_component::input::{Input, InputEvent, InputState, TabSize};
use gpui_component::tab::{Tab, TabBar};
use gpui_component::v_flex;
use tracing::error;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EditorTab {
    Text,
    Json,
}

impl EditorTab {
    pub fn language(&self) -> Language {
        match self {
            EditorTab::Text => Language::Plain,
            EditorTab::Json => Language::Json,
        }
    }
}

fn active_editor_text(active_tab: EditorTab, text_content: &str, json_content: &str) -> String {
    match active_tab {
        EditorTab::Text => text_content.to_string(),
        EditorTab::Json => json_content.to_string(),
    }
}

fn normalize_commit_text(active_tab: EditorTab, raw_text: &str) -> Result<String, json5::Error> {
    if active_tab == EditorTab::Json {
        return json5::from_str::<serde_json::Value>(raw_text).map(|value| value.to_string());
    }

    match json5::from_str::<serde_json::Value>(raw_text) {
        Ok(value) => Ok(value.to_string()),
        Err(_) => Ok(raw_text.to_string()),
    }
}

#[derive(Clone, Debug)]
pub enum MultiTextEditorEvent {
    ActiveEditorBlurred(String),
}

impl EventEmitter<MultiTextEditorEvent> for MultiTextEditor {}

pub struct MultiTextEditor {
    active_tab: EditorTab,
    text_editor: Entity<InputState>,
    json_editor: Entity<InputState>,
    has_user_edits: bool,
    suppress_edit_tracking: bool,
    _subs: Vec<Subscription>,
}

impl MultiTextEditor {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let text_editor = cx.new(|cx| {
            InputState::new(window, cx)
                .code_editor(EditorTab::Text.language())
                .line_number(true)
                .searchable(true)
                .indent_guides(true)
                .tab_size(TabSize {
                    tab_size: 2,
                    hard_tabs: false,
                })
                .soft_wrap(false)
                .placeholder("Enter your text here...")
        });

        let json_editor = cx.new(|cx| {
            InputState::new(window, cx)
                .code_editor(EditorTab::Json.language())
                .line_number(true)
                .searchable(true)
                .indent_guides(true)
                .tab_size(TabSize {
                    tab_size: 2,
                    hard_tabs: false,
                })
                .soft_wrap(false)
                .placeholder("Enter JSON here...")
        });

        let mut this = Self {
            active_tab: EditorTab::Text,
            text_editor,
            json_editor,
            has_user_edits: false,
            suppress_edit_tracking: false,
            _subs: Vec::new(),
        };
        this._subs = vec![
            cx.subscribe_in(
                &this.text_editor,
                window,
                |this, _, event: &InputEvent, _window, cx| match event {
                    InputEvent::Change if !this.suppress_edit_tracking => {
                        this.has_user_edits = true;
                    }
                    InputEvent::Blur if this.active_tab == EditorTab::Text => {
                        this.emit_blur_event(cx);
                    }
                    _ => {}
                },
            ),
            cx.subscribe_in(
                &this.json_editor,
                window,
                |this, _, event: &InputEvent, _window, cx| match event {
                    InputEvent::Change if !this.suppress_edit_tracking => {
                        this.has_user_edits = true;
                    }
                    InputEvent::Blur if this.active_tab == EditorTab::Json => {
                        this.emit_blur_event(cx);
                    }
                    _ => {}
                },
            ),
        ];
        this
    }

    pub fn switch_tab(&mut self, tab: EditorTab, window: &mut Window, cx: &mut Context<Self>) {
        if self.active_tab == tab {
            return;
        }

        let content = self
            .get_active_text(cx)
            .unwrap_or_else(|_| self.get_raw_active_text(cx));
        self.active_tab = tab;
        self.set_active_text(content, window, cx);
        cx.notify();
    }

    pub fn get_active_text(&self, cx: &App) -> Result<String, json5::Error> {
        let value = self.get_raw_active_text(cx);
        if self.active_tab == EditorTab::Json {
            return match json5::from_str::<serde_json::Value>(&value) {
                Ok(v) => Ok(v.to_string()),
                Err(e) => Err(e),
            };
        }
        Ok(value)
    }

    pub fn get_raw_active_text(&self, cx: &App) -> String {
        let text_content = self.text_editor.read(cx).text().to_string();
        let json_content = self.json_editor.read(cx).text().to_string();

        active_editor_text(self.active_tab, &text_content, &json_content)
    }

    pub fn get_writeback_text(&self, cx: &App) -> Result<String, json5::Error> {
        normalize_commit_text(self.active_tab, &self.get_raw_active_text(cx))
    }

    pub fn has_pending_writeback(&self) -> bool {
        self.has_user_edits
    }

    pub fn mark_writeback_clean(&mut self) {
        self.has_user_edits = false;
    }

    fn set_editor_values(
        &mut self,
        text_value: String,
        json_value: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.suppress_edit_tracking = true;
        self.text_editor.update(cx, |s, cx| {
            s.set_value(text_value.clone(), window, cx);
        });

        self.json_editor.update(cx, |s, cx| {
            s.set_value(json_value, window, cx);
        });
        self.suppress_edit_tracking = false;
    }

    pub fn set_active_text(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        // Try to parse and format as JSON for json editor
        let json_text = match json5::from_str::<serde_json::Value>(&text) {
            Ok(value) => serde_json::to_string_pretty(&value).unwrap_or(text.clone()),
            Err(_) => text.clone(),
        };

        self.set_editor_values(text, json_text, window, cx);
        self.mark_writeback_clean();
    }

    pub fn load_external_text(
        &mut self,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let is_json = json5::from_str::<serde_json::Value>(&text).is_ok();
        self.set_active_text(text, window, cx);
        if self.active_tab == EditorTab::Json && !is_json {
            self.active_tab = EditorTab::Text;
            cx.notify();
        }
    }

    fn emit_blur_event(&mut self, cx: &mut Context<Self>) {
        if !self.has_pending_writeback() {
            return;
        }

        let value = self
            .get_writeback_text(cx)
            .unwrap_or_else(|_| self.get_raw_active_text(cx));
        cx.emit(MultiTextEditorEvent::ActiveEditorBlurred(value));
    }

    pub fn format_json(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.json_editor.read(cx).text().to_string();
        match json5::from_str::<serde_json::Value>(&text) {
            Ok(value) => {
                if let Ok(formatted) = serde_json::to_string_pretty(&value) {
                    self.set_active_text(formatted, window, cx);
                    self.active_tab = EditorTab::Json;
                }
            }
            Err(e) => {
                error!("JSON解析错误: {:?}", e)
            }
        }
    }

    pub fn minify_json(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.json_editor.read(cx).text().to_string();
        match json5::from_str::<serde_json::Value>(&text) {
            Ok(value) => {
                if let Ok(minified) = serde_json::to_string(&value) {
                    self.set_active_text(minified, window, cx);
                    self.active_tab = EditorTab::Json;
                }
            }
            Err(e) => {
                error!("JSON压缩错误: {:?}", e)
            }
        }
    }
}

impl Render for MultiTextEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        use gpui::ParentElement;
        use gpui::prelude::FluentBuilder;
        use gpui_component::{IconName, Sizable, Size, button::Button, h_flex};

        let active_tab = self.active_tab;
        let is_json_tab = active_tab == EditorTab::Json;
        let active_index = if active_tab == EditorTab::Text { 0 } else { 1 };

        v_flex()
            .size_full()
            .child(
                TabBar::new("editor-tabs")
                    .with_size(Size::Small)
                    .selected_index(active_index)
                    .child(Tab::new().label("Text"))
                    .child(Tab::new().label("JSON"))
                    .on_click(cx.listener(|this, ix: &usize, window, cx| {
                        let tab = if *ix == 0 {
                            EditorTab::Text
                        } else {
                            EditorTab::Json
                        };
                        this.switch_tab(tab, window, cx);
                    }))
                    .suffix(h_flex().gap_2().when(is_json_tab, |this| {
                        this.child(
                            Button::new("format-json")
                                .with_size(Size::Small)
                                .label("Format")
                                .icon(IconName::Star)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.format_json(window, cx);
                                })),
                        )
                        .child(
                            Button::new("minify-json")
                                .with_size(Size::Small)
                                .label("Minify")
                                .icon(IconName::File)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.minify_json(window, cx);
                                })),
                        )
                    })),
            )
            .child(v_flex().flex_1().child(match active_tab {
                EditorTab::Text => Input::new(&self.text_editor).size_full(),
                EditorTab::Json => Input::new(&self.json_editor).size_full(),
            }))
    }
}

pub fn create_multi_text_editor_with_content(
    initial_content: Option<String>,
    window: &mut Window,
    cx: &mut App,
) -> Entity<MultiTextEditor> {
    cx.new(|cx| {
        let mut editor = MultiTextEditor::new(window, cx);
        if let Some(content) = initial_content {
            editor.set_active_text(content, window, cx);
        }
        editor
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_editor_text_returns_text_tab_content() {
        let value = active_editor_text(EditorTab::Text, "plain text", "{\n  \"a\": 1\n}");

        assert_eq!(value, "plain text");
    }

    #[test]
    fn active_editor_text_returns_json_tab_content() {
        let value = active_editor_text(EditorTab::Json, "plain text", "{\n  \"a\": 1\n}");

        assert_eq!(value, "{\n  \"a\": 1\n}");
    }

    #[test]
    fn normalize_commit_text_minifies_valid_json_from_text_tab() {
        let value = normalize_commit_text(EditorTab::Text, "{\n  \"a\": 1,\n  \"b\": true\n}")
            .expect("text tab JSON should be minified");

        assert_eq!(value, "{\"a\":1,\"b\":true}");
    }

    #[test]
    fn normalize_commit_text_preserves_plain_text_from_text_tab() {
        let value = normalize_commit_text(EditorTab::Text, "plain text")
            .expect("plain text should be preserved");

        assert_eq!(value, "plain text");
    }

    #[test]
    fn normalize_commit_text_requires_valid_json_from_json_tab() {
        let err = normalize_commit_text(EditorTab::Json, "{invalid json}")
            .expect_err("json tab should validate before commit");

        assert!(!err.to_string().is_empty());
    }

    #[test]
    fn get_writeback_text_minifies_json_for_writeback() {
        let value = normalize_commit_text(EditorTab::Text, "{\n  \"a\": 1,\n  \"b\": true\n}")
            .expect("writeback should minify valid json");

        assert_eq!(value, "{\"a\":1,\"b\":true}");
    }
}
