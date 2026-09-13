//! 通用 JSON 值视图:结构化树 / 原始字符串编辑器双模式。
//!
//! 面向"已经持有 `serde_json::Value`(或原始字符串)"的场景,例如
//! resource workbench 的 JSON 页面。Tree 模式渲染可折叠树;Raw 模式在
//! 文本编辑器中展示原始 JSON,编辑内容会防抖回解析,非法 JSON 提示错误
//! 并保留旧树,便于在两种视图间来回切换。

use std::collections::HashSet;
use std::sync::Arc;

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::button::Button;
use gpui_component::input::{Editor, EditorState, InputEvent};
use gpui_component::{ActiveTheme, Selectable as _, Sizable as _, h_flex, v_flex};
use rust_i18n::t;

use crate::tree::{
    FlatRow, NodePath, TreeCallbacks, collect_all_paths, flatten, render_flat_row,
    seed_default_expanded,
};

const PARSE_DEBOUNCE_MS: u64 = 300;

/// JSON 展示模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsonDisplayMode {
    /// 可折叠结构化树。
    Tree,
    /// 原始 JSON 字符串编辑器。
    Raw,
}

/// 面向已有 JSON 值的通用查看/编辑组件。
pub struct JsonValueView {
    value: Option<serde_json::Value>,
    raw: String,
    mode: JsonDisplayMode,
    error: Option<String>,
    notice: Option<String>,
    rows: Arc<Vec<FlatRow>>,
    expanded: HashSet<NodePath>,
    list_state: ListState,
    editor: Option<Entity<EditorState>>,
    focus_handle: FocusHandle,
    _subs: Vec<Subscription>,
}

impl JsonValueView {
    pub fn new(value: serde_json::Value, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            value: Some(value),
            raw: String::new(),
            mode: JsonDisplayMode::Tree,
            error: None,
            notice: None,
            rows: Arc::new(Vec::new()),
            expanded: HashSet::new(),
            list_state: ListState::new(0, ListAlignment::Top, px(200.)),
            editor: None,
            focus_handle: cx.focus_handle(),
            _subs: Vec::new(),
        };
        this.reset_from_value(window, cx);
        this
    }

    pub fn mode(&self) -> JsonDisplayMode {
        self.mode
    }

    /// 当前承载的 JSON 值快照,用于外部检测数据是否已变化。
    pub fn snapshot(&self) -> Option<&serde_json::Value> {
        self.value.as_ref()
    }

    /// 切换展示模式;首次进入 Raw 模式时惰性创建编辑器。
    pub fn set_mode(
        &mut self,
        mode: JsonDisplayMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.mode == mode {
            return;
        }
        self.mode = mode;
        self.error = None;
        self.notice = None;
        if mode == JsonDisplayMode::Raw {
            self.ensure_editor(window, cx);
            self.sync_editor_value(window, cx);
        }
        cx.notify();
    }

    /// 替换组件承载的 JSON 值(外部数据刷新入口)。
    pub fn set_value(
        &mut self,
        value: serde_json::Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.value = Some(value);
        self.error = None;
        self.notice = None;
        self.reset_from_value(window, cx);
        if self.mode == JsonDisplayMode::Raw {
            self.sync_editor_value(window, cx);
        }
        cx.notify();
    }

    pub fn expand_all(&mut self, cx: &mut Context<Self>) {
        if let Some(value) = &self.value {
            self.expanded.clear();
            collect_all_paths(value, &mut Vec::new(), &mut self.expanded);
            self.rebuild_rows();
            cx.notify();
        }
    }

    pub fn collapse_all(&mut self, cx: &mut Context<Self>) {
        self.expanded.clear();
        self.rebuild_rows();
        cx.notify();
    }

    fn copy_json(&mut self, cx: &mut Context<Self>) {
        let text = match &self.value {
            Some(value) => serde_json::to_string_pretty(value).unwrap_or_default(),
            None => self.raw.clone(),
        };
        if text.is_empty() {
            return;
        }
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.notice = Some(t!("JsonValue.copied").to_string());
        cx.notify();
    }

    fn copy_value(&mut self, raw: String, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(raw));
        self.notice = Some(t!("JsonValue.copied").to_string());
        cx.notify();
    }

    fn reset_from_value(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.raw = self
            .value
            .as_ref()
            .map(|value| serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".into()))
            .unwrap_or_default();
        self.expanded.clear();
        if let Some(value) = &self.value {
            seed_default_expanded(value, &mut Vec::new(), &mut self.expanded);
        }
        self.rebuild_rows();
        if self.editor.is_some() {
            self.sync_editor_value(window, cx);
        }
    }

    fn toggle_path(&mut self, path: &[u32], cx: &mut Context<Self>) {
        if self.expanded.contains(path) {
            self.expanded.remove(path);
        } else {
            self.expanded.insert(path.to_vec());
        }
        self.rebuild_rows();
        cx.notify();
    }

    fn rebuild_rows(&mut self) {
        let Some(value) = &self.value else {
            self.rows = Arc::new(Vec::new());
            self.list_state.reset(0);
            return;
        };
        let mut rows = Vec::new();
        flatten(
            value,
            "",
            0,
            true,
            &self.expanded,
            &mut Vec::new(),
            &mut rows,
        );
        self.rows = Arc::new(rows);
        self.list_state.reset(self.rows.len());
    }

    fn ensure_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.editor.is_some() {
            return;
        }
        let raw = self.raw.clone();
        let editor = cx.new(|cx| {
            let mut state = EditorState::new(window, cx)
                .language("json")
                .soft_wrap(true)
                .line_number(false);
            state.set_value(raw, window, cx);
            state
        });
        let editor_ref = editor.clone();
        let sub = cx.subscribe(&editor, move |this: &mut Self, _src, ev: &InputEvent, cx| {
            if !matches!(ev, InputEvent::Change | InputEvent::Blur) {
                return;
            }
            let raw = editor_ref.read(cx).value().to_string();
            this.raw = raw.clone();
            match ev {
                InputEvent::Change => this.schedule_parse(raw, cx),
                InputEvent::Blur => this.parse_raw(raw, cx),
                _ => {}
            }
        });
        self.editor = Some(editor);
        self._subs.push(sub);
    }

    fn sync_editor_value(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let raw = self.raw.clone();
        if let Some(editor) = &self.editor {
            editor.update(cx, |state, cx| state.set_value(raw, window, cx));
        }
    }

    fn schedule_parse(&mut self, raw: String, cx: &mut Context<Self>) {
        let weak = cx.weak_entity();
        cx.spawn(async move |_this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(PARSE_DEBOUNCE_MS))
                .await;
            let _ = weak.update(cx, |this, cx| this.parse_raw(raw, cx));
        })
        .detach();
    }

    fn parse_raw(&mut self, raw: String, cx: &mut Context<Self>) {
        if raw.trim().is_empty() {
            self.value = None;
            self.error = None;
            self.rebuild_rows();
            cx.notify();
            return;
        }
        match serde_json::from_str::<serde_json::Value>(&raw) {
            Ok(value) => {
                self.value = Some(value);
                self.error = None;
                self.expanded.clear();
                if let Some(value) = &self.value {
                    seed_default_expanded(value, &mut Vec::new(), &mut self.expanded);
                }
                self.rebuild_rows();
            }
            Err(error) => {
                self.error =
                    Some(t!("JsonValue.invalid_json", error = error.to_string()).to_string());
            }
        }
        cx.notify();
    }
}

impl Focusable for JsonValueView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for JsonValueView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let tree_active = self.mode == JsonDisplayMode::Tree;
        if !tree_active {
            self.ensure_editor(window, cx);
        }

        let mut toolbar = h_flex()
            .gap_1()
            .child(
                Button::new("json-value-mode-tree")
                    .small()
                    .label(t!("JsonValue.mode_tree"))
                    .selected(tree_active)
                    .on_click(cx.listener(|this, _ev, window, cx| {
                        this.set_mode(JsonDisplayMode::Tree, window, cx);
                    })),
            )
            .child(
                Button::new("json-value-mode-raw")
                    .small()
                    .label(t!("JsonValue.mode_raw"))
                    .selected(!tree_active)
                    .on_click(cx.listener(|this, _ev, window, cx| {
                        this.set_mode(JsonDisplayMode::Raw, window, cx);
                    })),
            );
        if tree_active {
            toolbar = toolbar
                .child(
                    Button::new("json-value-expand-all")
                        .small()
                        .label(t!("JsonValue.expand_all"))
                        .on_click(cx.listener(|this, _ev, _window, cx| this.expand_all(cx))),
                )
                .child(
                    Button::new("json-value-collapse-all")
                        .small()
                        .label(t!("JsonValue.collapse_all"))
                        .on_click(cx.listener(|this, _ev, _window, cx| this.collapse_all(cx))),
                );
        }
        toolbar = toolbar.child(
            Button::new("json-value-copy")
                .small()
                .label(t!("JsonValue.copy"))
                .on_click(cx.listener(|this, _ev, _window, cx| this.copy_json(cx))),
        );

        let header = h_flex()
            .w_full()
            .items_center()
            .justify_between()
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(t!("JsonValue.title")),
            )
            .child(toolbar.when_some(self.notice.clone(), |flex, notice| {
                flex.child(div().text_xs().text_color(theme.success).child(notice))
            }));

        let mono_font = theme.mono_font_family.clone();
        let fg = theme.foreground;
        let warn = theme.warning;
        let muted_fg = theme.muted_foreground;
        let success = theme.success;
        let info = theme.info;

        let body = if tree_active {
            let rows = self.rows.clone();
            let has_output = !rows.is_empty();
            let callbacks = TreeCallbacks {
                toggle: Arc::new({
                    let weak = cx.weak_entity();
                    move |path: &NodePath, _window, cx| {
                        let path = path.clone();
                        let _ = weak.update(cx, |this, cx| this.toggle_path(&path, cx));
                    }
                }),
                copy_value: Arc::new({
                    let weak = cx.weak_entity();
                    move |raw: String, _window, cx| {
                        let _ = weak.update(cx, |this, cx| this.copy_value(raw, cx));
                    }
                }),
            };
            div()
                .id("json-value-tree")
                .relative()
                .flex_1()
                .min_h_0()
                .min_w_0()
                .overflow_hidden()
                .border_1()
                .border_color(theme.border)
                .rounded(px(4.))
                .p_1()
                .bg(theme.muted.opacity(0.3))
                .child(
                    list(self.list_state.clone(), {
                        let callbacks = callbacks.clone();
                        let mono_font = mono_font.clone();
                        move |ix, _window, _cx| {
                            let Some(row) = rows.get(ix) else {
                                return div().h(px(0.)).into_any_element();
                            };
                            render_flat_row(
                                ix,
                                row,
                                &mono_font,
                                fg,
                                warn,
                                muted_fg,
                                success,
                                info,
                                &callbacks,
                            )
                        }
                    })
                    .flex_grow_1()
                    .size_full(),
                )
                .when(!has_output, |c| {
                    c.child(
                        div()
                            .absolute()
                            .top_0()
                            .right_0()
                            .bottom_0()
                            .left_0()
                            .child(
                                v_flex()
                                    .size_full()
                                    .items_center()
                                    .justify_center()
                                    .text_color(theme.muted_foreground)
                                    .text_sm()
                                    .child(t!("JsonValue.empty").to_string()),
                            ),
                    )
                })
                .into_any_element()
        } else {
            let editor = self.editor.clone().expect("editor exists in raw mode");
            div()
                .id("json-value-raw")
                .flex_1()
                .min_h_0()
                .min_w_0()
                .border_1()
                .border_color(theme.border)
                .rounded(px(4.))
                .overflow_hidden()
                .child(Editor::new(&editor).h_full())
                .into_any_element()
        };

        v_flex()
            .size_full()
            .min_w_0()
            .min_h_0()
            .gap_2()
            .p_2()
            .child(header)
            .when_some(self.error.clone(), |flex, error| {
                flex.child(div().text_sm().text_color(theme.danger).child(error))
            })
            .child(body)
    }
}
