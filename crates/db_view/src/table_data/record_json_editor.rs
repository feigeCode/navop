use gpui::{
    ClickEvent, Context, Entity, IntoElement, ParentElement, Render, Styled, Window, div,
    prelude::*,
};
use gpui_component::{
    Sizable as _, Size, WindowExt as _,
    button::Button,
    h_flex,
    input::{Editor, EditorState},
    v_flex,
};
use rust_i18n::t;

/// 单条记录的 JSON 阅读编辑器。
///
/// 记录本身是扁平的「字段 -> 值」对象，不需要树形展开；这里只提供
/// 格式化与压缩两个文本变换，保留语法高亮和复制能力。
pub struct RecordJsonEditor {
    editor: Entity<EditorState>,
}

impl RecordJsonEditor {
    pub fn new(value: &serde_json::Value, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let text = serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".to_string());
        let editor = cx.new(|cx| {
            let mut state = EditorState::new(window, cx)
                .language("json")
                .line_number(true)
                .soft_wrap(true);
            state.set_value(text, window, cx);
            state
        });

        Self { editor }
    }

    fn handle_format(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.transform(true, window, cx);
    }

    fn handle_compact(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.transform(false, window, cx);
    }

    fn transform(&mut self, pretty: bool, window: &mut Window, cx: &mut Context<Self>) {
        let raw = self.editor.read(cx).value().to_string();
        match transform_record_json(&raw, pretty) {
            Ok(text) => {
                self.editor.update(cx, |state, cx| {
                    state.set_value(text, window, cx);
                });
            }
            Err(_) => {
                window.push_notification(t!("TableDataGrid.record_json_invalid").to_string(), cx);
            }
        }
    }
}

fn transform_record_json(raw: &str, pretty: bool) -> Result<String, serde_json::Error> {
    let value: serde_json::Value = serde_json::from_str(raw)?;
    if pretty {
        serde_json::to_string_pretty(&value)
    } else {
        serde_json::to_string(&value)
    }
}

impl Render for RecordJsonEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .gap_2()
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("format-record-json")
                            .with_size(Size::Small)
                            .label(t!("TableDataGrid.record_json_format").to_string())
                            .on_click(cx.listener(Self::handle_format)),
                    )
                    .child(
                        Button::new("compact-record-json")
                            .with_size(Size::Small)
                            .label(t!("TableDataGrid.record_json_compact").to_string())
                            .on_click(cx.listener(Self::handle_compact)),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .overflow_hidden()
                    .child(Editor::new(&self.editor).readonly(true).size_full()),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::transform_record_json;

    #[test]
    fn format_record_json_uses_two_space_indentation() {
        let text = transform_record_json(r#"{"id":42,"enabled":true}"#, true)
            .expect("record JSON should format");

        assert!(text.contains("\n  \""));
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&text).unwrap(),
            serde_json::json!({"id": 42, "enabled": true})
        );
    }

    #[test]
    fn compact_record_json_removes_whitespace() {
        let text = transform_record_json("{\n  \"id\": 42,\n  \"enabled\": true\n}", false)
            .expect("record JSON should compact");

        assert!(!text.contains('\n'));
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&text).unwrap(),
            serde_json::json!({"id": 42, "enabled": true})
        );
    }

    #[test]
    fn transform_record_json_rejects_invalid_text() {
        assert!(transform_record_json("{invalid}", true).is_err());
    }
}
