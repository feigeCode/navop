//! 运行中可追加的用户输入。

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

/// 一张随用户消息附带的图片(供视觉模型直传)。
///
/// 运行时只持有 base64 数据与 MIME 类型,既不依赖 GPUI 也不持有原始字节缓冲——
/// UI 层在构造输入时负责把图片编码为 base64。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputImage {
    /// 图片 MIME 类型,如 `image/png`。
    pub mime: String,
    /// base64 编码后的图片数据(不含 `data:` 前缀)。
    pub data_base64: String,
}

impl InputImage {
    pub fn new(mime: impl Into<String>, data_base64: impl Into<String>) -> Self {
        Self {
            mime: mime.into(),
            data_base64: data_base64.into(),
        }
    }
}

/// 一条用户输入:文本 + 可选附带图片。
#[derive(Clone, Debug, Default)]
pub struct UserInput {
    pub text: String,
    pub images: Vec<InputImage>,
}

impl UserInput {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            images: Vec::new(),
        }
    }

    /// 附带一组图片。
    pub fn with_images(mut self, images: Vec<InputImage>) -> Self {
        self.images = images;
        self
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn images(&self) -> &[InputImage] {
        &self.images
    }
}

impl From<&str> for UserInput {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for UserInput {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

/// 一轮交互的输入项。
#[derive(Clone, Debug)]
pub enum TurnInput {
    /// 普通用户消息。
    User(UserInput),
}

impl TurnInput {
    pub fn as_text(&self) -> &str {
        match self {
            TurnInput::User(input) => input.text(),
        }
    }

    /// 该输入项附带的图片。
    pub fn images(&self) -> &[InputImage] {
        match self {
            TurnInput::User(input) => input.images(),
        }
    }
}

impl From<UserInput> for TurnInput {
    fn from(value: UserInput) -> Self {
        TurnInput::User(value)
    }
}

/// 运行中可追加输入的队列。当某一轮正在执行时,新到达的用户输入先入队,
/// 由当前任务在合适的时机消费(第一版任务在一轮结束后检查队列)。
#[derive(Default)]
pub struct InputQueue {
    pending: VecDeque<TurnInput>,
}

impl InputQueue {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, input: TurnInput) {
        self.pending.push_back(input);
    }

    /// 取出并清空全部待处理输入。
    pub fn drain(&mut self) -> Vec<TurnInput> {
        self.pending.drain(..).collect()
    }

    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    pub fn len(&self) -> usize {
        self.pending.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }
}
