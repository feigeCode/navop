mod logic;
mod render;

use gpui::{App, AppContext, Context, Entity, EventEmitter, Subscription, Window};
use gpui_component::input::{InputEvent, InputState, TabSize};
use logic::{
    JsonEditorSyncMode, active_editor_text, editor_values_for_text, normalize_commit_text,
};
pub use logic::{LargeTextEditorTab, large_text_values_equivalent};
use tracing::error;

#[derive(Clone, Debug)]
pub enum LargeTextEditorEvent {
    ActiveEditorBlurred(String),
}

impl EventEmitter<LargeTextEditorEvent> for LargeTextEditor {}

pub struct LargeTextEditor {
    active_tab: LargeTextEditorTab,
    text_editor: Entity<InputState>,
    json_editor: Entity<InputState>,
    has_user_edits: bool,
    suppress_edit_tracking: bool,
    _subs: Vec<Subscription>,
}

impl LargeTextEditor {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let text_editor = cx.new(|cx| build_editor(window, LargeTextEditorTab::Text, cx));
        let json_editor = cx.new(|cx| build_editor(window, LargeTextEditorTab::Json, cx));
        let mut this = Self::new_with_editors(text_editor, json_editor);
        this._subs = this.subscribe_editors(window, cx);
        this
    }

    fn new_with_editors(text_editor: Entity<InputState>, json_editor: Entity<InputState>) -> Self {
        Self {
            active_tab: LargeTextEditorTab::Text,
            text_editor,
            json_editor,
            has_user_edits: false,
            suppress_edit_tracking: false,
            _subs: Vec::new(),
        }
    }

    fn subscribe_editors(&self, window: &mut Window, cx: &mut Context<Self>) -> Vec<Subscription> {
        vec![
            cx.subscribe_in(&self.text_editor, window, Self::on_text_event),
            cx.subscribe_in(&self.json_editor, window, Self::on_json_event),
        ]
    }

    fn on_text_event(
        &mut self,
        _: &Entity<InputState>,
        event: &InputEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.handle_editor_event(LargeTextEditorTab::Text, event, cx);
    }

    fn on_json_event(
        &mut self,
        _: &Entity<InputState>,
        event: &InputEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.handle_editor_event(LargeTextEditorTab::Json, event, cx);
    }

    fn handle_editor_event(
        &mut self,
        tab: LargeTextEditorTab,
        event: &InputEvent,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Change if !self.suppress_edit_tracking => {
                self.has_user_edits = true;
            }
            InputEvent::Blur if self.active_tab == tab => self.emit_blur_event(cx),
            _ => {}
        }
    }

    pub fn switch_tab(
        &mut self,
        tab: LargeTextEditorTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
        if self.active_tab == LargeTextEditorTab::Json {
            return json5::from_str::<serde_json::Value>(&value).map(|v| v.to_string());
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

    pub fn set_active_text(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        self.set_active_text_with_dirty_state(text, self.has_user_edits, window, cx);
    }

    pub fn load_external_text(
        &mut self,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let is_json = json5::from_str::<serde_json::Value>(&text).is_ok();
        self.load_committed_text(text, window, cx);
        if self.active_tab == LargeTextEditorTab::Json && !is_json {
            self.active_tab = LargeTextEditorTab::Text;
            cx.notify();
        }
    }

    pub fn format_json(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.json_editor.read(cx).text().to_string();
        match json5::from_str::<serde_json::Value>(&text) {
            Ok(value) => {
                if let Ok(formatted) = serde_json::to_string_pretty(&value) {
                    self.apply_json_transform(formatted, window, cx);
                }
            }
            Err(e) => error!("JSON parse error: {:?}", e),
        }
    }

    pub fn minify_json(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.json_editor.read(cx).text().to_string();
        match json5::from_str::<serde_json::Value>(&text) {
            Ok(value) => {
                if let Ok(minified) = serde_json::to_string(&value) {
                    self.apply_json_transform(minified, window, cx);
                }
            }
            Err(e) => error!("JSON minify error: {:?}", e),
        }
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

    fn set_active_text_with_dirty_state(
        &mut self,
        text: String,
        next_dirty_state: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_text_with_sync_mode_and_dirty_state(
            text,
            JsonEditorSyncMode::Pretty,
            next_dirty_state,
            window,
            cx,
        );
    }

    fn set_text_with_sync_mode_and_dirty_state(
        &mut self,
        text: String,
        json_sync_mode: JsonEditorSyncMode,
        next_dirty_state: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (text_value, json_text) = editor_values_for_text(&text, json_sync_mode);
        self.set_editor_values(text_value, json_text, window, cx);
        self.has_user_edits = next_dirty_state;
    }

    fn load_committed_text(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        self.set_active_text_with_dirty_state(text, false, window, cx);
    }

    fn apply_json_transform(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        self.set_text_with_sync_mode_and_dirty_state(
            text,
            JsonEditorSyncMode::Mirror,
            true,
            window,
            cx,
        );
        self.active_tab = LargeTextEditorTab::Json;
    }

    fn emit_blur_event(&mut self, cx: &mut Context<Self>) {
        if !self.has_pending_writeback() {
            return;
        }

        let value = self
            .get_writeback_text(cx)
            .unwrap_or_else(|_| self.get_raw_active_text(cx));
        cx.emit(LargeTextEditorEvent::ActiveEditorBlurred(value));
    }
}

fn build_editor(
    window: &mut Window,
    tab: LargeTextEditorTab,
    cx: &mut Context<InputState>,
) -> InputState {
    InputState::new(window, cx)
        .code_editor(tab.language())
        .line_number(true)
        .searchable(true)
        .indent_guides(true)
        .tab_size(TabSize {
            tab_size: 2,
            hard_tabs: false,
        })
        .soft_wrap(false)
        .placeholder(match tab {
            LargeTextEditorTab::Text => "Enter your text here...",
            LargeTextEditorTab::Json => "Enter JSON here...",
        })
}

pub fn create_large_text_editor_with_content(
    initial_content: Option<String>,
    window: &mut Window,
    cx: &mut impl AppContext,
) -> Entity<LargeTextEditor> {
    cx.new(|cx| {
        let mut editor = LargeTextEditor::new(window, cx);
        if let Some(content) = initial_content {
            editor.load_committed_text(content, window, cx);
        }
        editor
    })
}
