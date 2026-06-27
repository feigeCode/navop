//! 模型交互层:[`ModelClient`] 抽象与测试用的 [`MockModelClient`]。

mod client;
mod mock;

pub use client::{ModelClient, ModelRequest, ModelResponse, function_tool_call};
pub use client::{ModelStream, ModelStreamEvent, collect_model_stream, model_response_into_stream};
pub use mock::MockModelClient;

// 重新导出运行时 API 中会用到的 llm-connector 类型,调用方无需直接依赖该 crate。
pub use llm_connector::types::{Function, FunctionCall, Message, Role, Tool, ToolCall, ToolChoice};
