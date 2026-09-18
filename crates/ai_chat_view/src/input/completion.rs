//! 组合补全：`@` 提及 + `/` 命令共用一个 `CompletionProvider`。
//!
//! gpui-component 的编辑器只挂一个 provider，而这两套触发条件互斥（`@` 出现在词首、
//! `/` 出现在整条输入开头），所以在这一层合并而不是往各自的 provider 里塞对方的逻辑。

use anyhow::Result;
use gpui::{App, Task, Window};
use gpui_component::input::CompletionProvider;
use gpui_component::Rope;
use lsp_types::{CompletionContext, CompletionResponse};

use crate::input::mention::{MentionCompletionProvider, MentionItem};
use crate::input::slash::{SlashCommandItem, SlashCompletionProvider};

/// 输入框实际挂载的补全 provider。
pub(crate) struct ComposerCompletionProvider {
    mentions: MentionCompletionProvider,
    slash: SlashCompletionProvider,
}

impl ComposerCompletionProvider {
    pub(crate) fn new(mentions: Vec<MentionItem>, commands: Vec<SlashCommandItem>) -> Self {
        Self {
            mentions: MentionCompletionProvider::new(mentions),
            slash: SlashCompletionProvider::new(commands),
        }
    }
}

impl CompletionProvider for ComposerCompletionProvider {
    fn completions(
        &self,
        rope: &Rope,
        offset: usize,
        trigger: CompletionContext,
        window: &mut Window,
        cx: &mut App,
    ) -> Task<Result<CompletionResponse>> {
        // 斜杠命令形态更严格（必须在输入开头），所以它先判：命中就只给命令，
        // 不会掉进提及分支。
        let text = rope.to_string();
        if SlashCompletionProvider::extract_slash_query(&text, offset).is_some() {
            return self.slash.completions(rope, offset, trigger, window, cx);
        }
        self.mentions.completions(rope, offset, trigger, window, cx)
    }

    fn is_completion_trigger(&self, offset: usize, new_text: &str, cx: &mut App) -> bool {
        self.slash.is_completion_trigger(offset, new_text, cx)
            || self.mentions.is_completion_trigger(offset, new_text, cx)
    }
}
