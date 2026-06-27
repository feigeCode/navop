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
use std::pin::Pin;

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
}

impl ModelResponse {
    /// 构造纯文本响应。
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            text: Some(text.into()),
            tool_calls: Vec::new(),
        }
    }

    /// 构造仅含工具调用的响应。
    pub fn tool_calls(tool_calls: Vec<ToolCall>) -> Self {
        Self {
            text: None,
            tool_calls,
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
    let mut events: Vec<Result<ModelStreamEvent, RuntimeError>> = Vec::new();
    if let Some(text) = &response.text
        && !text.is_empty()
    {
        events.push(Ok(ModelStreamEvent::TextDelta(text.clone())));
    }
    for call in &response.tool_calls {
        events.push(Ok(ModelStreamEvent::ToolCall(call.clone())));
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
    let mut completed: Option<ModelResponse> = None;
    while let Some(event) = stream.next().await {
        match event? {
            ModelStreamEvent::TextDelta(delta) => text.push_str(&delta),
            ModelStreamEvent::ReasoningDelta(_) => {}
            ModelStreamEvent::ToolCall(call) => tool_calls.push(call),
            ModelStreamEvent::Completed(response) => completed = Some(response),
        }
    }
    Ok(completed.unwrap_or(ModelResponse {
        text: (!text.is_empty()).then_some(text),
        tool_calls,
    }))
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
