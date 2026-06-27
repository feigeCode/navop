//! 把会话历史转换为模型消息。
//!
//! 第一版用"文字转录"的方式表达工具调用与观测(assistant / user 文本),而不
//! 严格复刻各家 provider 的 tool_call / tool_result 配对协议——后者跨 provider
//! 约束较多。转录方式简单、稳定,模型据此即可还原上下文做决策。

use crate::history::{HistoryItem, RuntimeHistory};
use llm_connector::types::{Message, MessageBlock, Role};

/// 将历史转换为消息序列(不含 system 提示,由调用方在最前面拼接)。
pub fn history_to_messages(history: &RuntimeHistory) -> Vec<Message> {
    let max = history.max_observation_bytes();
    let mut messages = Vec::with_capacity(history.len());
    for item in history.items() {
        match item {
            HistoryItem::User { text, images } => {
                if images.is_empty() {
                    messages.push(Message::user(text.clone()));
                } else {
                    messages.push(user_message_with_images(text, images));
                }
            }
            HistoryItem::Assistant(text) => messages.push(Message::assistant(text.clone())),
            HistoryItem::System(text) => messages.push(Message::system(text.clone())),
            HistoryItem::ToolCall(call) => {
                messages.push(Message::assistant(format!(
                    "我调用工具 `{}`,参数: {}",
                    call.tool_name, call.arguments
                )));
            }
            HistoryItem::Observation(obs) => {
                messages.push(Message::user(format!(
                    "工具 `{}` 的执行结果:\n{}",
                    obs.tool_name,
                    obs.model_text(max)
                )));
            }
        }
    }
    messages
}

/// 构造含图片的多模态用户消息(文本块 + 各图片的 base64 块)。
fn user_message_with_images(text: &str, images: &[crate::runtime::InputImage]) -> Message {
    let mut blocks: Vec<MessageBlock> = Vec::with_capacity(images.len() + 1);
    if !text.is_empty() {
        blocks.push(MessageBlock::text(text));
    }
    for image in images {
        blocks.push(MessageBlock::image_base64(
            image.mime.clone(),
            image.data_base64.clone(),
        ));
    }
    Message::new(Role::User, blocks)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::RuntimeHistory;
    use crate::runtime::InputImage;

    #[test]
    fn plain_user_history_becomes_text_message() {
        let mut history = RuntimeHistory::new();
        history.record_user("你好");
        let messages = history_to_messages(&history);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, Role::User);
        assert_eq!(messages[0].content.len(), 1);
        assert!(messages[0].content[0].is_text());
    }

    #[test]
    fn user_history_with_images_becomes_multimodal_message() {
        let mut history = RuntimeHistory::new();
        history.record_user_with_images("看这张图", vec![InputImage::new("image/png", "AAAABBBB")]);
        let messages = history_to_messages(&history);
        assert_eq!(messages.len(), 1);
        let msg = &messages[0];
        assert_eq!(msg.role, Role::User);
        // 文本块 + 图片块。
        assert_eq!(msg.content.len(), 2);
        assert!(msg.content[0].is_text());
        assert!(!msg.content[1].is_text(), "第二块应为图片块");
    }
}
