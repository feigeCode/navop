//! 通用 `@` 提及补全。
//!
//! 这是 AgentInput 的扩展点:调用方注入一组可被 `@` 引用的条目([`MentionItem`]),
//! 用户在输入框键入 `@` 时弹出补全菜单。本机制**不绑定任何业务**——条目可以是数据库
//! 连接、SSH 主机、文件、表名等,由上层决定。
//!
//! 实现复刻了 `db_view` 中 `TableMentionCompletionProvider` 的 `@` 检测与插入逻辑,
//! 但把数据来源抽象为通用的 [`MentionItem`] 列表。

use std::sync::Arc;

use anyhow::Result;
use gpui::{AppContext, Context, Task, Window};
use gpui_component::input::{CompletionProvider, InputState};
use gpui_component::{Rope, RopeExt};
use lsp_types::{
    CompletionContext, CompletionItem, CompletionItemKind, CompletionResponse, CompletionTextEdit,
    Documentation, InsertReplaceEdit, Range as LspRange,
};
use sum_tree::Bias;

/// 可被 `@` 引用的一个条目。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MentionItem {
    /// 稳定标识(如 connection_id);供上层在提交时解析引用。
    pub id: String,
    /// 显示与插入用的标签(插入文本为 `@label`)。
    pub label: String,
    /// 次要说明(类型、地址等),展示在补全项右侧。
    pub detail: String,
    /// 分类标识(如 `mysql` / `ssh` / `file`),用于图标 / 分组(可空)。
    pub kind: String,
}

impl MentionItem {
    pub fn new(
        id: impl Into<String>,
        label: impl Into<String>,
        detail: impl Into<String>,
        kind: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            detail: detail.into(),
            kind: kind.into(),
        }
    }

    /// 插入到输入框的提及文本(末尾带空格,便于继续输入)。
    pub fn mention_text(&self) -> String {
        if is_simple_name(&self.label) {
            format!("@{} ", self.label)
        } else if !self.label.contains('`') {
            format!("@`{}` ", self.label)
        } else {
            format!("@\"{}\" ", self.label)
        }
    }
}

/// 通用 `@` 提及补全 provider。
pub struct MentionCompletionProvider {
    items: Arc<Vec<MentionItem>>,
}

impl MentionCompletionProvider {
    pub fn new(items: Vec<MentionItem>) -> Self {
        Self {
            items: Arc::new(items),
        }
    }

    /// 从光标前文本中提取正在输入的 `@query`,返回 (`@` 起始 offset, query)。
    ///
    /// 仅当 `@` 处于词首(前面不是字母数字 / 下划线)且 query 仅由合法字符组成时命中。
    pub(crate) fn extract_mention_query(text: &str, offset: usize) -> Option<(usize, String)> {
        let mut offset = offset.min(text.len());
        while offset > 0 && !text.is_char_boundary(offset) {
            offset = offset.saturating_sub(1);
        }
        let before_cursor = &text[..offset];
        let at_index = before_cursor.rfind('@')?;
        if at_index > 0 {
            let before_at = before_cursor[..at_index].chars().last();
            if before_at.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                return None;
            }
        }
        let after_at = &before_cursor[at_index + 1..];
        if after_at.is_empty() {
            return Some((at_index, String::new()));
        }
        let first = after_at.chars().next()?;
        if first == '`' || first == '"' {
            let rest = &after_at[first.len_utf8()..];
            if rest.contains(first) {
                return None;
            }
            return Some((at_index, rest.to_string()));
        }
        if !after_at.chars().all(|c| c.is_alphanumeric() || c == '_') {
            return None;
        }
        Some((at_index, after_at.to_string()))
    }
}

impl CompletionProvider for MentionCompletionProvider {
    fn completions(
        &self,
        rope: &Rope,
        offset: usize,
        _trigger: CompletionContext,
        _window: &mut Window,
        cx: &mut Context<InputState>,
    ) -> Task<Result<CompletionResponse>> {
        let rope = rope.clone();
        let items = self.items.clone();

        cx.background_spawn(async move {
            let offset = rope.clip_offset(offset, Bias::Left);
            let text = rope.to_string();
            let Some((start_offset, prefix)) =
                MentionCompletionProvider::extract_mention_query(&text, offset)
            else {
                return Ok(CompletionResponse::Array(vec![]));
            };

            let prefix_lower = prefix.to_lowercase();
            let start_pos = rope.offset_to_position(start_offset);
            let end_pos = rope.offset_to_position(offset);
            let replace_range = LspRange::new(start_pos, end_pos);

            let mut completions = Vec::new();
            for item in items.iter() {
                let label_lower = item.label.to_lowercase();
                if !prefix_lower.is_empty() && !label_lower.contains(&prefix_lower) {
                    continue;
                }
                let mention_text = item.mention_text();
                let documentation = if item.detail.is_empty() {
                    None
                } else {
                    Some(Documentation::String(item.detail.clone()))
                };
                completions.push(CompletionItem {
                    label: format!("@{}", item.label),
                    kind: Some(CompletionItemKind::REFERENCE),
                    detail: (!item.detail.is_empty()).then(|| item.detail.clone()),
                    documentation,
                    text_edit: Some(CompletionTextEdit::InsertAndReplace(InsertReplaceEdit {
                        new_text: mention_text,
                        insert: replace_range,
                        replace: replace_range,
                    })),
                    filter_text: (!prefix.is_empty()).then(|| prefix.clone()),
                    sort_text: Some(label_lower),
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

    fn is_completion_trigger(
        &self,
        _offset: usize,
        new_text: &str,
        _cx: &mut Context<InputState>,
    ) -> bool {
        new_text.chars().last().is_some_and(|c| !c.is_whitespace())
    }
}

/// 是否为可直接插入(无需引号)的简单名称:首字符为字母 / 下划线,其余为字母数字 / 下划线。
fn is_simple_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_alphabetic() || first == '_') {
        return false;
    }
    chars.all(|c| c.is_alphanumeric() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_empty_query_right_after_at() {
        let got = MentionCompletionProvider::extract_mention_query("hello @", 7);
        assert_eq!(got, Some((6, String::new())));
    }

    #[test]
    fn extracts_partial_query() {
        let got = MentionCompletionProvider::extract_mention_query("see @prod", 9);
        assert_eq!(got, Some((4, "prod".to_string())));
    }

    #[test]
    fn no_mention_when_at_follows_word_char() {
        // `a@b` 中的 `@` 紧跟字母,不视为提及(类似邮箱)。
        assert!(MentionCompletionProvider::extract_mention_query("a@b", 3).is_none());
    }

    #[test]
    fn simple_name_inserts_without_quotes() {
        let item = MentionItem::new("c1", "prod_db", "mysql", "mysql");
        assert_eq!(item.mention_text(), "@prod_db ");
    }

    #[test]
    fn name_with_space_is_quoted() {
        let item = MentionItem::new("c2", "my db", "mysql", "mysql");
        assert_eq!(item.mention_text(), "@`my db` ");
    }
}
