//! 测试用的脚本化模型客户端。
//!
//! 按入队顺序依次返回预设的 [`ModelResponse`],并记录收到的全部
//! [`ModelRequest`] 以便断言。它让整条 Runtime / Planner 闭环可以脱离真实模型
//! 做确定性测试。

use super::client::{ModelClient, ModelRequest, ModelResponse, ModelStream, ModelStreamEvent};
use crate::error::RuntimeError;
use async_trait::async_trait;
use std::collections::VecDeque;
use std::sync::Mutex;

/// 脚本化模型:每次 `complete` 弹出队首响应;耗尽后返回错误。
pub struct MockModelClient {
    responses: Mutex<VecDeque<ModelResponse>>,
    requests: Mutex<Vec<ModelRequest>>,
    model_name: String,
}

impl MockModelClient {
    /// 用一组预设响应构造。
    pub fn new(responses: impl IntoIterator<Item = ModelResponse>) -> Self {
        Self {
            responses: Mutex::new(responses.into_iter().collect()),
            requests: Mutex::new(Vec::new()),
            model_name: "mock".to_string(),
        }
    }

    /// 追加一个响应到队尾。
    pub fn push(&self, response: ModelResponse) {
        self.responses
            .lock()
            .expect("mock 锁中毒")
            .push_back(response);
    }

    /// 返回截至目前收到的全部请求(克隆)。
    pub fn received_requests(&self) -> Vec<ModelRequest> {
        self.requests.lock().expect("mock 锁中毒").clone()
    }

    /// 已收到的请求数量。
    pub fn request_count(&self) -> usize {
        self.requests.lock().expect("mock 锁中毒").len()
    }
}

#[async_trait]
impl ModelClient for MockModelClient {
    async fn complete(&self, request: ModelRequest) -> Result<ModelResponse, RuntimeError> {
        self.requests.lock().expect("mock 锁中毒").push(request);
        self.responses
            .lock()
            .expect("mock 锁中毒")
            .pop_front()
            .ok_or_else(|| RuntimeError::model("mock 模型预设响应已耗尽"))
    }

    /// 把队首响应的文本切成多段 `TextDelta` 后再产出,用于确定性地测试流式路径。
    async fn complete_stream(&self, request: ModelRequest) -> Result<ModelStream, RuntimeError> {
        self.requests.lock().expect("mock 锁中毒").push(request);
        let response = self
            .responses
            .lock()
            .expect("mock 锁中毒")
            .pop_front()
            .ok_or_else(|| RuntimeError::model("mock 模型预设响应已耗尽"))?;

        let mut events: Vec<Result<ModelStreamEvent, RuntimeError>> = Vec::new();
        if let Some(text) = &response.text {
            for chunk in split_into_chunks(text, 3) {
                events.push(Ok(ModelStreamEvent::TextDelta(chunk)));
            }
        }
        for call in &response.tool_calls {
            events.push(Ok(ModelStreamEvent::ToolCall(call.clone())));
        }
        events.push(Ok(ModelStreamEvent::Completed(response)));
        Ok(Box::pin(futures::stream::iter(events)))
    }

    fn model_name(&self) -> &str {
        &self.model_name
    }
}

/// 把文本按字符切成若干块,用于模拟流式分段输出。
fn split_into_chunks(text: &str, chunk_chars: usize) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    chars
        .chunks(chunk_chars.max(1))
        .map(|c| c.iter().collect())
        .collect()
}
