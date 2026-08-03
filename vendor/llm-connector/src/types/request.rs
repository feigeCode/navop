//! Request types for chat completions

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

fn deserialize_null_as_empty_string<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<String>::deserialize(deserializer).map(Option::unwrap_or_default)
}

/// Role of a message sender
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// System message (instructions)
    System,
    /// User message
    User,
    /// Assistant message
    Assistant,
    /// Tool response message
    Tool,
}

/// Reasoning effort for models that support it (e.g. OpenAI o1)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReasoningEffort {
    Low,
    Medium,
    High,
}

/// Chat completion request
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ChatRequest {
    /// Model identifier (e.g., "openai/gpt-4", "deepseek/deepseek-chat")
    pub model: String,

    /// List of messages in the conversation
    pub messages: Vec<Message>,

    /// Sampling temperature (0.0 to 2.0)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,

    /// Nucleus sampling parameter (0.0 to 1.0)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,

    /// Maximum number of tokens to generate
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,

    /// Whether to stream the response
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,

    /// Stop sequences
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop: Option<Vec<String>>,

    /// Presence penalty (-2.0 to 2.0)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presence_penalty: Option<f32>,

    /// Frequency penalty (-2.0 to 2.0)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frequency_penalty: Option<f32>,

    /// Logit bias
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logit_bias: Option<HashMap<String, f32>>,

    /// User identifier
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,

    /// Random seed for deterministic outputs
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,

    /// Tools available to the model
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<Tool>>,

    /// Anthropic-native tools that should be passed through without OpenAI-style normalization
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anthropic_tools: Option<Vec<AnthropicToolDefinition>>,

    /// Tool choice strategy
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<ToolChoice>,

    /// Anthropic-native tool choice that should be passed through without lossy conversion
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anthropic_tool_choice: Option<AnthropicToolChoice>,

    /// Response format specification
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_format: Option<ResponseFormat>,

    /// Enable thinking/reasoning mode (provider-specific)
    ///
    /// For Aliyun: Enables reasoning content for hybrid models like qwen-plus
    /// For other providers: May be ignored
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enable_thinking: Option<bool>,

    /// Reasoning effort (e.g., for O1)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<ReasoningEffort>,

    /// Thinking budget in tokens (e.g., for Claude 3.7)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_budget: Option<u32>,

    /// Per-request API key override (for multi-tenant routing)
    ///
    /// When set, overrides the client's API key for this request.
    /// Applied to both `Authorization: Bearer` and `x-api-key` headers.
    #[serde(skip_serializing)]
    pub api_key: Option<String>,

    /// Per-request base URL override (for multi-tenant routing)
    ///
    /// When set, overrides the client's base URL for this request.
    #[serde(skip_serializing)]
    pub base_url: Option<String>,

    /// Per-request custom headers (e.g. `X-Trace-Id`, `anthropic-version`)
    ///
    /// When set, these headers are merged with the request. Values here
    /// override default provider headers for the same keys.
    #[serde(skip_serializing)]
    pub extra_headers: Option<HashMap<String, String>>,
}

impl ChatRequest {
    /// Create a new chat request with the given model
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            ..Default::default()
        }
    }

    /// Create a new chat request with model and initial messages
    pub fn new_with_messages(model: impl Into<String>, messages: Vec<Message>) -> Self {
        Self {
            model: model.into(),
            messages,
            ..Default::default()
        }
    }

    /// Set the messages for the request
    pub fn with_messages(mut self, messages: Vec<Message>) -> Self {
        self.messages = messages;
        self
    }

    /// Add a single message to the request
    pub fn add_message(mut self, message: Message) -> Self {
        self.messages.push(message);
        self
    }

    /// Add a content block to the last message (or create new user message)
    ///
    /// If the last message is a User message, appends the block to it.
    /// Otherwise, creates a new User message with this block.
    pub fn add_message_block(mut self, block: super::message_block::MessageBlock) -> Self {
        if let Some(last) = self.messages.last_mut()
            && last.role == Role::User
        {
            last.content.push(block);
            return self;
        }
        self.messages.push(Message::new(Role::User, vec![block]));
        self
    }

    /// Set the temperature
    pub fn with_temperature(mut self, temperature: f32) -> Self {
        self.temperature = Some(temperature);
        self
    }

    /// Set the top_p parameter
    pub fn with_top_p(mut self, top_p: f32) -> Self {
        self.top_p = Some(top_p);
        self
    }

    /// Set the maximum number of tokens
    pub fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = Some(max_tokens);
        self
    }

    /// Enable streaming
    pub fn with_stream(mut self, stream: bool) -> Self {
        self.stream = Some(stream);
        self
    }

    /// Set stop sequences
    pub fn with_stop(mut self, stop: Vec<String>) -> Self {
        self.stop = Some(stop);
        self
    }

    /// Set presence penalty
    pub fn with_presence_penalty(mut self, penalty: f32) -> Self {
        self.presence_penalty = Some(penalty);
        self
    }

    /// Set frequency penalty
    pub fn with_frequency_penalty(mut self, penalty: f32) -> Self {
        self.frequency_penalty = Some(penalty);
        self
    }

    /// Set the user identifier
    pub fn with_user(mut self, user: impl Into<String>) -> Self {
        self.user = Some(user.into());
        self
    }

    /// Set the random seed
    pub fn with_seed(mut self, seed: u64) -> Self {
        self.seed = Some(seed);
        self
    }

    /// Set the tools
    pub fn with_tools(mut self, tools: Vec<Tool>) -> Self {
        self.tools = Some(tools);
        self
    }

    /// Set Anthropic-native tools
    pub fn with_anthropic_tools(mut self, tools: Vec<AnthropicToolDefinition>) -> Self {
        self.anthropic_tools = Some(tools);
        self
    }

    /// Set the tool choice
    pub fn with_tool_choice(mut self, tool_choice: ToolChoice) -> Self {
        self.tool_choice = Some(tool_choice);
        self
    }

    /// Set the Anthropic-native tool choice
    pub fn with_anthropic_tool_choice(mut self, tool_choice: AnthropicToolChoice) -> Self {
        self.anthropic_tool_choice = Some(tool_choice);
        self
    }

    /// Set the response format
    pub fn with_response_format(mut self, format: ResponseFormat) -> Self {
        self.response_format = Some(format);
        self
    }

    /// Enable thinking/reasoning mode
    ///
    /// For Aliyun: Enables reasoning content for hybrid models
    pub fn with_enable_thinking(mut self, enable: bool) -> Self {
        self.enable_thinking = Some(enable);
        self
    }

    /// Set the reasoning effort
    pub fn with_reasoning_effort(mut self, effort: ReasoningEffort) -> Self {
        self.reasoning_effort = Some(effort);
        self
    }

    /// Set the thinking budget (tokens)
    pub fn with_thinking_budget(mut self, budget: u32) -> Self {
        self.thinking_budget = Some(budget);
        self
    }

    /// Override API key for this request (multi-tenant routing)
    pub fn with_api_key(mut self, api_key: impl Into<String>) -> Self {
        self.api_key = Some(api_key.into());
        self
    }

    /// Override base URL for this request (multi-tenant routing)
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = Some(base_url.into());
        self
    }

    /// Add a custom header (overrides default provider header for same key)
    pub fn with_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.extra_headers
            .get_or_insert_with(HashMap::new)
            .insert(key.into(), value.into());
        self
    }

    /// Set custom headers (overrides default provider headers for same keys)
    pub fn with_extra_headers(mut self, headers: HashMap<String, String>) -> Self {
        self.extra_headers = Some(headers);
        self
    }

    pub fn has_non_text_content(&self) -> bool {
        self.messages
            .iter()
            .flat_map(|message| message.content.iter())
            .any(|block| !block.is_text())
    }
}

/// A message in the conversation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    /// Role of the message sender
    pub role: Role,

    /// Content of the message (supports multi-modal content)
    ///
    /// Can contain text, images, and other content blocks
    pub content: Vec<super::message_block::MessageBlock>,

    /// Name of the message sender (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// Tool calls made by the assistant
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,

    /// Tool call ID (for tool responses)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,

    /// Provider-specific reasoning content (GLM style)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,

    /// Provider-specific reasoning (Qwen/DeepSeek/OpenAI o1 common key)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,

    /// Provider-specific thought (OpenAI o1 key)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thought: Option<String>,

    /// Provider-specific thinking (Anthropic key). Compatibility aggregate when
    /// structured [`MessageBlock::Thinking`] entries exist in `content`, those
    /// blocks are authoritative for Anthropic serialization.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking: Option<String>,
}

impl Default for Message {
    fn default() -> Self {
        Self {
            role: Role::User,
            content: Vec::new(),
            name: None,
            tool_calls: None,
            tool_call_id: None,
            reasoning_content: None,
            reasoning: None,
            thought: None,
            thinking: None,
        }
    }
}

impl Message {
    /// Create a new message with text content
    ///
    /// # Example
    ///
    /// ```rust
    /// use llm_connector::types::{Message, Role};
    ///
    /// let message = Message::text(Role::User, "Hello, world!");
    /// ```
    pub fn text(role: Role, text: impl Into<String>) -> Self {
        Self {
            role,
            content: vec![super::message_block::MessageBlock::text(text)],
            name: None,
            tool_calls: None,
            tool_call_id: None,
            reasoning_content: None,
            reasoning: None,
            thought: None,
            thinking: None,
        }
    }

    /// Create a new message with multi-modal content
    ///
    /// # Example
    ///
    /// ```rust
    /// use llm_connector::types::{Message, Role, MessageBlock};
    ///
    /// let message = Message::new(
    ///     Role::User,
    ///     vec![
    ///         MessageBlock::text("What's in this image?"),
    ///         MessageBlock::image_url("https://example.com/image.jpg"),
    ///     ],
    /// );
    /// ```
    pub fn new(role: Role, content: Vec<super::message_block::MessageBlock>) -> Self {
        Self {
            role,
            content,
            name: None,
            tool_calls: None,
            tool_call_id: None,
            reasoning_content: None,
            reasoning: None,
            thought: None,
            thinking: None,
        }
    }

    /// Create a system message
    pub fn system(text: impl Into<String>) -> Self {
        Self::text(Role::System, text)
    }

    /// Create a user message
    pub fn user(text: impl Into<String>) -> Self {
        Self::text(Role::User, text)
    }

    /// Create an assistant message
    pub fn assistant(text: impl Into<String>) -> Self {
        Self::text(Role::Assistant, text)
    }

    /// Convenience: get the first available reasoning-like content
    pub fn reasoning_any(&self) -> Option<&str> {
        self.reasoning_content
            .as_deref()
            .or(self.reasoning.as_deref())
            .or(self.thought.as_deref())
            .or(self.thinking.as_deref())
    }

    /// Extended-thinking bodies from structured [`MessageBlock::Thinking`] blocks in order.
    /// Does not include the [`Message.thinking`] aggregate field.
    pub fn thinking_blocks_concat(&self) -> String {
        self.content
            .iter()
            .filter_map(|block| block.as_thinking_body())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Extract all text content from message blocks
    ///
    /// Joins multiple text blocks with newlines
    pub fn content_as_text(&self) -> String {
        self.content
            .iter()
            .filter_map(|block| block.as_text())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Extract all Base64 encoded images from message content blocks
    ///
    /// Collects Base64 image data from all image blocks in the message.
    ///
    /// # Returns
    ///
    /// A `Vec<String>` containing Base64 encoded image data from all image blocks
    ///
    /// # Example
    ///
    /// ```rust
    /// use llm_connector::types::{Message, Role, MessageBlock};
    ///
    /// let message = Message::new(
    ///     Role::User,
    ///     vec![
    ///         MessageBlock::text("Describe this image"),
    ///         MessageBlock::image_base64("image/png", "base64_data_here"),
    ///     ],
    /// );
    ///
    /// let images = message.content_as_images_base64();
    /// assert_eq!(images.len(), 1);
    /// assert_eq!(images[0], "base64_data_here");
    /// ```
    pub fn content_as_images_base64(&self) -> Vec<String> {
        self.content
            .iter()
            .filter_map(|block| block.as_image_base64())
            .collect()
    }

    /// Check if message contains only text (no images or other media)
    pub fn is_text_only(&self) -> bool {
        self.content.iter().all(|block| block.is_text())
    }

    /// Check if message contains any images
    pub fn has_images(&self) -> bool {
        self.content.iter().any(|block| block.is_image())
    }

    /// Provider-agnostic post-processor: populate reasoning synonyms from raw JSON
    /// Scans nested JSON objects/arrays and fills each synonym field if present.
    pub fn populate_reasoning_from_json(&mut self, raw: &serde_json::Value) {
        fn collect_synonyms(
            val: &serde_json::Value,
            acc: &mut std::collections::HashMap<String, String>,
        ) {
            match val {
                serde_json::Value::Array(arr) => {
                    for v in arr {
                        collect_synonyms(v, acc);
                    }
                }
                serde_json::Value::Object(map) => {
                    for (k, v) in map {
                        let key = k.to_ascii_lowercase();
                        if let serde_json::Value::String(s) = v {
                            match key.as_str() {
                                "reasoning_content" | "reasoning" | "thought" | "thinking" => {
                                    acc.entry(key).or_insert_with(|| s.clone());
                                }
                                _ => {}
                            }
                        }
                        collect_synonyms(v, acc);
                    }
                }
                _ => {}
            }
        }

        let mut found = std::collections::HashMap::<String, String>::new();
        collect_synonyms(raw, &mut found);

        if self.reasoning_content.is_none()
            && let Some(v) = found.get("reasoning_content")
        {
            self.reasoning_content = Some(v.clone());
        }
        if self.reasoning.is_none()
            && let Some(v) = found.get("reasoning")
        {
            self.reasoning = Some(v.clone());
        }
        if self.thought.is_none()
            && let Some(v) = found.get("thought")
        {
            self.thought = Some(v.clone());
        }
        if self.thinking.is_none()
            && let Some(v) = found.get("thinking")
        {
            self.thinking = Some(v.clone());
        }
    }

    /// Create a tool response message
    pub fn tool(content: impl Into<String>, tool_call_id: impl Into<String>) -> Self {
        Self {
            role: Role::Tool,
            content: vec![super::message_block::MessageBlock::text(content)],
            tool_call_id: Some(tool_call_id.into()),
            name: None,
            tool_calls: None,
            ..Default::default()
        }
    }

    /// Set the name of the message sender
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Set tool calls for assistant messages
    pub fn with_tool_calls(mut self, tool_calls: Vec<ToolCall>) -> Self {
        self.tool_calls = Some(tool_calls);
        self
    }

    /// Create an assistant message with tool calls (no text content)
    ///
    /// This is used to reconstruct the assistant's tool-calling message
    /// when building the conversation history for multi-turn tool use.
    pub fn assistant_with_tool_calls(tool_calls: Vec<ToolCall>) -> Self {
        Self {
            role: Role::Assistant,
            content: Vec::new(),
            tool_calls: Some(tool_calls),
            ..Default::default()
        }
    }
}

/// Tool definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tool {
    /// Type of tool (usually "function")
    #[serde(rename = "type")]
    pub tool_type: String,

    /// Function definition
    pub function: Function,
}

impl Tool {
    /// Create a function tool definition
    ///
    /// # Parameters
    /// - `name`: Function name
    /// - `description`: Function description (optional)
    /// - `parameters`: JSON Schema for function parameters
    pub fn function(
        name: impl Into<String>,
        description: Option<String>,
        parameters: serde_json::Value,
    ) -> Self {
        Self {
            tool_type: "function".to_string(),
            function: Function {
                name: name.into(),
                description,
                parameters,
            },
        }
    }
}

/// Function definition for tools
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Function {
    /// Function name
    pub name: String,

    /// Function description
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Function parameters schema
    pub parameters: serde_json::Value,
}

/// Tool choice strategy
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ToolChoice {
    /// String mode: "none", "auto", or "required"
    Mode(String),
    /// Specific function to call
    Function {
        /// Type of tool (always "function")
        #[serde(rename = "type")]
        tool_type: String,
        /// Function to call
        function: FunctionChoice,
    },
}

impl ToolChoice {
    /// No tools should be called
    pub fn none() -> Self {
        Self::Mode("none".to_string())
    }

    /// Let the model decide
    pub fn auto() -> Self {
        Self::Mode("auto".to_string())
    }

    /// Tools must be called
    pub fn required() -> Self {
        Self::Mode("required".to_string())
    }

    /// Call a specific function
    pub fn function(name: impl Into<String>) -> Self {
        Self::Function {
            tool_type: "function".to_string(),
            function: FunctionChoice { name: name.into() },
        }
    }
}

/// Anthropic-native tool definition.
///
/// This preserves Anthropic request payloads without forcing them through the
/// OpenAI function-tool shape. Unknown Anthropic-specific fields can be stored
/// in `extra` and are serialized back unchanged.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct AnthropicToolDefinition {
    /// Anthropic-native tool type, for example `custom`, `server`, or `mcp`.
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub tool_type: Option<String>,

    /// Tool name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// Tool description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Anthropic JSON Schema draft 2020-12 payload.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_schema: Option<serde_json::Value>,

    /// Anthropic-native custom input schema.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_input_schema: Option<serde_json::Value>,

    /// Anthropic-native input examples.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_examples: Option<serde_json::Value>,

    /// Anthropic-native strict flag.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strict: Option<bool>,

    /// Anthropic-native allowed callers configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_callers: Option<serde_json::Value>,

    /// Anthropic-native defer loading flag.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub defer_loading: Option<bool>,

    /// Anthropic-native cache control configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<serde_json::Value>,

    /// Anthropic-native eager input streaming flag.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eager_input_streaming: Option<bool>,

    /// Additional Anthropic-native fields to preserve verbatim.
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

impl AnthropicToolDefinition {
    /// Create an Anthropic-native tool definition with a name.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: Some(name.into()),
            ..Default::default()
        }
    }

    /// Set the Anthropic-native tool type.
    pub fn with_type(mut self, tool_type: impl Into<String>) -> Self {
        self.tool_type = Some(tool_type.into());
        self
    }

    /// Set the Anthropic-native input schema.
    pub fn with_input_schema(mut self, schema: serde_json::Value) -> Self {
        self.input_schema = Some(schema);
        self
    }

    /// Set the Anthropic-native custom input schema.
    pub fn with_custom_input_schema(mut self, schema: serde_json::Value) -> Self {
        self.custom_input_schema = Some(schema);
        self
    }

    /// Preserve an additional Anthropic-native field.
    pub fn with_extra_field(mut self, key: impl Into<String>, value: serde_json::Value) -> Self {
        self.extra.insert(key.into(), value);
        self
    }
}

/// Anthropic-native tool choice.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AnthropicToolChoice {
    #[serde(rename = "type")]
    pub choice_type: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

impl AnthropicToolChoice {
    /// Do not allow tool calls.
    pub fn none() -> Self {
        Self {
            choice_type: "none".to_string(),
            name: None,
            extra: HashMap::new(),
        }
    }

    /// Let the model decide whether to call a tool.
    pub fn auto() -> Self {
        Self {
            choice_type: "auto".to_string(),
            name: None,
            extra: HashMap::new(),
        }
    }

    /// Require the model to call some tool.
    pub fn any() -> Self {
        Self {
            choice_type: "any".to_string(),
            name: None,
            extra: HashMap::new(),
        }
    }

    /// Require the model to call a specific tool.
    pub fn tool(name: impl Into<String>) -> Self {
        Self {
            choice_type: "tool".to_string(),
            name: Some(name.into()),
            extra: HashMap::new(),
        }
    }

    /// Preserve an additional Anthropic-native field.
    pub fn with_extra_field(mut self, key: impl Into<String>, value: serde_json::Value) -> Self {
        self.extra.insert(key.into(), value);
        self
    }
}

/// Specific function choice
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionChoice {
    /// Name of the function to call
    pub name: String,
}

/// Tool call made by the model
///
/// In streaming mode, fields may be incrementally populated across multiple chunks.
/// Use `Option` fields to support partial data in delta chunks.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ToolCall {
    /// Unique identifier for the tool call
    /// Present in the first chunk, may be empty in subsequent delta chunks
    #[serde(
        default,
        deserialize_with = "deserialize_null_as_empty_string",
        skip_serializing_if = "String::is_empty"
    )]
    pub id: String,

    /// Type of tool call (usually "function")
    /// Present in the first chunk, may be empty in subsequent delta chunks
    #[serde(
        rename = "type",
        default,
        deserialize_with = "deserialize_null_as_empty_string",
        skip_serializing_if = "String::is_empty"
    )]
    pub call_type: String,

    /// Function call details
    #[serde(default)]
    pub function: FunctionCall,

    /// Index of this tool call in the array (used in streaming to identify which call to update)
    /// This field is used internally for streaming accumulation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index: Option<usize>,

    /// Provider-specific thought signature (for Gemini 3.0+)
    /// Mandatory to return in subsequent requests to preserve reasoning state
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thought_signature: Option<String>,
}

/// Function call details
///
/// In streaming mode, `name` appears in the first chunk, and `arguments` are
/// accumulated across multiple chunks.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FunctionCall {
    /// Name of the function
    /// Present in the first chunk, may be empty in subsequent delta chunks
    #[serde(
        default,
        deserialize_with = "deserialize_null_as_empty_string",
        skip_serializing_if = "String::is_empty"
    )]
    pub name: String,

    /// Arguments as JSON string
    /// In streaming mode, this is accumulated across multiple chunks
    #[serde(
        default,
        deserialize_with = "deserialize_null_as_empty_string",
        skip_serializing_if = "String::is_empty"
    )]
    pub arguments: String,

    /// Provider-specific thought signature (for Gemini 3.0+)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thought_signature: Option<String>,
}

impl ToolCall {
    /// Merge delta data from another ToolCall into this one
    /// Used for accumulating streaming chunks
    pub fn merge_delta(&mut self, delta: &ToolCall) {
        // Update id if present in delta
        if !delta.id.is_empty() {
            self.id = delta.id.clone();
        }

        // Update type if present in delta
        if !delta.call_type.is_empty() {
            self.call_type = delta.call_type.clone();
        }

        // Merge function data
        if !delta.function.name.is_empty() {
            self.function.name = delta.function.name.clone();
        }

        // Accumulate arguments
        if !delta.function.arguments.is_empty() {
            self.function.arguments.push_str(&delta.function.arguments);
        }

        // Update index if present
        if delta.index.is_some() {
            self.index = delta.index;
        }

        // Update thought_signature if present
        if delta.thought_signature.is_some() {
            self.thought_signature = delta.thought_signature.clone();
        }
        if delta.function.thought_signature.is_some() {
            self.function.thought_signature = delta.function.thought_signature.clone();
        }
    }

    /// Check if this tool call is complete (has all required fields)
    pub fn is_complete(&self) -> bool {
        !self.id.is_empty() && !self.call_type.is_empty() && !self.function.name.is_empty()
        // arguments can be empty for functions with no parameters
    }

    /// Parse the arguments JSON string into a typed value
    ///
    /// # Example
    /// ```rust
    /// use llm_connector::types::ToolCall;
    /// use serde::Deserialize;
    ///
    /// #[derive(Deserialize)]
    /// struct WeatherArgs {
    ///     location: String,
    ///     unit: Option<String>,
    /// }
    ///
    /// let tool_call = ToolCall {
    ///     function: llm_connector::types::FunctionCall {
    ///         name: "get_weather".to_string(),
    ///         arguments: r#"{"location":"Beijing"}"#.to_string(),
    ///         thought_signature: None,
    ///     },
    ///     ..Default::default()
    /// };
    ///
    /// let args: WeatherArgs = tool_call.parse_arguments().unwrap();
    /// assert_eq!(args.location, "Beijing");
    /// ```
    pub fn parse_arguments<T: serde::de::DeserializeOwned>(&self) -> Result<T, serde_json::Error> {
        serde_json::from_str(&self.function.arguments)
    }

    /// Parse the arguments as a generic serde_json::Value
    pub fn arguments_value(&self) -> Result<serde_json::Value, serde_json::Error> {
        if self.function.arguments.is_empty() {
            Ok(serde_json::Value::Object(serde_json::Map::new()))
        } else {
            serde_json::from_str(&self.function.arguments)
        }
    }
}

/// Response format specification
///
/// Supports three modes:
/// - `text`: Default text output
/// - `json_object`: JSON mode (model outputs valid JSON)
/// - `json_schema`: Structured Outputs (model outputs JSON conforming to a schema)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseFormat {
    /// Type of response format ("text", "json_object", or "json_schema")
    #[serde(rename = "type")]
    pub format_type: String,

    /// JSON Schema specification (only used when format_type is "json_schema")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub json_schema: Option<JsonSchemaSpec>,
}

/// JSON Schema specification for Structured Outputs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonSchemaSpec {
    /// Schema name (required by OpenAI)
    pub name: String,

    /// Description of the schema (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// The JSON Schema object
    pub schema: serde_json::Value,

    /// Whether to enable strict schema adherence (default: true)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strict: Option<bool>,
}

impl ResponseFormat {
    /// Create a text response format
    pub fn text() -> Self {
        Self {
            format_type: "text".to_string(),
            json_schema: None,
        }
    }

    /// Create a JSON object response format
    pub fn json_object() -> Self {
        Self {
            format_type: "json_object".to_string(),
            json_schema: None,
        }
    }

    /// Create a JSON Schema response format (Structured Outputs)
    ///
    /// # Parameters
    /// - `name`: Schema name
    /// - `schema`: JSON Schema object
    pub fn json_schema(name: impl Into<String>, schema: serde_json::Value) -> Self {
        Self {
            format_type: "json_schema".to_string(),
            json_schema: Some(JsonSchemaSpec {
                name: name.into(),
                description: None,
                schema,
                strict: Some(true),
            }),
        }
    }

    /// Create a JSON Schema response format with description
    pub fn json_schema_with_desc(
        name: impl Into<String>,
        description: impl Into<String>,
        schema: serde_json::Value,
    ) -> Self {
        Self {
            format_type: "json_schema".to_string(),
            json_schema: Some(JsonSchemaSpec {
                name: name.into(),
                description: Some(description.into()),
                schema,
                strict: Some(true),
            }),
        }
    }
}
