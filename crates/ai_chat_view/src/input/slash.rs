//! `/` 斜杠命令补全。
//!
//! ACP 的 `available_commands`（`session/update` 推来的）此前只用来显示一个计数徽章，
//! 命令本身打不出来。这里把它接成输入框 `/` 补全的来源。
//!
//! 选中即插入 `/name `，不另开执行通道：协议里斜杠命令就是 prompt 文本，输入框发出去的
//! 那条消息原样交给 agent 解析。所以这里只负责「补全到和 agent 的命令名逐字对上」。

use std::sync::Arc;

use anyhow::Result;
use gpui::{App, AppContext, Task, Window};
use gpui_component::input::CompletionProvider;
use gpui_component::{Rope, RopeExt};
use lsp_types::{
    CompletionContext, CompletionItem, CompletionItemKind, CompletionResponse, CompletionTextEdit,
    Documentation, InsertReplaceEdit, Range as LspRange,
};
use sum_tree::Bias;

/// 一条可补全的斜杠命令。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlashCommandItem {
    /// agent 那边的命令名，补全插入的就是它，不做任何改写。
    pub name: String,
    /// 展示用的说明（可为空，就不显示）。
    pub description: String,
    /// 参数提示（协议里是可选的 `input`），有就显示在右侧。
    pub input_hint: Option<String>,
}

impl SlashCommandItem {
    pub fn new(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            input_hint: None,
        }
    }

    pub fn with_input_hint(mut self, hint: impl Into<String>) -> Self {
        let hint = hint.into();
        self.input_hint = (!hint.trim().is_empty()).then_some(hint);
        self
    }

    /// 补全菜单里的标签。
    pub fn completion_label(&self) -> String {
        format!("/{}", self.name)
    }

    /// 插入输入框的文本：命令名后跟一个空格，便于接着写参数。
    pub fn insert_text(&self) -> String {
        format!("/{} ", self.name)
    }
}

/// 通用 `/` 命令补全 provider。
pub struct SlashCompletionProvider {
    items: Arc<Vec<SlashCommandItem>>,
}

impl SlashCompletionProvider {
    pub fn new(items: Vec<SlashCommandItem>) -> Self {
        Self {
            items: Arc::new(items),
        }
    }

    /// 从光标前文本里提取正在输入的命令，返回（`/` 的 offset，query）。
    ///
    /// 只认**整条输入以 `/` 开头**这一种形态：斜杠命令是「这一轮要干什么」而不是句子里的
    /// 一个词，中间冒出来的 `/` 更可能只是路径。已经打了空格说明在写参数，也不再补全。
    pub(crate) fn extract_slash_query(text: &str, offset: usize) -> Option<(usize, String)> {
        let mut offset = offset.min(text.len());
        while offset > 0 && !text.is_char_boundary(offset) {
            offset = offset.saturating_sub(1);
        }
        let rest = text[..offset].strip_prefix('/')?;
        if rest
            .chars()
            .any(|c| c.is_whitespace() || !(c.is_ascii_alphanumeric() || c == '_' || c == '-'))
        {
            return None;
        }
        Some((0, rest.to_string()))
    }
}

impl CompletionProvider for SlashCompletionProvider {
    fn completions(
        &self,
        rope: &Rope,
        offset: usize,
        _trigger: CompletionContext,
        _window: &mut Window,
        cx: &mut App,
    ) -> Task<Result<CompletionResponse>> {
        let rope = rope.clone();
        let items = self.items.clone();

        cx.background_spawn(async move {
            let offset = rope.clip_offset(offset, Bias::Left);
            let text = rope.to_string();
            let Some((start_offset, prefix)) =
                SlashCompletionProvider::extract_slash_query(&text, offset)
            else {
                return Ok(CompletionResponse::Array(vec![]));
            };

            let prefix_lower = prefix.to_lowercase();
            let start_pos = rope.offset_to_position(start_offset);
            let end_pos = rope.offset_to_position(offset);
            let replace_range = LspRange::new(start_pos, end_pos);

            let mut completions = Vec::new();
            for item in items.iter() {
                let name_lower = item.name.to_lowercase();
                if !prefix_lower.is_empty()
                    && !name_lower.contains(&prefix_lower)
                    && !item.description.to_lowercase().contains(&prefix_lower)
                {
                    continue;
                }
                let detail = item
                    .input_hint
                    .clone()
                    .unwrap_or_else(|| item.description.clone());
                let documentation = (!item.description.is_empty())
                    .then(|| Documentation::String(item.description.clone()));
                completions.push(CompletionItem {
                    label: item.completion_label(),
                    kind: Some(CompletionItemKind::FUNCTION),
                    detail: (!detail.is_empty()).then_some(detail),
                    documentation,
                    text_edit: Some(CompletionTextEdit::InsertAndReplace(InsertReplaceEdit {
                        new_text: item.insert_text(),
                        insert: replace_range,
                        replace: replace_range,
                    })),
                    filter_text: (!prefix.is_empty()).then(|| prefix.clone()),
                    sort_text: Some(name_lower),
                    ..Default::default()
                });
            }

            completions.sort_by(|a, b| {
                a.sort_text
                    .as_ref()
                    .unwrap_or(&a.label)
                    .cmp(b.sort_text.as_ref().unwrap_or(&b.label))
            });
            completions.truncate(50);
            Ok(CompletionResponse::Array(completions))
        })
    }

    fn is_completion_trigger(&self, offset: usize, new_text: &str, _cx: &mut App) -> bool {
        // `offset` 是插入后的光标位置：只有 `"/"` 落在整条输入开头时才弹菜单。
        offset <= 1 && new_text.chars().last() == Some('/')
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_empty_query_right_after_slash() {
        assert_eq!(
            Some((0, String::new())),
            SlashCompletionProvider::extract_slash_query("/", 1)
        );
    }

    #[test]
    fn extracts_partial_command_name() {
        assert_eq!(
            Some((0, "cre".to_string())),
            SlashCompletionProvider::extract_slash_query("/cre", 4)
        );
    }

    #[test]
    fn ignores_slash_that_is_not_at_the_start() {
        assert!(
            SlashCompletionProvider::extract_slash_query("看看 src/main", 12).is_none(),
            "mid-text slashes are paths, not commands"
        );
    }

    #[test]
    fn stops_once_arguments_are_being_typed() {
        assert!(SlashCompletionProvider::extract_slash_query("/create a plan", 14).is_none());
    }

    #[test]
    fn insert_text_keeps_the_agent_command_name_verbatim() {
        let item = SlashCommandItem::new("create_plan", "Make a plan")
            .with_input_hint("what to plan");

        assert_eq!("/create_plan", item.completion_label());
        assert_eq!("/create_plan ", item.insert_text());
    }

    #[test]
    fn blank_input_hint_is_dropped() {
        let item = SlashCommandItem::new("x", "d").with_input_hint("   ");
        assert!(item.input_hint.is_none());
    }
}
