//! Mock Provider for Testing
//!
//! Provides a `MockProvider` that implements the `Provider` trait without making real API calls.
//! Useful for downstream libraries (like `rustora`) to write unit tests.
//!
//! # Basic Usage
//!
//! ```rust
//! use llm_connector::{LlmClient, ChatRequest, Message};
//! use llm_connector::providers::mock::MockProvider;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let client = LlmClient::mock("Hello from mock!");
//!
//!     let request = ChatRequest::new("test-model")
//!         .add_message(Message::user("Hi"));
//!
//!     let response = client.chat(&request).await?;
//!     assert_eq!(response.content, "Hello from mock!");
//!     Ok(())
//! }
//! ```
//!
//! # Advanced Usage with MockProviderBuilder
//!
//! ```rust
//! use llm_connector::{LlmClient, ChatRequest, Message, Usage};
//! use llm_connector::providers::mock::MockProviderBuilder;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let client = MockProviderBuilder::new()
//!         .with_content("Mock response")
//!         .with_model("gpt-4-mock")
//!         .with_usage(Usage {
//!             prompt_tokens: 10,
//!             completion_tokens: 5,
//!             total_tokens: 15,
//!             ..Default::default()
//!         })
//!         .build_client();
//!
//!     let response = client.chat(&ChatRequest::new("any")).await?;
//!     assert_eq!(response.content, "Mock response");
//!     assert_eq!(response.model, "gpt-4-mock");
//!     assert_eq!(response.total_tokens(), 15);
//!     Ok(())
//! }
//! ```

use async_trait::async_trait;
use std::any::Any;
use std::sync::{Arc, Mutex};

use crate::core::Provider;
use crate::error::LlmConnectorError;
use crate::types::{
    ChatRequest, ChatResponse, Choice, EmbedRequest, EmbedResponse, EmbeddingData, Message, Role,
    ToolCall, Usage,
};

#[cfg(feature = "streaming")]
use crate::types::ChatStream;

/// A mock provider for testing that returns pre-configured responses.
///
/// Supports two modes:
/// - **Static mode**: Returns the same response for every request
/// - **Sequential mode**: Returns different responses for consecutive requests
pub struct MockProvider {
    responses: Mutex<Vec<Result<ChatResponse, LlmConnectorError>>>,
    default_response: ChatResponse,
    /// Track all requests received (for assertions in tests)
    requests: Mutex<Vec<ChatRequest>>,
}

impl MockProvider {
    /// Create a simple mock that always returns the given content
    pub fn new(content: impl Into<String>) -> Self {
        let content = content.into();
        Self {
            responses: Mutex::new(Vec::new()),
            default_response: Self::make_response(content, None, None),
            requests: Mutex::new(Vec::new()),
        }
    }

    /// Create a mock that returns an error
    pub fn with_error(error: LlmConnectorError) -> Self {
        let mut provider = Self::new("");
        provider.responses = Mutex::new(vec![Err(error)]);
        provider
    }

    /// Create a mock with sequential responses (consumed in order)
    pub fn with_responses(responses: Vec<Result<ChatResponse, LlmConnectorError>>) -> Self {
        Self {
            responses: Mutex::new(responses.into_iter().rev().collect()),
            default_response: Self::make_response("".to_string(), None, None),
            requests: Mutex::new(Vec::new()),
        }
    }

    /// Get all requests that were sent to this mock
    pub fn get_requests(&self) -> Vec<ChatRequest> {
        self.requests.lock().unwrap().clone()
    }

    /// Get the number of requests received
    pub fn request_count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }

    fn make_response(content: String, model: Option<String>, usage: Option<Usage>) -> ChatResponse {
        let message = Message::text(Role::Assistant, &content);
        ChatResponse {
            id: "mock-id".to_string(),
            object: "chat.completion".to_string(),
            created: 0,
            model: model.unwrap_or_else(|| "mock-model".to_string()),
            choices: vec![Choice {
                index: 0,
                message,
                finish_reason: Some("stop".to_string()),
                logprobs: None,
            }],
            content,
            reasoning_content: None,
            usage,
            system_fingerprint: None,
        }
    }

    fn make_tool_call_response(
        tool_calls: Vec<ToolCall>,
        model: Option<String>,
        usage: Option<Usage>,
    ) -> ChatResponse {
        let message = Message::assistant_with_tool_calls(tool_calls);
        let content = message.content_as_text();
        ChatResponse {
            id: "mock-id".to_string(),
            object: "chat.completion".to_string(),
            created: 0,
            model: model.unwrap_or_else(|| "mock-model".to_string()),
            choices: vec![Choice {
                index: 0,
                message,
                finish_reason: Some("tool_calls".to_string()),
                logprobs: None,
            }],
            content,
            reasoning_content: None,
            usage,
            system_fingerprint: None,
        }
    }
}

#[async_trait]
impl Provider for MockProvider {
    fn name(&self) -> &str {
        "mock"
    }

    fn capabilities(&self) -> crate::protocols::common::capabilities::ProviderCapabilities {
        crate::protocols::common::capabilities::ProviderCapabilities::default()
    }

    async fn chat(&self, request: &ChatRequest) -> Result<ChatResponse, LlmConnectorError> {
        self.requests.lock().unwrap().push(request.clone());

        let mut responses = self.responses.lock().unwrap();
        if let Some(response) = responses.pop() {
            response
        } else {
            Ok(self.default_response.clone())
        }
    }

    #[cfg(feature = "streaming")]
    async fn chat_stream(&self, _request: &ChatRequest) -> Result<ChatStream, LlmConnectorError> {
        Err(LlmConnectorError::UnsupportedOperation(
            "MockProvider does not support streaming".to_string(),
        ))
    }

    async fn models(&self) -> Result<Vec<String>, LlmConnectorError> {
        Ok(vec!["mock-model".to_string()])
    }

    async fn embed(&self, request: &EmbedRequest) -> Result<EmbedResponse, LlmConnectorError> {
        Ok(EmbedResponse {
            object: "list".to_string(),
            data: vec![EmbeddingData {
                object: "embedding".to_string(),
                embedding: vec![0.1, 0.2, 0.3],
                index: 0,
            }],
            model: request.model.clone(),
            usage: Usage::default(),
        })
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Builder for creating `MockProvider` with fine-grained control
///
/// # Example
///
/// ```rust
/// use llm_connector::providers::mock::MockProviderBuilder;
/// use llm_connector::LlmConnectorError;
///
/// // Simple text response
/// let client = MockProviderBuilder::new()
///     .with_content("Hello!")
///     .build_client();
///
/// // Sequential responses (first call returns "A", second returns error)
/// let client = MockProviderBuilder::new()
///     .add_response_content("First response")
///     .add_error(LlmConnectorError::RateLimitError("slow down".into()))
///     .add_response_content("Third response")
///     .build_client();
/// ```
pub struct MockProviderBuilder {
    content: Option<String>,
    model: Option<String>,
    usage: Option<Usage>,
    tool_calls: Option<Vec<ToolCall>>,
    responses: Vec<Result<ChatResponse, LlmConnectorError>>,
}

impl MockProviderBuilder {
    pub fn new() -> Self {
        Self {
            content: None,
            model: None,
            usage: None,
            tool_calls: None,
            responses: Vec::new(),
        }
    }

    /// Set the default response content
    pub fn with_content(mut self, content: impl Into<String>) -> Self {
        self.content = Some(content.into());
        self
    }

    /// Set the model name in the response
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    /// Set usage statistics in the response
    pub fn with_usage(mut self, usage: Usage) -> Self {
        self.usage = Some(usage);
        self
    }

    /// Set tool calls in the response (simulates model requesting tool execution)
    pub fn with_tool_calls(mut self, tool_calls: Vec<ToolCall>) -> Self {
        self.tool_calls = Some(tool_calls);
        self
    }

    /// Add a successful text response to the sequential queue
    pub fn add_response_content(mut self, content: impl Into<String>) -> Self {
        let resp =
            MockProvider::make_response(content.into(), self.model.clone(), self.usage.clone());
        self.responses.push(Ok(resp));
        self
    }

    /// Add a successful ChatResponse to the sequential queue
    pub fn add_response(mut self, response: ChatResponse) -> Self {
        self.responses.push(Ok(response));
        self
    }

    /// Add an error response to the sequential queue
    pub fn add_error(mut self, error: LlmConnectorError) -> Self {
        self.responses.push(Err(error));
        self
    }

    /// Build the MockProvider
    pub fn build(self) -> MockProvider {
        if !self.responses.is_empty() {
            MockProvider::with_responses(self.responses)
        } else if let Some(tool_calls) = self.tool_calls {
            let default = MockProvider::make_tool_call_response(
                tool_calls,
                self.model.clone(),
                self.usage.clone(),
            );
            MockProvider {
                responses: Mutex::new(Vec::new()),
                default_response: default,
                requests: Mutex::new(Vec::new()),
            }
        } else {
            let content = self.content.unwrap_or_default();
            let default = MockProvider::make_response(content, self.model, self.usage);
            MockProvider {
                responses: Mutex::new(Vec::new()),
                default_response: default,
                requests: Mutex::new(Vec::new()),
            }
        }
    }

    /// Build and wrap in LlmClient directly
    pub fn build_client(self) -> crate::client::LlmClient {
        let provider = self.build();
        crate::client::LlmClient::from_provider(Arc::new(provider))
    }
}

impl Default for MockProviderBuilder {
    fn default() -> Self {
        Self::new()
    }
}
