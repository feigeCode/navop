//! JSON 格式化器视图(可折叠树形)。
//!
//! 移植自 verve 的 `src/ui/json_panel.rs`:左栏粘贴 JSON,右栏以可折叠的树形
//! 展示。树模型与行渲染复用 `tree.rs`;通用值视图见 `JsonValueView`。

use std::collections::HashSet;
use std::sync::Arc;

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::button::Button;
use gpui_component::input::{Editor, EditorState, InputEvent};
use gpui_component::resizable::{h_resizable, resizable_panel};
use gpui_component::{ActiveTheme, Sizable as _, h_flex, v_flex};
use rust_i18n::t;

use crate::tree::{
    FlatRow, NodePath, TreeCallbacks, collect_all_paths, render_flat_row, seed_default_expanded,
    simplify,
};

pub struct JsonFormatterView {
    input: Entity<EditorState>,
    value: Option<serde_json::Value>,
    parsing: bool,
    error: Option<String>,
    notice: Option<String>,
    compact: bool,
    rows: Arc<Vec<FlatRow>>,
    list_state: ListState,
    expanded: HashSet<NodePath>,
    format_timer: Option<Task<()>>,
    pub(crate) focus_handle: FocusHandle,
    _subs: Vec<Subscription>,
}

impl JsonFormatterView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| {
            let mut s = EditorState::new(window, cx)
                .language("json")
                .soft_wrap(true)
                .line_number(false)
                .placeholder(t!("JsonFormatter.input_placeholder").to_string());
            s.set_value(String::new(), window, cx);
            s
        });

        let input_clone = input.clone();
        let input_sub = cx.subscribe(&input, move |this: &mut Self, _src, ev: &InputEvent, cx| {
            if matches!(ev, InputEvent::Blur) {
                let raw = input_clone.read(cx).value().to_string();
                this.compact = false;
                this.cancel_format_timer();
                this.do_format(&raw, false, cx);
            } else if matches!(ev, InputEvent::Change) {
                let raw = input_clone.read(cx).value().to_string();
                this.schedule_format(raw, cx);
            }
        });

        Self {
            input,
            value: None,
            parsing: false,
            error: None,
            notice: None,
            compact: false,
            rows: Arc::new(Vec::new()),
            list_state: ListState::new(0, ListAlignment::Top, px(200.)),
            expanded: HashSet::new(),
            format_timer: None,
            focus_handle: cx.focus_handle(),
            _subs: vec![input_sub],
        }
    }

    pub fn is_compact_active(&self) -> bool {
        self.compact
    }

    pub fn toggle_compact(&mut self, cx: &mut Context<Self>) {
        self.compact = !self.compact;
        let raw = self.input.read(cx).value().to_string();
        self.do_format(&raw, self.compact, cx);
        if self.value.is_some() {
            self.notice = Some(if self.compact {
                t!("JsonFormatter.compact").to_string()
            } else {
                t!("JsonFormatter.formatted").to_string()
            });
            cx.notify();
        }
    }

    pub fn expand_all(&mut self, cx: &mut Context<Self>) {
        if let Some(ref value) = self.value {
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

    pub fn copy_result(&mut self, cx: &mut Context<Self>) {
        let Some(ref value) = self.value else { return };
        if let Ok(json_str) = serde_json::to_string_pretty(value) {
            cx.write_to_clipboard(ClipboardItem::new_string(json_str));
            self.notice = Some(t!("JsonFormatter.copy_success").to_string());
            cx.notify();
        }
    }

    pub fn copy_value(&mut self, raw: String, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(raw));
        self.notice = Some(t!("JsonFormatter.copy_success").to_string());
        cx.notify();
    }

    fn schedule_format(&mut self, raw: String, cx: &mut Context<Self>) {
        let weak = cx.weak_entity();
        let timer = cx.spawn(async move |_this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(300))
                .await;
            let _ = weak.update(cx, |this, cx| this.do_format(&raw, false, cx));
        });
        self.format_timer = Some(timer);
    }

    fn cancel_format_timer(&mut self) {
        self.format_timer.take();
    }

    fn do_format(&mut self, raw: &str, compact: bool, cx: &mut Context<Self>) {
        if raw.trim().is_empty() {
            self.error = None;
            self.value = None;
            self.parsing = false;
            self.rows = Arc::new(Vec::new());
            self.list_state.reset(0);
            self.expanded.clear();
            cx.notify();
            return;
        }

        self.error = None;
        self.notice = None;
        self.parsing = true;
        self.value = None;
        self.rows = Arc::new(Vec::new());
        self.list_state.reset(0);
        cx.notify();

        let raw_owned = raw.to_string();
        let weak = cx.weak_entity();
        cx.spawn(async move |_this, cx| {
            let parsed = cx
                .background_executor()
                .spawn(async move {
                    match serde_json::from_str::<serde_json::Value>(&raw_owned) {
                        Ok(value) => Ok(if compact { simplify(&value) } else { value }),
                        Err(e) => Err(e),
                    }
                })
                .await;
            let _ = weak.update(cx, |this, cx| this.apply_format_result(parsed, cx));
        })
        .detach();
    }

    fn apply_format_result(
        &mut self,
        parsed: Result<serde_json::Value, serde_json::Error>,
        cx: &mut Context<Self>,
    ) {
        match parsed {
            Ok(value) => {
                self.value = Some(value);
                self.error = None;
                self.parsing = false;
                self.expanded.clear();
                if let Some(ref v) = self.value {
                    seed_default_expanded(v, &mut Vec::new(), &mut self.expanded);
                }
                self.rebuild_rows();
            }
            Err(e) => {
                self.error =
                    Some(t!("JsonFormatter.invalid_json", error = e.to_string()).to_string());
                self.value = None;
                self.parsing = false;
                self.rows = Arc::new(Vec::new());
                self.list_state.reset(0);
            }
        }
        self.notice = None;
        cx.notify();
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
        crate::tree::flatten(
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
}

impl Render for JsonFormatterView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();

        let toolbar = h_flex()
            .w_full()
            .items_center()
            .justify_between()
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(t!("JsonFormatter.formatted").to_string()),
            )
            .child(
                h_flex()
                    .gap_1()
                    .child(
                        Button::new("json-expand-all")
                            .small()
                            .label(t!("JsonFormatter.expand_all"))
                            .on_click(cx.listener(|this, _ev, _window, cx| this.expand_all(cx))),
                    )
                    .child(
                        Button::new("json-collapse-all")
                            .small()
                            .label(t!("JsonFormatter.collapse_all"))
                            .on_click(cx.listener(|this, _ev, _window, cx| this.collapse_all(cx))),
                    )
                    .child(
                        Button::new("json-compact")
                            .small()
                            .label(t!("JsonFormatter.compact"))
                            .on_click(
                                cx.listener(|this, _ev, _window, cx| this.toggle_compact(cx)),
                            ),
                    )
                    .child(
                        Button::new("json-copy")
                            .small()
                            .label(t!("JsonFormatter.copy_result"))
                            .on_click(cx.listener(|this, _ev, _window, cx| this.copy_result(cx))),
                    )
                    .when_some(self.notice.clone(), |flex, notice| {
                        flex.child(div().text_xs().text_color(theme.success).child(notice))
                    }),
            );

        let input_el = v_flex()
            .size_full()
            .gap_2()
            .p_2()
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(t!("JsonFormatter.input_placeholder")),
            )
            .child(
                div()
                    .id("json-input-box")
                    .flex_1()
                    .min_h_0()
                    .border_1()
                    .border_color(theme.border)
                    .rounded(px(4.))
                    .overflow_hidden()
                    .child(Editor::new(&self.input).h_full()),
            )
            .when_some(self.error.clone(), |flex, err| {
                flex.child(div().text_sm().text_color(theme.danger).child(err))
            });

        let rows = self.rows.clone();
        let has_output = !rows.is_empty();
        let parsing = self.parsing;
        let mono_font = theme.mono_font_family.clone();
        let fg = theme.foreground;
        let warn = theme.warning;
        let muted_fg = theme.muted_foreground;
        let success = theme.success;
        let info = theme.info;
        let callbacks = TreeCallbacks {
            toggle: Arc::new({
                let weak = cx.weak_entity();
                move |path: &NodePath, _window, cx| {
                    let path = path.clone();
                    let _ = weak.update(cx, |panel, cx| panel.toggle_path(&path, cx));
                }
            }),
            copy_value: Arc::new({
                let weak = cx.weak_entity();
                move |raw: String, _window, cx| {
                    let _ = weak.update(cx, |panel, cx| panel.copy_value(raw, cx));
                }
            }),
        };

        let output_el = v_flex().size_full().gap_2().p_2().child(toolbar).child(
            div()
                .id("json-output-list")
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
                    list(self.list_state.clone(), move |ix, _window, _cx| {
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
                    })
                    .flex_grow_1()
                    .size_full(),
                )
                .when(parsing, |c| {
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
                                    .gap_2()
                                    .text_color(theme.muted_foreground)
                                    .child(
                                        div()
                                            .text_sm()
                                            .child(t!("JsonFormatter.parsing").to_string()),
                                    ),
                            ),
                    )
                })
                .when(!has_output && !parsing, |c| {
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
                                    .child(t!("JsonFormatter.input_placeholder").to_string()),
                            ),
                    )
                }),
        );

        h_resizable("api-json-split")
            .child(
                resizable_panel()
                    .size(px(420.))
                    .size_range(px(240.)..px(900.))
                    .overflow_hidden()
                    .child(
                        div()
                            .id("json-input-pane")
                            .size_full()
                            .min_w_0()
                            .min_h_0()
                            .overflow_hidden()
                            .child(input_el),
                    ),
            )
            .child(
                resizable_panel()
                    .size_range(px(320.)..px(2000.))
                    .overflow_hidden()
                    .child(
                        div()
                            .id("json-output-pane")
                            .size_full()
                            .min_w_0()
                            .min_h_0()
                            .overflow_hidden()
                            .child(output_el),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn renderer_keeps_tree_list_full_size_and_input_resizable() {
        let source = include_str!("json_view.rs");
        let render = source
            .find("impl Render for JsonFormatterView")
            .map(|start| &source[start..])
            .expect("json formatter render impl");

        assert!(
            render.contains("h_resizable(\"api-json-split\")"),
            "JSON formatter must use a resizable horizontal split"
        );
        assert!(
            render.contains(".flex_grow_1()") && render.contains(".size_full()"),
            "the virtualized JSON tree must keep growing to fill its pane"
        );
        assert!(
            render.contains(".size_range(px(240.)"),
            "the input pane must have a minimum width so it cannot be squeezed"
        );
    }
}
