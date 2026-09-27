//! 模型客户端抽象。
//!
//! 运行时通过 [`ModelClient`] 与大模型交互。这里**不直接依赖** onetcli 的
//! `LlmProvider`,而是定义自己的窄接口:输入消息 + 工具规格,输出文本或
//! 工具调用。这样可以用 [`super::MockModelClient`] 做确定性单元测试;真实环境
//! 下由一个适配器把 `core::llm::LlmProvider` 包装成 `ModelClient`。
//!
//! 复用 `llm-connector` 的 [`Message`] / [`Tool`] / [`ToolCall`] / [`ToolChoice`]
//! 类型,避免重复造消息模型,也方便与现有 LLM 层对接。

use crate::error::RuntimeError;
use async_trait::async_trait;
use futures::{Stream, StreamExt};
use llm_connector::types::{FunctionCall, Message, Tool, ToolCall, ToolChoice};
use serde::{Deserialize, Serialize};
use std::pin::Pin;

/// 一次模型采样的 token 计量,由 provider 响应携带;不报告时为 `None`。
///
/// `total_tokens` 个别 provider 只填部分字段,消费方应做 `max` 兜底
/// (见 [`crate::runtime::Session::record_token_usage`])。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenUsage {
    /// 输入(提示词)token 数——近似等于当前上下文占用。
    pub prompt_tokens: u64,
    /// 输出(补全)token 数。
    pub completion_tokens: u64,
    /// 总数。provider 未填时可能为 0。
    pub total_tokens: u64,
}

impl TokenUsage {
    /// 这一次请求过后的上下文占用估算:输入 + 输出。
    ///
    /// 比 `total_tokens` 更可靠——个别 provider 只填 `prompt_tokens`。
    pub fn context_tokens(&self) -> u64 {
        self.total_tokens
            .max(self.prompt_tokens.saturating_add(self.completion_tokens))
    }
}

/// 一次模型采样请求。
#[derive(Debug, Clone, Default)]
pub struct ModelRequest {
    /// 完整对话消息(system / user / assistant / tool)。
    pub messages: Vec<Message>,
    /// 本次允许调用的工具规格。
    pub tools: Vec<Tool>,
    /// 工具选择策略(auto / required / 指定函数)。
    pub tool_choice: Option<ToolChoice>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
}

impl ModelRequest {
    pub fn new(messages: Vec<Message>) -> Self {
        Self {
            messages,
            ..Default::default()
        }
    }

    pub fn with_tools(mut self, tools: Vec<Tool>) -> Self {
        self.tools = tools;
        self
    }

    pub fn with_tool_choice(mut self, choice: ToolChoice) -> Self {
        self.tool_choice = Some(choice);
        self
    }

    pub fn with_temperature(mut self, temperature: f32) -> Self {
        self.temperature = Some(temperature);
        self
    }

    pub fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = Some(max_tokens);
        self
    }
}

/// 一次模型采样结果。
///
/// 要么是纯文本回答,要么包含一个或多个工具调用(也可能两者皆有:模型先给
/// 一段说明再调用工具)。
#[derive(Debug, Clone, Default)]
pub struct ModelResponse {
    /// 助手文本内容(可能为空)。
    pub text: Option<String>,
    /// 模型请求的工具调用(可能为空)。
    pub tool_calls: Vec<ToolCall>,
    /// 本次采样的 token 计量;provider 不报告时为 `None`。
    pub usage: Option<TokenUsage>,
}

impl ModelResponse {
    /// 构造纯文本响应。
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            text: Some(text.into()),
            tool_calls: Vec::new(),
            usage: None,
        }
    }

    /// 构造仅含工具调用的响应。
    pub fn tool_calls(tool_calls: Vec<ToolCall>) -> Self {
        Self {
            text: None,
            tool_calls,
            usage: None,
        }
    }

    /// 构造单个工具调用响应。
    pub fn tool_call(call: ToolCall) -> Self {
        Self::tool_calls(vec![call])
    }

    pub fn has_tool_calls(&self) -> bool {
        !self.tool_calls.is_empty()
    }

    pub fn first_tool_call(&self) -> Option<&ToolCall> {
        self.tool_calls.first()
    }
}

/// 模型流式事件。对齐 codex 的 `ResponseEvent`,但只保留运行时需要的子集。
#[derive(Debug, Clone)]
pub enum ModelStreamEvent {
    /// 助手可见文本增量(逐段输出)。
    TextDelta(String),
    /// 推理 / 思考增量(部分模型提供;运行时可选择是否展示)。
    ReasoningDelta(String),
    /// 一个(已累积完整的)工具调用。
    ToolCall(ToolCall),
    /// 一次采样的 token 计量。流式 provider 在最终 chunk 上报告;
    /// 供消费方在流结束前就地记账,`Completed` 也会携带同一份数据。
    Usage(TokenUsage),
    /// 流结束,携带最终聚合结果(完整文本 + 全部工具调用)。
    Completed(ModelResponse),
}

/// 模型流。每个元素是一个 [`ModelStreamEvent`] 或错误。
pub type ModelStream = Pin<Box<dyn Stream<Item = Result<ModelStreamEvent, RuntimeError>> + Send>>;

/// 模型客户端接口。实现者负责把 [`ModelRequest`] 发送给某个具体后端并返回结果。
#[async_trait]
pub trait ModelClient: Send + Sync {
    /// 执行一次(非流式)采样。
    async fn complete(&self, request: ModelRequest) -> Result<ModelResponse, RuntimeError>;

    /// 执行一次流式采样。
    ///
    /// 默认实现基于 [`ModelClient::complete`] 退化:先取完整结果,再依次产出
    /// 文本增量、工具调用与 `Completed`。因此仅实现 `complete` 的客户端也能用于
    /// 流式任务(只是不会有逐段输出)。真实模型适配器应重写本方法接入底层流。
    async fn complete_stream(&self, request: ModelRequest) -> Result<ModelStream, RuntimeError> {
        let response = self.complete(request).await?;
        Ok(model_response_into_stream(response))
    }

    /// 当前使用的模型名,仅用于日志 / 事件展示。
    fn model_name(&self) -> &str {
        "unknown"
    }
}

/// 把一个完整 [`ModelResponse`] 转换为单步流(文本一次性产出 + 工具调用 + Completed)。
pub fn model_response_into_stream(response: ModelResponse) -> ModelStream {
    let usage = response.usage;
    let mut events: Vec<Result<ModelStreamEvent, RuntimeError>> = Vec::new();
    if let Some(text) = &response.text
        && !text.is_empty()
    {
        events.push(Ok(ModelStreamEvent::TextDelta(text.clone())));
    }
    for call in &response.tool_calls {
        events.push(Ok(ModelStreamEvent::ToolCall(call.clone())));
    }
    // 非流式响应退化成流时,计量同样以 Usage 事件先行——消费方无需区分来源。
    if let Some(usage) = usage {
        events.push(Ok(ModelStreamEvent::Usage(usage)));
    }
    events.push(Ok(ModelStreamEvent::Completed(response)));
    Box::pin(futures::stream::iter(events))
}

/// 消费一个 [`ModelStream`] 并聚合为 [`ModelResponse`]。
///
/// 真实适配器可借此用 `complete_stream` 实现 `complete`:累积文本增量与工具调用,
/// 若收到 `Completed` 则以其为准。
pub async fn collect_model_stream(mut stream: ModelStream) -> Result<ModelResponse, RuntimeError> {
    let mut text = String::new();
    let mut tool_calls = Vec::new();
    let mut usage: Option<TokenUsage> = None;
    let mut completed: Option<ModelResponse> = None;
    while let Some(event) = stream.next().await {
        match event? {
            ModelStreamEvent::TextDelta(delta) => text.push_str(&delta),
            ModelStreamEvent::ReasoningDelta(_) => {}
            ModelStreamEvent::ToolCall(call) => tool_calls.push(call),
            ModelStreamEvent::Usage(reported) => usage = Some(reported),
            ModelStreamEvent::Completed(response) => {
                // Completed 自带的计量优先;适配器两边都发时是同一份数据。
                usage = usage.or(response.usage);
                completed = Some(response);
            }
        }
    }
    // Completed 的计量优先,但流中单独报的 Usage 事件也要合并——
    // 适配器两边都发时是同一份数据,只报一边时不能丢。
    let mut response = completed.unwrap_or(ModelResponse {
        text: (!text.is_empty()).then_some(text),
        tool_calls,
        usage: None,
    });
    response.usage = response.usage.or(usage);
    Ok(response)
}

/// 构造一个 function-calling 工具调用。
///
/// 供测试与真实模型适配器复用:`arguments` 为 JSON 字符串。
pub fn function_tool_call(
    id: impl Into<String>,
    name: impl Into<String>,
    arguments: impl Into<String>,
) -> ToolCall {
    ToolCall {
        id: id.into(),
        call_type: "function".to_string(),
        function: FunctionCall {
            name: name.into(),
            arguments: arguments.into(),
            ..Default::default()
        },
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;

    fn sample_usage() -> TokenUsage {
        TokenUsage {
            prompt_tokens: 900,
            completion_tokens: 100,
            total_tokens: 1000,
        }
    }

    #[test]
    fn context_tokens_prefers_the_larger_of_total_and_parts() {
        // provider 只填部分字段时,输入+输出是更可靠的占用估算。
        let partial = TokenUsage {
            prompt_tokens: 700,
            completion_tokens: 80,
            total_tokens: 0,
        };
        assert_eq!(partial.context_tokens(), 780);
        assert_eq!(sample_usage().context_tokens(), 1000);
    }

    #[tokio::test]
    async fn completed_response_degrades_into_a_usage_event_before_completed() {
        let response = ModelResponse {
            text: Some("答案".into()),
            tool_calls: Vec::new(),
            usage: Some(sample_usage()),
        };
        let mut stream = model_response_into_stream(response);
        let mut events = Vec::new();
        while let Some(event) = stream.next().await {
            events.push(event.expect("退化流不应出错"));
        }
        let usage_index = events
            .iter()
            .position(
                |event| matches!(event, ModelStreamEvent::Usage(usage) if *usage == sample_usage()),
            )
            .expect("应在 Completed 前产出 Usage 事件");
        let completed_index = events
            .iter()
            .position(|event| matches!(event, ModelStreamEvent::Completed(_)))
            .expect("流应以 Completed 结束");
        assert!(usage_index < completed_index);
    }

    #[tokio::test]
    async fn collect_model_stream_aggregates_usage_from_the_event() {
        let usage = sample_usage();
        let events = vec![
            Ok(ModelStreamEvent::TextDelta("部分".into())),
            Ok(ModelStreamEvent::TextDelta("回答".into())),
            Ok(ModelStreamEvent::Usage(usage)),
            Ok(ModelStreamEvent::Completed(ModelResponse {
                text: Some("完整回答".into()),
                tool_calls: Vec::new(),
                usage: None,
            })),
        ];
        let response = collect_model_stream(Box::pin(futures::stream::iter(events)))
            .await
            .expect("聚合不应失败");
        assert_eq!(response.text.as_deref(), Some("完整回答"));
        assert_eq!(response.usage, Some(usage));
    }

    #[tokio::test]
    async fn collect_model_stream_without_usage_reports_none() {
        let events = vec![Ok(ModelStreamEvent::Completed(ModelResponse {
            text: Some("回答".into()),
            tool_calls: Vec::new(),
            usage: None,
        }))];
        let response = collect_model_stream(Box::pin(futures::stream::iter(events)))
            .await
            .expect("聚合不应失败");
        assert_eq!(response.usage, None);
    }
}
