## [1.4.0] - 2026-05-07

### Fixes

- **OpenAI-compatible request conversion**: `openai_message_converter_with_strategy()` now correctly converts `MessageBlock::Image` (Anthropic format) to `{"type":"image_url","image_url":...}` instead of serializing the raw Anthropic shape `{"type":"image","source":...}`. This fixes request failures with strict OpenAI-compatible providers (e.g. xiaomi). Added `block_to_openai_value()` and `blocks_to_openai_content()` helpers in `src/protocols/common/request.rs`.

### Documentation

- Fixed markdown formatting in `docs/ANTHROPIC_THINKING_SIGNATURE.md`.

## [1.3.0] - 2026-05-03

### Breaking

- **`MessageBlock`**: added `Thinking { thinking, signature }` for Anthropic extended-thinking blocks. Downstream exhaustive `match`es on `MessageBlock` must handle the new variant.

### Features

- **Anthropic extended thinking**: `parse_response` now preserves `signature` in structured `MessageBlock::Thinking`; `build_request` serializes those blocks back into Anthropic `content` (compat `Message.thinking` aggregate still filled).
- **OpenAI-compatible requests**: thinking blocks are stripped from serialized `content` and merged into `reasoning_content` so providers do not see unknown `type: "thinking"` parts.
- **Anthropic SSE**: `interpret_anthropic_event` maps `thinking_delta` and `signature_delta` to `Delta.thinking` and `Delta.thinking_signature`.
- **Text-only adapters (Ollama, Tencent)**: prepend `thinking` bodies from structured blocks so they are not dropped from prompts.

### Documentation

- Added `docs/ANTHROPIC_THINKING_SIGNATURE.md` (design and implementation scope).

### Notes

- Assistant `thinking` / `tool_use` interleaving is still limited when `tool_calls` are appended after `content` in Anthropic `build_request`; see the new doc.

## [1.2.1] - 2026-04-29

### Features

- **Anthropic-native tool definition support**:
  - Added public `AnthropicToolDefinition` and `AnthropicToolChoice` request types.
  - Added `ChatRequest::with_anthropic_tools(...)` and `ChatRequest::with_anthropic_tool_choice(...)` for native Anthropic tool payloads.
  - Anthropic requests now preserve native fields such as `custom_input_schema`, `input_examples`, `strict`, `allowed_callers`, `defer_loading`, `cache_control`, and `eager_input_streaming`.
  - Anthropic-native `tool_choice` values now serialize without lossy conversion, including `none`, `auto`, `any`, and named-tool selection.

### Tests

- Added adapter-level regression coverage for Anthropic-native tool serialization.
- Added public request-model regression coverage to verify draft-2020-12 schema payloads and `custom_input_schema` survive protocol mapping.

## [1.2.0] - 2026-04-28

### Fixes

- **OpenAI-compatible assistant tool-call serialization**:
  - Added a capability-driven `empty_assistant_tool_content_strategy` for OpenAI-compatible request mapping.
  - `Message::assistant_with_tool_calls(...)` no longer serializes empty assistant `content` as `[]` by default.
  - Standard OpenAI-compatible providers now use `null`; text-only OpenAI-compatible providers use `""`.
  - Preserved `tool_calls` in request bodies while making empty assistant content safer for OpenAI-compatible gateways.

### Tests

- Added regression coverage for assistant tool-call request serialization:
  - OpenAI-compatible standard path keeps `tool_calls` and serializes empty assistant content as `null`.
  - Text-only OpenAI-compatible path keeps `tool_calls` and serializes empty assistant content as `""`.
- Added capability-default assertions for the new empty assistant tool content strategy.

## [1.1.20] - 2026-04-18

### Fixes

- **OpenAI `tool_choice` capability regression**:
  - Restored `ProviderCapabilities::openai().supports_tool_choice = true` so chat requests with `tools + tool_choice` are no longer rejected before dispatch.
  - Preserved native Anthropic `tool_choice` support introduced in `1.1.19`.

### Tests

- Added capability-precheck regression tests for OpenAI:
  - non-stream `ToolChoice::required()`
  - stream `ToolChoice::function("get_weather")`
- Added OpenAI adapter request-serialization tests for `required` and specific-function `tool_choice`.
- Re-ran Anthropic `tool_choice` precheck regression to ensure no behavior rollback.

## [1.1.19] - 2026-04-18

### Fixes

- **Anthropic native `tool_choice` capability gate**:
  - Enabled `supports_tool_choice` in `ProviderCapabilities::anthropic()`, so native Anthropic requests with `tool_choice` are no longer rejected during precheck.
  - Kept Anthropic adapter mapping behavior (`auto -> auto`, `required -> any`, `function(name) -> tool`).

### Tests

- Added regression coverage to ensure Anthropic capability precheck accepts `tool_choice`.
- Added adapter-level serialization test for Anthropic `ToolChoice::required()` -> `{ "type": "any" }`.

## [1.1.18] - 2026-04-18

### Features

- **Anthropic tools round-trip support (`/v1/messages`)**:
  - Added request mapping for `tools` and `tool_choice` in `AnthropicRequest`.
  - Added assistant `tool_calls -> content[type=tool_use]` serialization.
  - Added tool message `Role::Tool -> content[type=tool_result]` serialization (no text downgrade).
  - Added non-streaming response parsing for `content[type=tool_use] -> message.tool_calls`.
  - Added streaming event parsing for `content_block_start(tool_use)` and `content_block_delta(input_json_delta)` into `delta.tool_calls`.

### Tests

- Added Anthropic regression tests covering single-tool, multi-tool, non-streaming, and streaming tool-call flows.

## [1.1.14] - 2026-03-20

### Fixes

- **Tencent Feature Compilation**:
  - Fixed a missing `capabilities` method implementation in `TencentProvider`.
  - Added missing `ProviderCapabilities` import in `src/providers/mod.rs` to fix compilation when the `tencent` feature is enabled.
- **Tencent Connectivity**:
  - Verified connectivity and basic/reasoning chat functionality for Tencent Hunyuan with real API keys.
  - Corrected `TencentNativeProtocol` endpoint handling and signature construction.

## [1.1.13] - 2026-03-20

## [1.1.11] - 2026-03-20

### Improvements

- **Moonshot real-world verification updates**:
  - Verified `moonshot:cn` (`https://api.moonshot.cn/v1`) and `moonshot:global` (`https://api.moonshot.ai/v1`) separately with region-specific API keys.
  - Confirmed chat, streaming, tool-calling, and thinking flows on both endpoints in no-proxy verification runs.
- **Examples & scripts refresh**:
  - Extended `scripts/run_provider_example_no_proxy.sh` to support `moonshot` examples.
  - Updated Moonshot examples with `REGION` fallback support, no-proxy verification guidance, and notes that cn/global keys may differ.
  - Updated `examples/README.md` with Moonshot environment variables and no-proxy example commands.

## [1.1.10] - 2026-03-20

### Features

- **Protocol stage hooks refinement**:
  - Continued the four-stage protocol refactor across shared protocol entry points and adapters.
  - Extracted clearer Transport/Auth responsibilities via shared auth strategy, header policy, and request metadata policy abstractions.
  - Added reusable common modules for OpenAI-compatible request/stream handling and transport composition.

### Fixes

- **Zhipu streaming reasoning normalization**:
  - Fixed streaming convenience-field backfill so reasoning/thinking deltas are no longer incorrectly emitted as normal `content`.
  - Added regression coverage for reasoning-only streaming chunks.

### Improvements

- **Zhipu real-world verification workflow**:
  - Added a unified no-proxy provider example runner in `scripts/run_provider_example_no_proxy.sh`.
  - Verified both `zhipu:global` (`z.ai`) and `zhipu:cn` (`open.bigmodel.cn`) for chat, streaming, and tool-calling flows without local proxy interference.
- **Examples & docs refresh**:
  - Updated Zhipu examples to reflect `cn/global` endpoint support, `REGION` fallback behavior, and no-proxy verification guidance.
  - Updated examples documentation for the new no-proxy test entry.

## [1.1.9] - 2026-03-18

### Features

- **OpenAI Responses API support**:
  - Added `ResponsesRequest` / `ResponsesResponse` / `ResponsesStreamEvent` types.
  - Added `LlmClient::invoke_responses` and `LlmClient::invoke_responses_stream`.
  - Added protocol/provider support for direct `/responses` requests.
- **Automatic fallback compatibility**:
  - If upstream does not support `/responses` (e.g. `404`), connector falls back to `/chat/completions`.
  - Fallback stream emits compatible minimal event sequence:
    - `response.created`
    - `response.output_text.delta` (repeated)
    - `response.completed`

### Improvements

- **Error diagnostics**: Enriched endpoint errors with provider/endpoint/status/body snippet for easier gateway troubleshooting.
- **Documentation & examples**:
  - Updated `README.md` with Responses API and Responses streaming examples.
  - Updated `www` docs (`index`, `guide/getting-started`, `guide/streaming`, `examples`) with Responses API usage.
  - Updated `examples/openai.rs` to include both non-streaming and streaming Responses examples.

## [1.1.5] - 2026-03-05

### Fixes

- **Header Duplication Fix**: Resolved duplicate authentication headers (`Authorization`, `x-api-key`) when using per-request API key overrides.
- **Selective Auth Injection**: Refactored `build_request_overrides` to inject only protocol-specific auth headers instead of all known types.
- **Header Overwrite Logic**: Updated `HttpClient` to use `HeaderMap` overwrite semantics instead of append semantics for request overrides.
- **Debug Logging**: Added `LLM_DEBUG_OUTBOUND` environment variable support for inspecting outbound request URLs.

## [1.1.4] - 2026-03-05

### Fixes

- **Anthropic Thinking Block**: Fixed `missing field 'text'` error when Anthropic returns pure `thinking` blocks without text content.
- **OpenAI Compatible Downgrade**: Added automatic content format downgrade (array -> string) for providers that do not support complex content blocks (e.g. DeepSeek, Moonshot, MiniMax), fixing `400 Bad Request` errors.
- **DeepSeek/Moonshot/MiniMax Support**: Verified compatibility with these providers using the new downgrade logic.

## [1.1.2] - 2026-03-05

### Fixes

- **Ollama Cloud streaming compatibility**: Added strict Ollama NDJSON chunk fallback parsing for `/api/chat` streaming chunks that do not include OpenAI-style fields like `id`.
- **Protocol-layer stream routing**: Added protocol-aware streaming parse modes so Ollama explicitly uses `OllamaStrict` and OpenAI-compatible adapters use `OpenAIOnly`, reducing cross-protocol misclassification risk.
- **Streaming parser hardening**: Added strict Ollama shape checks and tests for `thinking` chunks and final usage chunks.
- **Ollama example reliability**: Updated `examples/ollama.rs` to auto-select an available model and avoid hardcoded `llama3` failures in cloud-backed local setups.
- **Tencent import path cleanup**: Included provider import path adjustment in `src/providers/mod.rs`.

## [1.1.1] - 2026-03-05

### 🔧 Improvements & Fixes

- **Updated `llm-providers`**: Bumped `llm_providers` dev-dependency to `0.7.4` to natively support `deepseek:global` and `moonshot:global` automatic discovery.
- **Enhanced Integration Testing**: Refactored `real_world_connectivity_test.rs` to comprehensively test `tool_calling` (Moonshot, Anthropic, OpenAI) and advanced `reasoning` block extracting (DeepSeek, MiniMax, Anthropic) across all major providers.
- **Dynamic Region Reliability**: Fixed Moonshot, Zhipu, and MiniMax examples to gracefully fallback from provider-specific `_REGION` vars to generic `REGION` environment variables.

## [1.1.0] - 2026-03-05

### 🚀 V2 Pure Protocol Engine Architecture

- **Anti-Corruption Layer**: Providers are now strictly adapters (`src/protocols/adapters/`).
- **Standardized Chat Formats**: Introduced `src/protocols/formats/chat_completions.rs` to unify parsing logic for all OpenAI-compatible providers, eliminating massive code duplication.
- **Native Reasoning Output Extraction**:
  - Full support for DeepSeek and Gemini reasoning streams.
  - Automatically identifies and extracts `<think>` blocks from models like MiniMax into the unified `reasoning_content` field.
- **Dynamic Discovery Ready**: The core is now a transparent gateway. Hardcoded URLs are eradicated. Integrated examples heavily with `llm-providers` for automatic capability reading.

## [1.0.1] - 2026-03-03

- **Removed Static Endpoints**: Deleted `src/endpoints.rs` and all internal hardcoded API URLs to ensure the library remains a pure, transparent protocol parser.
- **Dynamic URL Configuration**: Updated all provider examples to use `_BASE_URL` environment variables, enabling seamless metadata discovery via `llm-providers`.
- **Improved Provider Robustness**:
  - Fixed Zhipu GLM 404 errors by preventing double-versioning in URL construction.
  - Enhanced Tencent Native endpoint handling to ensure valid protocol schemes.
  - Refined Aliyun DashScope URL logic for flexibility with versioned base URLs.
- **New Examples**: Added dedicated `deepseek.rs` and `minimax.rs` examples for OpenAI-compatible testing.


- **Stage-Based Protocol Decomposition**: Protocol handling is now split into 4 distinct, reusable stages:
  - **Schema Mapping**: Converting unified `ChatRequest` to provider-specific schemas.
  - **Transport & Auth**: Handling HTTP requests, authentication headers, and connectivity.
  - **Stream Interpretation**: Chunked parsing for SSE/NDJSON streams with unified reasoning extraction.
  - **Response Normalization**: Converting raw API responses into a consistent `ChatResponse` format.
- **Improved Maintainability**: Providers now inherit from a clean, modular structure in `src/protocols/common`.
- **Comprehensive Examples**: Consolidated all fragmented examples into high-quality, unified provider examples.
- **Enhanced Test Suite**: Revitalized the entire test directory with modular unit and integration tests.
- **Bug Fixes & Robustness**:
  - Fixed double `/v1` path issues for Anthropic and Aliyun.
  - Resolved compiler warnings across the entire crate.
  - Added new `tool_calling.rs` example for function calling verification.


## [0.9.1] - 2026-03-03

### ⚠️ Breaking Changes - Pure Protocol Engine Architecture

- **Mandatory `base_url`**: All client constructors (`LlmClient::openai`, `LlmClient::anthropic`, `LlmClient::zhipu`, etc.) and provider factory functions now **mandatorily require** a `base_url` parameter. Hardcoded default URLs have been removed from the library core.
- **Provider Consolidation**: Redundant provider modules (`deepseek.rs`, `moonshot.rs`, `volcengine.rs`, `xiaomi.rs`, `longcat.rs`) have been removed and consolidated into generic protocols (`OpenAIProtocol`, `AnthropicProtocol`).
- **Removed Methods**: `openai_with_base_url()`, `ollama_with_base_url()`, and various `_with_timeout()` factory methods have been removed as their functionality is now covered by the mandatory `base_url` and consolidated `_with_config` methods.

### 🚀 New Features

- **Protocol-First Design**: Positioned as a standalone, "URL-Blind" protocol engine that focuses exclusively on protocol adaptation, streaming, and token counting.
- **Reference Endpoints**: Added `src/endpoints.rs` featuring `pub const` official API endpoints for developer convenience.
- **Standalone Library**: Explicitly decoupled core logic from endpoint management, ensuring zero dependencies on external URL management projects for core functionality.

### 📝 Documentation

- Updated `README.md` to reflect the new "Pure Gateway" positioning.
- Updated all examples and tests to use explicit `base_url` parameters.

## [0.8.1] - 2026-02-27

### 🔧 Fixed

- **Code Quality**: Resolved all Clippy warnings (unused imports, collapsible matches/ifs).
- **Format**: Applied `cargo fmt` to the entire codebase to comply with CI requirements.

## [0.8.0] - 2026-02-27

### 🚀 Major Features

- **Full OpenAI Protocol Compatibility**:
  - Validated support for **OpenAI o3-mini** with `ReasoningEffort`.
  - Validated **Claude 3.7 Thinking** models via OpenAI-compatible gateways.
  - Verified **Function Calling** and **JSON Mode** support.
  - Added comprehensive test suite `test_gateway_proxy` covering standard chat, reasoning, streaming, tools, and structured output.

- **Dynamic Service Resolution (Gateway Mode)**
  - Introduced `ServiceResolver` trait for dynamic endpoint/key resolution.
  - Request-level overrides via `ChatRequest::with_api_key()` and `with_base_url()`.
  - Enables high-performance gateway scenarios with connection pooling.

- **Reasoning & Thinking Support**
  - Unified support for reasoning models (Claude 3.7, OpenAI o1/o3, Gemini 2.0).
  - Added `reasoning_effort` (Low/Medium/High) and `thinking_budget` parameters.
  - Automatic handling of provider-specific parameters (e.g., `thinking` for Anthropic).

- **Robust Streaming Engine**
  - Completely rewritten SSE parser in `src/sse.rs`.
  - Auto-detection of stream formats: Standard SSE, NDJSON (Ollama), and Hybrid SSE (Zhipu).
  - Improved resilience against network jitter and packet fragmentation.

- **Developer Experience (DX)**
  - `MessageBlock::from_file_path()` for one-line file/image uploads.
  - Automatic MIME type detection and Base64 encoding.

### 🔧 Improvements

- **Refactored Architecture**: Clear separation of "Driver Layer" (llm-connector) and "Gateway Layer".
- **Documentation**: Updated README with new features and architectural diagrams.

## [0.7.2] - 2026-02-24

### 📝 Documentation

- Added comprehensive changelog and development status to website
- Updated website configuration with new development section

## [0.7.1] - 2026-03-02

### Added

- **Embedding API Support**
  - Unified `EmbedRequest` and `EmbedResponse` types
  - Added `.embed()` method to `LlmClient`
  - Support for 7+ providers: OpenAI, Anthropic, Google Gemini (`batchEmbedContents`), Ollama, Aliyun DashScope, Zhipu GLM, and MockProvider
- **Multimodal Document Support**
  - Added `MessageBlock::Document` and `MessageBlock::DocumentUrl` (e.g. for PDF support)
  - Updated Zhipu and other providers to handle document blocks
- **Enhanced Usage Tracking**
  - Added `prompt_cache_hit_tokens` and `prompt_cache_miss_tokens` to `Usage` struct
  - Robust mapping for Anthropic (`cache_read_input_tokens`, `cache_creation_input_tokens`) and OpenAI caching diagnostics

### Fixed

- **Zhipu Protocol**: Resolved non-exhaustive pattern matching for `MessageBlock`
- **Documentation**: Added [`docs/PARAMETER_MAPPING.md`](file:///Users/mac-m4/github/llm-connector/docs/PARAMETER_MAPPING.md) for cross-provider parameter reference

## [0.7.0] - 2026-02-23

### Added

- **Per-Request Overrides (Multi-Tenant / Gateway)**
  - `ChatRequest` now supports `api_key`, `base_url`, and `extra_headers` for per-request overrides
  - `with_api_key()`, `with_base_url()`, `with_header()`, `with_extra_headers()` builder methods
  - Supports multi-tenant routing without creating a new client per tenant
  - Custom headers (e.g. `X-Trace-Id`, `anthropic-version`) override default provider headers
  - Works with all `GenericProvider`-based providers (OpenAI, Anthropic, DeepSeek, Moonshot, Volcengine, etc.)

## [0.6.1] - 2026-02-20

### 🔧 Build / Compatibility

- Rust 2024 edition (MSRV: Rust 1.85+)
- Reqwest uses `rustls-tls` by default (better Android cross-compilation compatibility)

## [0.6.0] - 2026-02-15

### 🚀 New Features

- **Rust 2024 edition** — MSRV is now Rust 1.85+

- **Builder Pattern for LlmClient** — `LlmClient::builder()` provides a fluent API for client construction with optional `base_url()`, `timeout()`, `proxy()` configuration. Supports all 12+ providers.
  ```rust
  let client = LlmClient::builder()
      .deepseek("sk-...")
      .timeout(60)
      .build()?;
  ```

- **Zhipu Multimodal Support** — Zhipu protocol now supports image URLs and base64 images via `MessageBlock::image_url()` and `MessageBlock::image_base64()`. Works with `glm-4v-flash` and other vision models.

### ⚡ Breaking Changes (minor)

- **Streaming now enabled by default** — `streaming` feature is included in `default` features, so `chat_stream()`, `ChatStream`, `StreamingResponse` etc. are available without extra configuration. Downstream libraries no longer need `features = ["streaming"]`.

## [0.5.17] - 2026-02-14

### 🚀 New Features

- **Mock Client for Testing**
  - New `MockProvider` for unit testing without real API calls
  - `MockProviderBuilder` with fluent API for fine-grained control
  - `LlmClient::mock("content")` one-liner for simple cases
  - Sequential response mode for multi-turn test scenarios
  - Error simulation support for testing error handling paths
  - Request tracking via `as_mock().request_count()` / `get_requests()`
  - Tool call simulation via `MockProviderBuilder::with_tool_calls()`

## [0.5.16] - 2026-02-14

### 🚀 New Features

- **Enhanced Tool Calling (P0)**
  - `ChatResponse`: Added `tool_calls()`, `is_tool_call()`, `finish_reason()` convenience methods
  - `ToolCall`: Added `parse_arguments<T>()` for typed deserialization, `arguments_value()` for generic JSON
  - `Message`: Added `assistant_with_tool_calls()` constructor for multi-turn tool use
  - `Tool`: Added `Tool::function()` convenience constructor

- **Structured Outputs (P1)**
  - `ResponseFormat`: Extended to support `json_schema` mode (OpenAI Structured Outputs)
  - New `JsonSchemaSpec` type with `name`, `description`, `schema`, `strict` fields
  - Convenience constructors: `ResponseFormat::text()`, `json_object()`, `json_schema()`, `json_schema_with_desc()`
  - OpenAI protocol now correctly passes `response_format` to API requests

- **Error Type Refinement (P2)**
  - Added `ContextLengthExceeded` error variant
  - `is_retryable()` now includes `ServerError`, `TimeoutError`, `ConnectionError`
  - New helper methods: `should_reduce_context()`, `is_auth_error()`, `is_rate_limited()`
  - Context length detection in OpenAI, Anthropic, Aliyun, Zhipu `map_error` implementations

- **Token Usage (P2)**
  - `Usage` now derives `Default` for easier construction

### 📝 Documentation

- Updated README with new Tool Calling and Structured Output examples
- Updated docs/TOOLS.md with convenience API usage
- Added integration test example: `examples/test_wishlist.rs`

## [0.5.15] - 2026-02-14

### 📝 Documentation

- **README Refactoring** - Simplified from 1316 lines to ~220 lines
  - Moved detailed content to dedicated docs/ files with links
  - Cleaner structure with quick reference table
- **New Documentation Files**
  - `docs/STREAMING.md` - Comprehensive streaming guide
  - `docs/TOOLS.md` - Function calling / tools guide
  - `docs/MULTIMODAL.md` - Multi-modal content guide
- **Updated docs/PROVIDERS.md**
  - Added Xiaomi MiMo provider section
  - Added Google Gemini detailed configuration
  - Added Ollama Model Management section
  - Added environment variables reference table

## [0.5.14] - 2026-02-14

### 🚀 New Features

- **Xiaomi MiMo Provider**
  - Added support for Xiaomi MiMo API (OpenAI-compatible)
  - New provider functions: `xiaomi()`, `xiaomi_with_config()`
  - New client methods: `LlmClient::xiaomi()`, `LlmClient::xiaomi_with_config()`
  - Base URL: `https://api.xiaomimimo.com/v1`
  - Supported model: `mimo-v2-flash`
  - Added example: `examples/xiaomi_basic.rs`

## [0.5.13] - 2026-01-03

### 🔧 Breaking Changes

- **OpenAI-compatible providers**: Removed automatic `/v1` path appending
  - `OpenAIProtocol` endpoints now only append `/chat/completions` and `/models`
  - `ConfigurableProtocol::openai_compatible` templates no longer include `/v1`
  - `openai_with_config` defaults to `https://api.openai.com/v1` only when no base_url provided
  - Base URLs are now respected verbatim for all OpenAI-compatible providers

### 📝 Documentation

- Updated provider tests to reflect new endpoint URL behavior

## [0.5.11] - 2026-01-02

### 🚀 New Features

- **Tencent Hunyuan Native API v3 Streaming**
  - Implemented `chat_stream` for Tencent Native API v3.
  - Added strict streaming response parsing for Hunyuan's PascalCase SSE format.
  - Added example: `examples/tencent_native_streaming.rs`

### 📝 Documentation

- Updated README with Tencent Native streaming usage.
- Updated provider docs to mention Tencent streaming support.

## [0.5.12] - 2026-01-03

### 🚀 New Features

- **Google Gemini Streaming**
  - Implemented `chat_stream` for Google Gemini via SSE (`streamGenerateContent`).
  - Added example: `examples/google_streaming.rs`

### 🔧 Improvements

- Updated Google authentication to use `x-goog-api-key` header (per official docs).

### 📝 Documentation

- Updated README with Google Gemini streaming usage and runnable example.

## [0.5.10] - 2026-01-02

### 🚀 New Features

- **Google Gemini provider**
  - `LlmClient::google(api_key)`
  - `LlmClient::google_with_config(api_key, base_url, timeout_secs, proxy)`
  - Example: `examples/google_basic.rs`

### 📝 Documentation

- Updated README with Google Gemini provider usage and notes.

## [0.5.9] - 2026-01-02

### 🔧 Maintenance

- Version bump and release preparation.

## [0.5.8] - 2026-01-02

### ⚠️ Breaking Changes

#### Tencent Hunyuan Native API v3
- **BREAKING**: Replaced OpenAI-compatible wrapper with native Tencent Cloud API v3 using `TC3-HMAC-SHA256` signature.
- **Affected**: `LlmClient::tencent()` and `tencent()` provider functions.
- **New Signature**: `tencent(secret_id, secret_key)` (previously `tencent(api_key)`).
- **Rationale**: Support native signature verification for better security and stability.

### ✨ Improvements

- **Security**: Hardcoded API keys removed from documentation and code.
- **Documentation**: Updated Tencent guide with native API usage.


## [0.5.7] - 2025-11-23

### 🚀 New Features

#### Aliyun DashScope Tools Support
- **Added**: Full tool calling support for Aliyun DashScope API
  - Non-streaming tool calls
  - Streaming tool calls
  - Compatible with OpenAI tool format (no conversion needed)
- **Updated**: `AliyunParameters` - Added `tools` and `tool_choice` fields
- **Updated**: `AliyunMessage` - Added `tool_calls` field
- **Updated**: Request/response conversion to handle tools
- **Updated**: Streaming response to handle tool calls

### 🔧 Improvements

#### Repository Cleanup
- **Removed**: Personal tool configurations from git tracking
  - `.augment/rules/rust.md` - Augment AI configuration
  - `.zed/settings.json` - Zed editor configuration
  - Files removed from git but preserved locally
  - Already in `.gitignore` but were tracked before

### 🧪 Testing

#### New Test Examples
- **Added**: `examples/test_aliyun_tools.rs`
  - Demonstrates non-streaming tool calls
  - Demonstrates streaming tool calls
  - Weather tool example
  - Calculator tool example

### 📚 Documentation

#### New Documentation
- **Added**: `docs/ALIYUN_TOOLS_FIX_ANALYSIS.md`
  - Detailed problem analysis
  - Solution design
  - Code change examples
- **Added**: `docs/ALIYUN_TOOLS_IMPLEMENTATION_SUMMARY.md`
  - Implementation summary
  - Testing results
  - Usage examples

### ✅ Verification

- All 82 tests passing
- Build successful
- Fully backward compatible
- No breaking changes

### 🔄 Migration

No migration needed. All changes are backward compatible.

#### Using Aliyun Tools (New Feature)

```rust
use llm_connector::{LlmClient, types::*};

let client = LlmClient::aliyun("your-api-key")?;

let tools = vec![
    Tool {
        tool_type: "function".to_string(),
        function: Function {
            name: "get_weather".to_string(),
            description: Some("Get weather information".to_string()),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "location": {"type": "string"}
                },
                "required": ["location"]
            }),
        },
    },
];

let request = ChatRequest {
    model: "qwen-plus".to_string(),
    messages: vec![Message::text(Role::User, "What's the weather in Beijing?")],
    tools: Some(tools),
    tool_choice: Some(ToolChoice::Auto),
    ..Default::default()
};

let response = client.chat(&request).await?;

if let Some(tool_calls) = &response.choices[0].message.tool_calls {
    for tool_call in tool_calls {
        println!("Tool: {}", tool_call.function.name);
        println!("Arguments: {}", tool_call.function.arguments);
    }
}
```

---

## [0.5.6] - 2025-11-23

### 🔥 Critical Fix

#### Proxy Timeout Issue
- **Fixed**: Disabled system proxy by default in HTTP client
  - **Root cause**: reqwest automatically uses system proxy settings
  - **Problem**: System proxy can be slow, unreachable, or misconfigured
  - **Impact**: Caused timeout errors even when direct connection works
  - **Solution**: Explicitly disable system proxy, require explicit configuration

#### Changes
- **HttpClient::new()**: Added `.no_proxy()` to disable system proxy
- **HttpClient::with_config()**: Added `.no_proxy()` when proxy parameter is None
- **Explicit proxy**: Only use proxy when explicitly configured via parameters

### 🎯 Benefits

1. **No unexpected timeouts** - System proxy no longer interferes
2. **Better performance** - Direct connections are faster
3. **Predictable behavior** - Same behavior across all environments
4. **Explicit control** - Users must explicitly set proxy if needed

### 🧪 Testing

#### New Test Examples
- **Added**: `examples/test_proxy_issue.rs`
  - Tests default behavior (no proxy)
  - Tests invalid proxy handling
  - Checks system proxy environment variables

#### Test Results
- ✅ All 82 tests passing
- ✅ Zhipu streaming: 2.05s, 52 chunks (no timeout)
- ✅ Invalid proxy fails fast: 41ms (no long timeout)
- ✅ Direct connection works reliably

### 📚 Documentation

#### New Documentation
- **Added**: `docs/PROXY_TIMEOUT_FIX.md`
  - Detailed technical explanation
  - Root cause analysis
  - Usage examples and migration guide
- **Added**: `docs/PROXY_FIX_SUMMARY.md`
  - Executive summary
  - Impact analysis
  - Migration guide for different scenarios

### 🔄 Migration

#### Most Users (No Change Needed)
```rust
// Works better now (no timeout issues)
let client = LlmClient::zhipu_openai_compatible(api_key)?;
```

#### Users Who Need Proxy
```rust
// Explicitly set proxy
let client = LlmClient::zhipu_with_config(
    api_key,
    true,                           // OpenAI compatible
    None,                           // Default base URL
    None,                           // Default timeout
    Some("http://proxy:8080"),      // Explicit proxy
)?;
```

#### Users Relying on System Proxy (Rare)
```rust
// Read system proxy and set explicitly
let proxy = std::env::var("HTTPS_PROXY").ok();
let client = LlmClient::zhipu_with_config(
    api_key,
    true,
    None,
    None,
    proxy.as_deref(),
)?;
```

### ⚠️ Breaking Change (Rare)

**Only affects users relying on system proxy settings**:
- Before: System proxy was used automatically
- After: System proxy is ignored, must be set explicitly

**Workaround**: See migration guide above

### ✅ Verification

- All tests passing (82/82)
- Streaming verified with Zhipu GLM API
- Performance improved (no proxy overhead)
- Fully backward compatible for most users

---

## [0.5.5] - 2025-11-23

### 🚀 Improvements

#### Streaming Timeout Configuration
- **Increased**: Default HTTP timeout from 30s to 60s
  - Better support for long streaming responses
  - More reasonable default for LLM APIs
  - Reduces risk of premature timeouts
- **Added**: Streaming-specific HTTP headers
  - `Accept: text/event-stream` - Standard SSE header
  - `Cache-Control: no-cache` - Prevents caching issues
  - `Connection: keep-alive` - Maintains connection
- **Improved**: Error messages for timeout errors
  - Now suggests increasing timeout for long-running streams
  - More actionable guidance for troubleshooting

#### Code Quality
- **Completed**: Full Chinese to English translation
  - All source code comments now in English
  - 100% internationalization complete
  - Zero Chinese characters remaining in codebase
  - 18 source files completely translated
  - 4 cleanup scripts created for future reference

### 🧪 Testing

#### New Test Examples
- **Added**: `examples/test_zhipu_streaming_timeout.rs`
  - Tests basic streaming functionality
  - Verifies timeout behavior
  - Measures chunk count and timing
- **Added**: `examples/test_zhipu_long_streaming.rs`
  - Tests long streaming responses (17+ seconds)
  - Verifies no timeout issues with extended streams
  - Demonstrates 600+ chunk handling

### 📚 Documentation

#### New Documentation
- **Added**: `docs/STREAMING_TIMEOUT_FIX.md`
  - Detailed explanation of timeout improvements
  - Usage recommendations and guidelines
  - Timeout configuration examples
- **Added**: `docs/STREAMING_INVESTIGATION_REPORT.md`
  - Investigation results and findings
  - Test evidence and verification
  - Recommendations for integration
- **Added**: `docs/COMPLETE_CHINESE_CLEANUP_FINAL.md`
  - Complete translation summary
  - Statistics and verification
  - Quality assurance report
- **Added**: `docs/CHINESE_CLEANUP_STATUS.md`
  - Translation progress tracking
  - Remaining work documentation

### ✅ Verification

- All 82 tests passing
- Streaming verified with Zhipu GLM API
- Short responses: 2.27s, 52 chunks ✅
- Long responses: 17.4s, 633 chunks ✅
- No timeout issues observed
- Fully backward compatible

### 🔄 Migration

No breaking changes. All existing code continues to work.

Optional: Use custom timeout for very long responses:
```rust
// 120 seconds timeout for long responses
let client = LlmClient::zhipu_with_timeout(api_key, 120)?;
```

---

## [0.5.4] - 2025-01-22

### 🔧 Bug Fixes

#### Streaming Tool Calls Fix
- **Fixed**: Incremental accumulation and deduplication logic for streaming tool_calls
  - Modified `ToolCall` and `FunctionCall` data structures to support incremental parsing
  - Implemented `merge_delta()` method for merging incremental data
  - Added accumulation state management in `sse_to_streaming_response`
  - Only send complete tool_calls to prevent duplicate execution
- **Improved**: Support for OpenAI streaming API's incremental tool_calls format
  - Added `index` field to identify tool_calls
  - Implemented `is_complete()` method to check completeness
  - Accumulate each tool_call's incremental data by `index`
- **Guaranteed**: Each tool_call is sent only once, preventing duplicate execution
- **Compatible**: Fully backward compatible, no impact on existing code
- **Tests**: Added comprehensive test suite to verify accumulation logic
  - `test_streaming_tool_calls_accumulation` - Verify accumulation logic
  - `test_streaming_tool_calls_parsing` - Verify incremental parsing
- **Documentation**:
  - Added `docs/STREAMING_TOOL_CALLS.md` - Technical documentation
  - Added `docs/STREAMING_TOOL_CALLS_FIX.md` - Fix summary
  - Updated `README.md` with Function Calling section

### 📚 Documentation

#### Documentation Cleanup
- **Cleanup**: Moved outdated temporary documents to archive
  - `DOCS_CLEANUP_SUMMARY.md` → `docs/archive/reports/`
  - `RELEASE_v0.5.3_SUMMARY.md` → `docs/archive/releases/`
  - `SENSITIVE_INFO_OBFUSCATION.md` → `docs/archive/reports/`
- **Updated**: `docs/README.md` with new documentation index
- **Simplified**: README.md changelog section, pointing to CHANGELOG.md
- **Removed**: Outdated references to non-existent examples (`test_keys_yaml`, `debug_deepseek`, `fetch_models_simple`)
- **Updated**: Examples section to reflect actual available examples
- **Improved**: Troubleshooting section with practical solutions
- **Reorganized**: README.md structure for better user experience
  - Moved "Supported Providers" overview right after Quick Start
  - Moved "Function Calling / Tools" and "Streaming" sections earlier
  - Moved architecture details ("Unified Output Format") to later sections
  - Removed duplicate sections
  - New flow: Introduction → Features → Quick Start → Providers → Key Features → Architecture → Advanced Topics

#### Language Standardization
- **Converted**: All Chinese text to English for international accessibility
  - README.md Function Calling section
  - CHANGELOG.md v0.5.4 section
  - Test files and example files
  - All comments and user-facing messages
  - Removed Chinese characters from provider names (Tencent Hunyuan, Volcengine, Moonshot)
  - `docs/README.md` - Converted entire documentation index to English
  - `docs/REASONING_MODELS_SUPPORT.md` - Completely rewritten in English
- **Simplified**: Removed excessive emoji usage from README.md
  - Kept minimal, professional formatting
  - Replaced emoji bullets with standard markdown bullets
  - Removed decorative emojis from section headers
- **Archived**: Moved original Chinese version of REASONING_MODELS_SUPPORT.md to `docs/archive/reports/`
- **Documentation**:
  - Added `docs/CHINESE_TO_ENGLISH_CONVERSION.md` for conversion summary
  - Added `docs/EMOJI_AND_CHINESE_CLEANUP.md` for cleanup summary

## [0.5.3] - 2025-01-15

### 🎉 New Features

#### Universal Reasoning Models Support 🧠
- **Added**: Universal support for reasoning models across all providers
- **Supported Models**:
  - ✅ Volcengine Doubao-Seed-Code (`reasoning_content`)
  - ✅ DeepSeek R1 (`reasoning_content` / `reasoning`)
  - ✅ OpenAI o1 series (`thought` / `reasoning_content`)
  - ✅ Qwen reasoning models (`reasoning`)
  - ✅ Anthropic Claude extended thinking (`thinking`)
- **Key Benefits**:
  - Zero configuration - automatic field detection
  - Unified interface - same code for all reasoning models
  - Backward compatible - standard models work as before
  - Priority-based extraction - standard `content` field takes precedence
- **Documentation**: Added `docs/REASONING_MODELS_SUPPORT.md`

### 📚 Documentation

#### Documentation Structure Cleanup
- **Reorganized**: Cleaned up docs directory from 52 to 30 files (-42%)
- **New Structure**:
  - Core documents (6): Architecture, migration guides, reasoning models support
  - Provider guides (7): Dedicated guide for each provider in `docs/guides/`
  - Archive (17): Historical releases and reports in `docs/archive/`
- **New Provider Guides**:
  - `docs/guides/ALIYUN_GUIDE.md` - Aliyun DashScope usage guide
  - `docs/guides/ANTHROPIC_GUIDE.md` - Anthropic Claude usage guide
  - `docs/guides/ZHIPU_GUIDE.md` - Zhipu GLM usage guide
  - Updated existing guides for DeepSeek, Moonshot, Tencent, Volcengine
- **Improvements**:
  - Clear documentation index in `docs/README.md`
  - Removed duplicate and outdated content
  - Better organization and discoverability

#### Security
- **Obfuscated**: All sensitive information in documentation and examples
  - API keys replaced with placeholders
  - Endpoint IDs replaced with example values
  - Created `keys.yaml.example` for configuration reference
- **Documentation**: Added `docs/SENSITIVE_INFO_OBFUSCATION.md`

### 🐛 Bug Fixes

#### Volcengine Streaming Support
- **Fixed**: Volcengine streaming now correctly extracts content from reasoning models (Doubao-Seed-Code)
- **Issue**: `StreamingResponse.get_content()` returned `None` for Doubao-Seed-Code model responses
- **Root Cause**: Reasoning models output content in `delta.reasoning_content` field instead of `delta.content`
- **Solution**: Enhanced SSE parser to check multiple content fields in priority order:
  1. `delta.content` (standard OpenAI format, non-empty)
  2. `delta.reasoning_content` (Volcengine Doubao-Seed-Code, DeepSeek R1)
  3. `delta.reasoning` (Qwen, DeepSeek)
  4. `delta.thought` (OpenAI o1)
  5. `delta.thinking` (Anthropic)
- **Impact**: Benefits all reasoning models across different providers
- **Files Changed**: `src/sse.rs`
- **Tests Added**:
  - Unit test: `test_streaming_response_content_population`
  - Integration test: `examples/volcengine_streaming.rs`
  - Automation script: `scripts/test_volcengine_streaming.sh`
- **Documentation**:
  - `docs/VOLCENGINE_STREAMING_FIX.md` - Detailed fix documentation
  - `docs/VOLCENGINE_STREAMING_SUMMARY.md` - Fix summary
  - `docs/VOLCENGINE_STREAMING_FINAL_REPORT.md` - Final report
  - `docs/REASONING_MODELS_SUPPORT.md` - Universal reasoning models guide
- **Test Results**:
  - ✅ 221 tests passed
  - ✅ Volcengine streaming: 101 chunks, 477 chars extracted
  - ✅ All existing functionality preserved

## [0.5.1] - 2025-01-21

### 🔧 Improvements

#### Code Quality
- Fixed all compilation errors and warnings discovered by rust-analyzer
- Fixed unused variable warnings by using underscore prefix
- Cleaned up 69% of example files (39 → 12)
- Cleaned up 56% of test files (18 → 8)
- Removed 36 duplicate, debug, and outdated files

#### Documentation
- Added `docs/RUST_CODING_RULES.md` - Rust coding standards
- Added `docs/MIGRATION_GUIDE_v0.5.0.md` - Complete migration guide
- Added `docs/RELEASE_v0.5.0.md` - Release notes
- Updated `examples/README.md` with cleaner structure
- Updated all examples to use new API

#### Examples Cleanup
- Removed duplicate examples (test_aliyun_basic.rs, test_deepseek.rs, etc.)
- Removed debug files (debug_aliyun_response.rs, debug_longcat_stream.rs, etc.)
- Removed verification files (verify_aliyun_choices.rs, verify_reasoning_content.rs, etc.)
- Removed shell test scripts (9 files)
- Renamed test_aliyun_enable_thinking.rs → aliyun_thinking.rs

#### Bug Fixes
- Fixed Message construction in all examples
- Fixed content access using content_as_text()
- Fixed streaming examples with proper feature gates
- Fixed tencent_basic.rs API usage
- Fixed all integration tests

### 📊 Statistics

- **Tests**: 221 passed; 0 failed (100% pass rate)
- **Compilation**: 0 errors, 0 warnings
- **Code reduction**: 74% fewer lines in examples/tests

## [0.5.0] - 2025-01-21

### 🎉 Major Features - Multi-modal Content Support

**⚠️ BREAKING CHANGE**: `Message.content` changed from `String` to `Vec<MessageBlock>`

This is a major architectural improvement that enables native multi-modal content support (text + images).

#### New Types

- **`MessageBlock`** - Enum for different content types
  - `Text { text: String }` - Text content
  - `Image { source: ImageSource }` - Image (Anthropic format)
  - `ImageUrl { image_url: ImageUrl }` - Image URL (OpenAI format)
- **`ImageSource`** - Image source (Base64 or URL)
- **`ImageUrl`** - Image URL with optional detail level

#### Migration Guide

```rust
// Old (0.4.x)
let message = Message {
    role: Role::User,
    content: "Hello".to_string(),
    ..Default::default()
};

// New (0.5.0) - Option 1: Use text() constructor (recommended)
let message = Message::text(Role::User, "Hello");

// New (0.5.0) - Option 2: Use new() with MessageBlock
let message = Message::new(
    Role::User,
    vec![MessageBlock::text("Hello")],
);

// New (0.5.0) - Multi-modal example
let message = Message::new(
    Role::User,
    vec![
        MessageBlock::text("What's in this image?"),
        MessageBlock::image_url("https://example.com/image.jpg"),
    ],
);
```

#### New Methods

**Message**:
- `Message::text(role, text)` - Create text-only message
- `Message::new(role, blocks)` - Create multi-modal message
- `Message::content_as_text()` - Extract all text content
- `Message::is_text_only()` - Check if message contains only text
- `Message::has_images()` - Check if message contains images

**MessageBlock**:
- `MessageBlock::text(text)` - Create text block
- `MessageBlock::image_base64(media_type, data)` - Create Base64 image
- `MessageBlock::image_url(url)` - Create image URL block
- `MessageBlock::image_url_with_detail(url, detail)` - Create image URL with detail
- `MessageBlock::as_text()` - Get text content if it's a text block
- `MessageBlock::is_text()` - Check if it's a text block
- `MessageBlock::is_image()` - Check if it's an image block

#### Updated Protocols

- ✅ **OpenAI** - Supports both string and array formats
- ✅ **Anthropic** - Always uses array format
- ✅ **Aliyun** - Converts to text format
- ✅ **Zhipu** - Converts to text format
- ✅ **Ollama** - Converts to text format

#### Examples

- `examples/multimodal_basic.rs` - Comprehensive multi-modal examples

#### Tests

- Added 8 new unit tests for `MessageBlock`
- All 64 tests passing

#### Documentation

- `docs/MULTIMODAL_CONTENT_DESIGN.md` - Design comparison
- `docs/MULTIMODAL_NATIVE_DESIGN.md` - Native design approach
- `docs/MULTIMODAL_MIGRATION_PLAN.md` - Migration plan

---

## [Unreleased]

### Added
- **Moonshot (Moonshot AI) Provider**
  - OpenAI-compatible API
  - `LlmClient::moonshot(api_key)`
  - Models: moonshot-v1-8k, moonshot-v1-32k, moonshot-v1-128k
  - Long context support (up to 128k tokens)
  - Full streaming support

- **DeepSeek Provider**
  - OpenAI-compatible API
  - `LlmClient::deepseek(api_key)`
  - Models: deepseek-chat, deepseek-reasoner
  - Reasoning model support with thinking process
  - Automatic extraction of reasoning content
  - Full streaming support for both chat and reasoning models

## [0.4.20] - 2025-10-21

### 🎯 Major Update: Unified Output Format & Configuration-Driven Architecture

#### ✨ Unified Output Format

**All providers now output the same unified `StreamingResponse` format**, regardless of their native API format.

```
Different Input Formats → Protocol Conversion → Unified StreamingResponse
```

**Benefits**:
- ✅ Consistent API across all providers
- ✅ Easy provider switching without changing business logic
- ✅ Type-safe compile-time guarantees
- ✅ Lower learning curve - learn once, use everywhere

**Example**:
```rust
// Same code works with ANY provider
let mut stream = client.chat_stream(&request).await?;
while let Some(chunk) = stream.next().await {
    let chunk = chunk?;  // Always StreamingResponse
    if let Some(content) = chunk.get_content() {
        print!("{}", content);
    }
}
```

#### 🏗️ Configuration-Driven Architecture

**New Core Modules**:

1. **ProviderBuilder** (`src/core/builder.rs`)
   - Unified builder pattern for all providers
   - Chain-able API: `.timeout()` / `.proxy()` / `.header()`
   - Eliminates repetitive `xxx_with_config` boilerplate
   - Reduces code by ~50%

2. **ConfigurableProtocol** (`src/core/configurable.rs`)
   - Configuration-driven protocol adapter
   - `ProtocolConfig` - Static configuration (name, endpoints, auth)
   - `EndpointConfig` - Endpoint templates with `{base_url}` variable
   - `AuthConfig` - Flexible authentication (Bearer/ApiKeyHeader/None/Custom)
   - New providers only need configuration, not code

**Code Reduction**:
- Tencent: 169 lines → 122 lines (-28%)
- Volcengine: 169 lines → 145 lines (-14%)
- LongCat: 169 lines → 145 lines (-14%)
- **Average: -19% code reduction**

#### 🆕 New Providers

1. **Tencent Hunyuan (Hunyuan) **
   - OpenAI-compatible API
   - `LlmClient::tencent(api_key)`
   - Models: hunyuan-lite, hunyuan-standard, hunyuan-pro, hunyuan-turbo

2. **LongCat API**
   - Dual format support
   - `LlmClient::longcat_openai(api_key)` - OpenAI format
   - `LlmClient::longcat_anthropic(api_key)` - Anthropic format with Bearer auth

#### 🔧 Anthropic Streaming Fix

**Problem**: LongCat Anthropic streaming failed with "missing field `id`" error

**Solution**: Implemented custom `parse_stream_response` for Anthropic protocol
- Correctly handles Anthropic's multi-event streaming format:
  - `message_start` - Extract message ID
  - `content_block_delta` - Extract text increments
  - `message_delta` - Extract usage and stop_reason
- Converts to unified `StreamingResponse` format
- **Now works perfectly with LongCat Anthropic!**

**Test Results**:
```
✅ LongCat Anthropic non-streaming: Working
✅ LongCat Anthropic streaming: Working (fixed!)
   - Total chunks: 20
   - Content chunks: 19
   - finish_reason: end_turn
   - usage: prompt_tokens: 15, completion_tokens: 30
```

#### 🧹 Code Cleanup

- Removed deprecated v1 architecture code (5641 lines)
- Removed `v1-legacy` feature flag
- Cleaner codebase with focused abstractions

#### 📚 Documentation

**New Documents**:
- `docs/REFACTORING_SUMMARY.md` - Complete refactoring documentation
- `docs/POST_REFACTORING_TEST_REPORT.md` - Comprehensive test report (90% pass rate)
- `docs/ANTHROPIC_STREAMING_FIX.md` - Anthropic streaming fix details

**Updated**:
- README.md - Added unified output format explanation
- README.md - Added new providers (Tencent, LongCat)

#### ✅ Testing

**Comprehensive Testing**:
- ✅ All providers tested: 10/10 tests passed
- ✅ Non-streaming: 100% pass rate (5/5)
- ✅ Streaming: 100% pass rate (5/5)
- ✅ 46 unit tests passing
- ✅ Full backward compatibility verified

**Tested Providers**:
- Tencent (refactored) - ✅ Non-streaming + Streaming
- LongCat OpenAI (unchanged) - ✅ Non-streaming + Streaming
- LongCat Anthropic (refactored) - ✅ Non-streaming + Streaming (fixed!)
- Zhipu (unchanged) - ✅ Non-streaming + Streaming
- Aliyun (unchanged) - ✅ Non-streaming + Streaming

#### 📈 Performance & Metrics

- Code reduction: -19% in refactored providers
- New provider cost: -70% (170 lines → 50 lines)
- Maintenance cost: -50% (centralized logic)
- Test pass rate: 100% (10/10)

#### 🔄 Migration Guide

**No breaking changes!** All existing APIs continue to work.

**Before (still works)**:
```rust
let client = LlmClient::openai_compatible(
    "sk-...",
    "https://api.hunyuan.cloud.tencent.com",
    "tencent"
)?;
```

**After (recommended)**:
```rust
let client = LlmClient::tencent("sk-...")?;
```

---

## [0.4.19] - 2025-10-18

### ✨ New Features

#### **Add Dedicated Volcengine Provider**

**Volcengine Overview**:
- Volcengine is ByteDance's cloud platform
- Provides LLM services (Ark)
- Uses an OpenAI-compatible API format, but the endpoint path differs

**New Capabilities**:

1. **Create a VolcengineProtocol adapter**
   - Wraps the OpenAI protocol but uses Volcengine endpoint paths
   - Endpoint: `/api/v3/chat/completions` (instead of `/v1/chat/completions`)
   - Fully compatible with OpenAI request/response format

2. **Add dedicated client methods**
   - `LlmClient::volcengine()` - create a Volcengine client
   - `LlmClient::volcengine_with_config()` - client with custom configuration

3. **Support reasoning-model features**
   - Supports the `reasoning_content` field (thinking process)
   - In streaming responses, thinking content arrives before the final answer
   - Similar to OpenAI o1-style reasoning models

**Example**:

```rust
// Create client
let client = LlmClient::volcengine("xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx")?;

// Create request (use endpoint ID)
let request = ChatRequest {
    model: "ep-20250118155555-xxxxx".to_string(),  // endpoint ID
    messages: vec![Message {
        role: Role::User,
        content: "Hello".to_string(),
        ..Default::default()
    }],
    max_tokens: Some(1000),
    ..Default::default()
};

// Non-streaming
let response = client.chat(&request).await?;

// Streaming
#[cfg(feature = "streaming")]
{
    let mut stream = client.chat_stream(&request).await?;
    while let Some(chunk) = stream.next().await {
        // Handle streaming response
    }
}
```

**Test Results**:

| Feature | Status | Details |
|------|------|------|
| Non-streaming response | ✅ | Fully working |
| Streaming response | ✅ | Fully working |
| reasoning_content | ✅ | Thinking process supported |
| llm-connector compatibility | ✅ | Fully compatible |

**New Files**:
- `src/providers/volcengine.rs` - Volcengine dedicated provider
- `examples/test_volcengine.rs` - Test example
- `tests/test_volcengine_raw.sh` - Raw API test
- `tests/test_volcengine_streaming_raw.sh` - Streaming API test
- `docs/VOLCENGINE_GUIDE.md` - Full usage guide

**Important Notes**:
- Volcengine uses endpoint IDs (`ep-xxxxxx`) instead of model names
- Endpoint IDs must be created and retrieved in the Volcengine console
- API keys are UUID format, not `sk-` format

---

## [0.4.18] - 2025-10-18

### ✨ New Features

#### **Add LongCat API Support**

**LongCat Overview**:
- LongCat is an AI service platform providing high-performance chat models
- Supports both OpenAI and Anthropic API formats
- Daily free quota: 500,000 Tokens

**What’s Added**:

1. **LongCat OpenAI Format Support** - ✅ Fully Available
   - Use `LlmClient::openai_compatible()` method
   - Endpoint: `https://api.longcat.chat/openai`
   - Supports non-streaming and streaming responses
   - Fully compatible with llm-connector
   - Works seamlessly with llm-connector

 2. **LongCat Anthropic Format Support** - ✅ Non-streaming Available
    - Create `LongCatAnthropicProtocol` adapter
    - Uses `Authorization: Bearer` auth (instead of standard Anthropic `x-api-key`)
    - Add `LlmClient::longcat_anthropic()` method
    - Add `LlmClient::longcat_anthropic_with_config()` method
    - Supports non-streaming responses
    - ⚠️ Streaming not supported yet (Anthropic event format needs a dedicated parser)

**Example Usage**:

```rust
// Method 1: OpenAI format (recommended, streaming and non-streaming)
let client = LlmClient::openai_compatible(
    "ak_...",
    "https://api.longcat.chat/openai",
    "longcat"
)?;

// Method 2: Anthropic format (non-streaming only)
let client = LlmClient::longcat_anthropic("ak_...")?;
```

**Test Results**:

| Test Item | OpenAI Format | Anthropic Format |
|--------|------------|---------------|
| Non-streaming response | ✅ Success | ✅ Success |
| Streaming response | ✅ Success | ⚠️ Not supported yet |
| llm-connector compatibility | ✅ Fully compatible | ✅ Non-streaming compatible |

**New Files**:
- `src/providers/longcat.rs` - LongCat Anthropic adapter
- `examples/test_longcat_openai.rs` - OpenAI format test
- `examples/test_longcat_anthropic.rs` - Anthropic format test
- `tests/test_longcat_anthropic_raw.sh` - Anthropic raw API test
- `tests/test_longcat_anthropic_streaming_raw.sh` - Streaming format test
- `docs/LONGCAT_TESTING_REPORT.md` - Full testing report

**Recommended Usage**:
- Streaming: `LlmClient::openai_compatible("ak_...", "https://api.longcat.chat/openai", "longcat")`
- Non-streaming: `LlmClient::longcat_anthropic("ak_...")` or the OpenAI format

 ### 🐛 Bug Fixes

#### **Fix Missing Methods in AliyunProviderImpl**

**Issue**: Tests call `provider.protocol()` and `provider.client()`, but these methods did not exist.

**Fix**:
- Add `protocol()` to return a reference to the protocol instance
- Add `client()` to return a reference to the HTTP client
- Fix `models()` error messages to match test expectations
- Fix calls to non-existent methods in the `as_ollama()` doctest

 ### 📝 Documentation

- Add `docs/LONGCAT_TESTING_REPORT.md` - Full LongCat API testing report
- Update `src/client.rs` - Add LongCat usage examples

---

## [0.4.17] - 2025-10-18

### 🐛 Bug Fixes

#### **Fix Aliyun Response Parsing and Streaming**

**Issue 1: Inconsistent ChatResponse structure**

**Symptoms**:
- `choices` was empty
- `content` was populated but not derived from `choices[0]`
- `usage` information was missing

**Fix**:
- Update Aliyun response structs (`src/providers/aliyun.rs`) to include `usage`, `request_id`, and `finish_reason`
- Rebuild `choices` properly and derive convenience `content` from `choices[0].message.content`

**Issue 2: Streaming responses returned no content**

**Root cause**:
- Missing `X-DashScope-SSE: enable` header
- Missing `incremental_output: true` parameter
- Default SSE parsing did not match Aliyun's streaming format

**Fix**:
- Add streaming parameters and required headers
- Implement custom stream parsing and convert to unified `StreamingResponse`

**Result**:
- Non-streaming and streaming both work as expected
- Backward compatible (existing `content` field still works)

### 🧪 Testing

**New Tests**:
1. `examples/test_aliyun_streaming.rs` - Streaming response test
2. `examples/verify_aliyun_choices.rs` - choices array verification
3. `tests/test_aliyun_streaming_format.sh` - API raw response test

### 📝 Documentation

 - Add `docs/ALIYUN_FIXES_SUMMARY.md` - Aliyun fixes summary
 - Add `docs/CHATRESPONSE_DESIGN_ANALYSIS.md` - ChatResponse design analysis
 - Add `docs/ALIYUN_RESPONSE_VERIFICATION.md` - Aliyun response verification report

---

## [0.4.16] - 2025-10-18

### 🐛 Bug Fixes

#### **Fix Duplicate Content-Type Header Causing Provider Failures**

**Problem**:
- Some providers (e.g., Aliyun) failed because `Content-Type` was set twice:
  - Once by `Protocol::auth_headers()`
  - Once by `.json(body)` in the HTTP client

**Fix**:
- Remove manual `Content-Type` from auth headers / provider header builders where `.json()` already sets it
- Apply the same cleanup across multiple providers to avoid duplicate header values

**Affected Providers**:
- ✅ **Aliyun (DashScope)** - Fix failure to use
- ✅ **Zhipu (GLM)** - Fix potential issue
- ✅ **Anthropic (Vertex AI, Bedrock)** - Fix potential issue
- ✅ **Ollama** - Fix potential issue
- ✅ **OpenAI (Azure, Compatible)** - Fix potential issue

**Testing Verification**:
- ✅ Compile successfully
- ✅ Add `examples/test_aliyun_basic.rs` to verify fix
- ✅ All providers no longer set duplicate `Content-Type`

**Fix Statistics**:
- Fixed files: 5
- Fixed providers: 6
- Removed duplicate settings: 9
- Added comments: 9

**Impact**:
- ✅ Fix Aliyun provider failure to use
- ✅ Fix potential compatibility issues in other providers
- ✅ Improve HTTP header setting consistency
- ✅ Fully backward compatible, no user code changes needed

- ✅ **Ollama** - Fix potential issue
- ✅ **OpenAI (Azure, Compatible)** - Fix potential issue

**Testing Verification**:
- ✅ Compile successfully
- ✅ Add `examples/test_aliyun_basic.rs` to verify fix
- ✅ All providers no longer set duplicate `Content-Type`

**Fix Statistics**:
- Fixed files: 5
- Fixed providers: 6
- Removed duplicate settings: 9
- Added comments: 9

**Impact**:
- ✅ Fix critical Aliyun provider failure to use
- ✅ Fix potential compatibility issues in other providers
- ✅ Improve HTTP header setting consistency
- ✅ Fully backward compatible, no user code changes needed

### 🧪 Testing

#### **Add Zhipu Streaming tool_calls Verification Tests**

**New Tests**:
1. `tests/test_zhipu_streaming_direct.sh` - Test Zhipu API raw streaming response
2. `examples/test_zhipu_flash_streaming_tool_calls.rs` - Test llm-connector parsing
3. `examples/debug_zhipu_streaming_tool_calls.rs` - Detailed debug example

**Verification Results**:
- ✅ Zhipu API returns tool_calls in streaming mode
- ✅ llm-connector parses tool_calls correctly
- ✅ Confirms llm-connector 0.4.15 works as expected

### 📝 Documentation

- Add `docs/FIX_DUPLICATE_CONTENT_TYPE_HEADER.md` - Duplicate header fix documentation
- Add `docs/ZHIPU_STREAMING_TOOL_CALLS_VERIFICATION.md` - Zhipu streaming verification report

---

## [0.4.15] - 2025-10-18

### Bug Fixes

#### **Fix Example Compilation Errors and Warnings**

**Fix**:
1. **Remove calls to non-existent methods** (`examples/test_openai_tool_streaming.rs`)
   - Remove calls to `LlmClient::openrouter()` (non-existent)
   - Use `LlmClient::openai()` instead

2. **Fix type errors** (`examples/test_openai_tool_streaming.rs`)
   - Fix tool_calls reference type issues
   - Replace `&tool_calls_buffer[0]` with `tool_calls_buffer[0].clone()`

3. **Reduce unused import warnings** (7 example files)
   - Move streaming imports under `#[cfg(feature = "streaming")]`
   - Avoid unused import warnings when streaming is disabled
   - Affected files:
     - `test_zhipu_tool_messages_detailed.rs`
     - `test_deepseek_tools.rs`
     - `test_openai_tool_streaming.rs`
     - `test_zhipu_tool_streaming_issue.rs`
     - `test_glm_models_tool_streaming.rs`
     - `zhipu_tools_streaming.rs`
     - `test_all_providers_tool_streaming.rs`

4. **Reduce unused field warnings** (`examples/test_all_providers_tool_streaming.rs`)
   - Add `#[allow(dead_code)]` to `TestResult`

5. **Fix clippy warnings**
   - Fix doc comment empty-line warnings
   - Replace `len() > 0` with `!is_empty()`

### Documentation

- Add `docs/EXAMPLES_AND_TESTS_REVIEW.md` - Examples and tests review report
- Add `docs/RELEASE_v0.4.14.md` - v0.4.14 release summary

**Verification**:
- All examples compile
- All tests pass
- No build errors
- Significantly fewer warnings

**Impact**:
- Fix example compilation issues
- Improve code quality
- Fully backward compatible

---

## [0.4.14] - 2025-10-18

### Bug Fixes

#### **Fix OpenAI Tool Calling Support + Remove Zhipu GLM Streaming Forced Fallback**

**Issue 1: OpenAI protocol missing tool calling support**

**Symptoms**:
- Missing fields in request/message/response for tool calling
- Tool calling did not work for providers built on the OpenAI protocol

**Fix**:
1. **Add tool fields to OpenAIRequest** (`src/protocols/openai.rs`)
   ```rust
   pub struct OpenAIRequest {
       // ... other fields
       pub tools: Option<Vec<serde_json::Value>>,      // ✅ added
       pub tool_choice: Option<serde_json::Value>,     // ✅ added
   }
   ```

2. **Add tool fields to OpenAIMessage** (`src/protocols/openai.rs`)
   ```rust
   pub struct OpenAIMessage {
       pub role: String,
       pub content: String,
       pub tool_calls: Option<Vec<serde_json::Value>>,  // ✅ added
       pub tool_call_id: Option<String>,                // ✅ added
       pub name: Option<String>,                        // ✅ added
   }
   ```

3. **Add tool fields to OpenAIResponseMessage** (`src/protocols/openai.rs`)
   ```rust
   pub struct OpenAIResponseMessage {
       pub content: Option<String>,                     // ✅ changed to Option
       pub tool_calls: Option<Vec<serde_json::Value>>,  // ✅ added
   }
   ```

4. **Map tool calling fields in build_request** (`src/protocols/openai.rs:48-106`)
   - Map `tools`
   - Map `tool_choice`
   - Map message fields `tool_calls`, `tool_call_id`, `name`

5. **Parse tool calls in parse_response** (`src/protocols/openai.rs:116-149`)
   - Extract `tool_calls` from the response
   - Convert to the unified `ToolCall` type

**Issue 2: Zhipu GLM streaming was forced to fall back**

**Symptoms**:
- Hard-coded logic in `src/core/traits.rs` forced non-streaming when `Role::Tool` messages were present
- ❌ GLM-4.5 should return many streaming chunks, but tool results caused a forced fallback to a single chunk
- ❌ This was a temporary workaround and is no longer needed

**Fix**:
- Remove the hard-coded workaround (`src/core/traits.rs:155-173`)
- Zhipu GLM streaming now works when tool results are included

**Verification**:
- ✅ OpenAI protocol fully supports tool calling (tools, tool_choice, tool_calls)
- ✅ DeepSeek tool calling works
- ✅ All OpenAI-compatible services can use tool calling
- ✅ Zhipu GLM streaming works with Role::Tool
- ✅ All core library tests pass (27 tests)

**New Example**:
- `examples/verify_tool_fix.rs` - Verify tool calling fix

**Impact**:
- Fix tool calling for all OpenAI-protocol based services
- Remove Zhipu GLM streaming limitation
- Fully backward compatible

---

## [0.4.13] - 2025-10-18

### 🐛 Bug Fixes

#### **Fix Zhipu GLM Multi-round Tool Calling Support**

**Summary**:
- Add missing `tool_call_id` and `name` fields for tool messages
- Ensure multi-round function calling flows work correctly

**Details**:
1. **ZhipuMessage struct updates** (`src/providers/zhipu.rs:272-282`)
   ```rust
   pub struct ZhipuMessage {
       pub role: String,
       pub content: String,
       pub tool_calls: Option<Vec<serde_json::Value>>,
       pub tool_call_id: Option<String>,  // ✅ added
       pub name: Option<String>,          // ✅ added
   }
   ```

2. **build_request mapping updates** (`src/providers/zhipu.rs:77-96`)
   - Map `tool_call_id` correctly
   - Map `name` correctly

**Verification**:
- ✅ Single-round: User prompt → tool_calls returned
- ✅ Multi-round: Tool result → text response returned
- ✅ Three-round: Follow-up triggers new tool_calls
- ✅ Tool message serialization is correct (`role="tool"`, `tool_call_id`, `name`)

**New Examples**:
- `examples/zhipu_multiround_tools.rs` - Multi-round tool calling demo
- `examples/zhipu_tools_edge_cases.rs` - Edge case tests
- `examples/verify_tool_message_serialization.rs` - Serialization verification

**Impact**:
- Fix multi-round tool calling for Zhipu GLM
- Align with OpenAI Function Calling conventions
- Backward compatible

---

## [0.4.12] - 2025-10-18

### 🐛 Bug Fixes

#### **Fix Zhipu GLM Streaming Responses and Tool Calling Support**

**Summary**:
- Implement a Zhipu-specific streaming parser for single-newline SSE
- Ensure `content` is populated correctly for streaming chunks
- Add missing request fields (`stream`, `tools`, `tool_choice`) and response parsing for tool_calls

**Details**:
1. **Streaming parser** (`src/providers/zhipu.rs:126-201`)
   - Implement Zhipu-specific `parse_stream_response()`
   - Support single-newline SSE
   - Populate `content` from `delta.content`

2. **Request fields** (`src/providers/zhipu.rs:216-234`)
   - Add `stream: Option<bool>`
   - Add `tools: Option<Vec<Tool>>`
   - Add `tool_choice: Option<ToolChoice>`

3. **Response parsing** (`src/providers/zhipu.rs:240-264`)
   - Use `#[serde(default)]` for `ZhipuMessage.content` (may be empty for tool calls)
   - Support `ZhipuMessage.tool_calls`
   - Ensure `ZhipuResponse` includes id/created/usage
   - Support `finish_reason` in `ZhipuChoice`

**Verification**:
- Streaming: chunks and output received correctly
- Tool calling works in both non-streaming and streaming modes

**Impact**:
- Only affects the Zhipu GLM provider
- Backward compatible
- Aligns behavior with the OpenAI protocol

**New Examples**:
- `examples/zhipu_tools.rs` - Tool calling (non-streaming)
- `examples/zhipu_tools_streaming.rs` - Tool calling (streaming)

---

## [0.4.11] - 2025-10-17

### 🐛 Bug Fixes

**Initial fix for Zhipu streaming response parsing**
- Implement dedicated `ZhipuProtocol::parse_stream_response()`
- Support single-newline SSE format
- Handle `data:` prefix with or without spaces

---

## [Unreleased]

### 🐛 **BUGFIX: Fix Zhipu Streaming Response Parsing**

**Problem**:
Zhipu API uses single-newline SSE events (`data: {...}\n`) rather than the standard double-newline format. This caused the default SSE parser to produce zero chunks.

**Fix**:
- Add a dedicated `ZhipuProtocol::parse_stream_response()` parser
- Support single-newline SSE
- Handle `data:` prefix with or without spaces
- Skip `[DONE]` and empty payloads
- Provide detailed error messages including raw JSON

**Test Improvements**:
- Update `examples/zhipu_streaming.rs` with chunk counters and parser type hints
  - Use `glm-4-flash` for faster responses
  - Add a warning when zero chunks are produced

#### **Impact**
- ✅ **Fix**: Zhipu streaming API works correctly
- ✅ **Compatibility**: Does not affect other providers' streaming
- ✅ **Debuggability**: Show raw JSON on parse failures

---

### ✨ **FEAT: API Naming Standardization**

#### **Changed**
- **Unified Constructor Naming**
  - `ollama_with_url()` → `ollama_with_base_url()` (kept old name as deprecated)
  - Removed redundant `zhipu_default()` (use `zhipu()` directly)
  - All providers now follow consistent `{provider}_with_base_url()` pattern

#### **Added**
- **Type-Safe Provider Conversions**
  - `LlmClient::as_ollama()` → `Option<&OllamaProvider>`
  - `LlmClient::as_openai()` → `Option<&OpenAIProvider>`
  - `LlmClient::as_aliyun()` → `Option<&AliyunProvider>`
  - `LlmClient::as_anthropic()` → `Option<&AnthropicProvider>`
  - `LlmClient::as_zhipu()` → `Option<&ZhipuProvider>`

- **API Key Validation Functions**
  - `validate_openai_key()`
  - `validate_aliyun_key()`
  - `validate_anthropic_key()` (already existed)
  - `validate_zhipu_key()` (already existed)

- **Advanced Configuration Methods**
  - All `{provider}_with_config()` methods now exposed in `LlmClient`
  - All `{provider}_with_timeout()` methods now exposed in `LlmClient`
  - Cloud-specific methods: `anthropic_vertex()`, `anthropic_bedrock()`, `aliyun_international()`, etc.

#### **Documentation**
- **NEW**: `docs/NAMING_CONVENTIONS.md` - Comprehensive naming standards guide
- **NEW**: `.augment/rules/naming.md` - Qoder auto-check rules
- Updated all examples to use new naming conventions

#### **Deprecated**
- `LlmClient::ollama_with_url()` → Use `ollama_with_base_url()`
- `providers::zhipu_default()` → Use `zhipu()` directly
- `LlmClient::ollama()` (the method, not constructor) → Use `as_ollama()`

#### **Impact**
- ✅ **Consistency**: All providers follow same naming pattern
- ✅ **Type Safety**: No more manual `downcast_ref` needed
- ✅ **Completeness**: All provider variants exposed in `LlmClient`
- ✅ **Documentation**: Clear naming rules for contributors

---

## [0.4.9] - 2025-10-16

### 📚 **DOCS: Fix API Documentation and Examples**

#### **Fixed**
- **README API Examples** - Updated streaming API examples to reflect current V2 architecture
  - Replaced deprecated `chat_stream_universal()`, `chat_stream_sse()`, `chat_stream_ndjson()` with current `chat_stream()`
  - Updated streaming examples to use `StreamingResponse` and `get_content()` method
  - Added clear distinction between V2 (current) and V1 (legacy) APIs in changelog
  - Fixed 29 documentation tests that used incorrect import paths

#### **Added**
- **New Example**: `streaming_v2_demo.rs` - Demonstrates current V2 streaming API
- **API Clarification**: Clear documentation of current streaming interface
- **Migration Guide**: Explains differences between V1 and V2 streaming APIs

#### **Impact**
- ✅ **Documentation**: All examples now reflect current API
- ✅ **Tests**: All 93 tests pass (including 50 documentation tests)
- ✅ **Clarity**: Clear separation between current and legacy APIs
- ✅ **Examples**: Working examples for current streaming interface

## [0.4.8] - 2025-10-16

### 🔧 **REFACTOR: Simplify Configuration Module Structure**

#### **Simplified**
- **Configuration Module** - Simplified `src/config/` directory to single `src/config.rs` file
  - Eliminated confusion between `src/config/provider.rs` and `src/providers/` directory
  - Consolidated all configuration types into single, clear module
  - Maintained all existing functionality and API compatibility
  - All 28 unit tests pass

#### **Structure Changes**
- **Before**: `src/config/mod.rs` + `src/config/provider.rs` (confusing)
- **After**: `src/config.rs` (clear and simple)
- **Benefits**: No naming confusion, easier to find configuration code, simpler maintenance

#### **Impact**
- ✅ **Clarity**: Eliminated naming confusion with providers
- ✅ **Simplicity**: Single file for all configuration needs
- ✅ **Maintainability**: Easier to locate and modify configuration code
- ✅ **Compatibility**: No breaking changes to public API

## [0.4.7] - 2025-10-16

### 🏗️ **ARCHITECTURE: Correct Protocol vs Provider Separation**

#### **Refactored**
- **Protocol/Provider Architecture** - Implemented correct separation of public vs private protocols
  - **Public Protocols** (`src/protocols/`): Only industry-standard protocols (OpenAI, Anthropic)
  - **Private Protocols** (`src/providers/`): Provider-specific protocols inline with implementations
  - Moved `AliyunProtocol` and `ZhipuProtocol` from `protocols/` to `providers/` as private protocols
  - Updated exports: Standard protocols from `protocols`, private protocols from `providers`
  - All 78 unit and integration tests pass

#### **Design Principles**
- **Public Protocols**: Industry-recognized standards that multiple providers might implement
- **Private Protocols**: Provider-specific APIs that are tightly coupled to their implementations
- **Clean Separation**: Protocols define API formats, providers implement service logic
- **Maintainability**: Private protocols are co-located with their implementations

#### **Impact**
- ✅ **Architecture**: Clean separation of public vs private protocols
- ✅ **Maintainability**: Private protocols are easier to maintain alongside providers
- ✅ **Extensibility**: Clear guidelines for adding new protocols vs providers
- ✅ **Tests**: All functionality tests pass (78/78)

## [0.4.6] - 2025-10-16

### 🔧 **HOTFIX: Streaming Integration Test Errors**

#### **Fixed**
- **Streaming integration test compilation errors** - Fixed all compilation errors in streaming tests
  - Fixed `tests/streaming_integration_tests.rs`: Added missing `Role` import
  - Updated all `Message::user()` calls to use proper `Message` construction with `Role::User`
  - Fixed all client creation calls: `.unwrap()` → `?` for V2 architecture
  - Fixed error handling test to properly detect authentication errors
  - All streaming integration tests now pass (4/4 passed, 4 ignored for API keys)

#### **Impact**
- ✅ **Streaming Tests**: All streaming integration tests compile and pass
- ✅ **Test Coverage**: Complete test coverage for streaming functionality
- ✅ **V2 Architecture**: All tests use correct V2 architecture APIs

## [0.4.5] - 2025-10-16

### 🔧 **HOTFIX: Test Compilation Errors**

#### **Fixed**
- **Test compilation errors** - Fixed compilation errors in test files
  - Fixed `tests/client_tests.rs`: Updated `protocol_name()` → `provider_name()` method calls
  - Fixed main documentation tests in `src/lib.rs` and `src/client.rs`
  - Updated import statements to use correct V2 architecture paths
  - All unit tests and integration tests now pass successfully

#### **Impact**
- ✅ **Tests**: All unit and integration tests compile and pass (78/78)
- ✅ **Documentation**: Main documentation examples work correctly
- ✅ **CI/CD**: Test suite runs successfully for automated builds

## [0.4.4] - 2025-10-16

### 🔧 **HOTFIX: Examples Compilation Errors**

#### **Fixed**
- **Examples compilation errors** - Fixed all compilation errors and warnings in example files
  - Updated `examples/zhipu_basic.rs`: Fixed API calls and imports for V2 architecture
  - Updated `examples/zhipu_streaming.rs`: Fixed Message construction and client creation
  - Updated `examples/streaming_basic.rs`: Fixed imports and Result handling
  - Updated `examples/ollama_model_management.rs`: Fixed Ollama provider interface usage
  - Updated `examples/v1_vs_v2_comparison.rs`: Removed deprecated feature flags
  - All examples now use V2 architecture APIs correctly

#### **Impact**
- ✅ **Examples**: All examples compile and run successfully
- ✅ **Documentation**: Examples serve as accurate V2 architecture documentation
- ✅ **User Experience**: Users can run examples without compilation errors

## [0.4.3] - 2025-10-16

### 🔧 **HOTFIX: Module Privacy Error**

#### **Fixed**
- **Critical module privacy error** - Fixed private module access in streaming functionality
  - Fixed import path: `crate::types::streaming::ChatStream` → `crate::types::ChatStream`
  - Fixed import path: `crate::types::streaming::StreamingResponse` → `crate::types::StreamingResponse`
  - The `streaming` module is conditionally exported and should be accessed through `types` module
  - Affected file: `src/sse.rs`

#### **Impact**
- ✅ **Compilation**: Now compiles successfully without privacy errors
- ✅ **Streaming**: All streaming features work correctly
- ✅ **Functionality**: No breaking changes to public API

## [0.4.2] - 2025-10-16

### 🔧 **HOTFIX: Type Mismatch Error**

#### **Fixed**
- **Critical type mismatch error** - Fixed streaming response type conversion
  - Added `sse_to_streaming_response()` function to convert `String` stream to `StreamingResponse` stream
  - Fixed type mismatch: expected `StreamingResponse` but found `String` in streaming methods
  - Affected files: `src/sse.rs`, `src/core/traits.rs`, `src/protocols/zhipu.rs`, `src/providers/ollama.rs`
  - All streaming functionality now works correctly with proper type conversion

#### **Impact**
- ✅ **Compilation**: Now compiles successfully without type errors
- ✅ **Streaming**: All streaming features work with correct types
- ✅ **Functionality**: No breaking changes to public API

## [0.4.1] - 2025-10-16

### 🔧 **HOTFIX: Compilation Error**

#### **Fixed**
- **Critical compilation error** - Fixed unresolved import `crate::sse::SseStream`
  - Replaced incorrect `SseStream::new(response)` calls with `crate::sse::sse_events(response)`
  - Affected files: `src/core/traits.rs`, `src/protocols/zhipu.rs`, `src/providers/ollama.rs`
  - All streaming functionality now works correctly

#### **Impact**
- ✅ **Compilation**: Now compiles successfully without errors
- ✅ **Streaming**: All streaming features work as expected
- ✅ **Functionality**: No breaking changes to public API

## [0.4.0] - 2025-10-16

### 🚀 **MAJOR RELEASE: V2 Architecture**

This is a major release that introduces the new V2 architecture as the default, providing significant performance improvements and a cleaner API design.

#### ⚡ **Performance Improvements**
- **7,000x+ faster client creation** - From ~53ms to ~7µs
- **Minimal memory footprint** - Only 16 bytes per client instance
- **Zero-cost cloning** - Arc-based sharing for efficient cloning

#### 🏗️ **New Architecture**
- **Clear Protocol/Provider separation** - Protocols define API specs, Providers implement services
- **Unified trait system** - `Protocol` and `Provider` traits for maximum extensibility
- **Type-safe HTTP client** - Compile-time guarantees for correctness
- **Generic provider implementation** - `GenericProvider<Protocol>` for most use cases

#### 🔄 **API Changes (Breaking)**

##### **Client Creation**
```rust
// V1 (Legacy)
let client = LlmClient::openai("sk-...", None);
let client = LlmClient::ollama(None);

// V2 (New Default)
let client = LlmClient::openai("sk-...")?;  // Returns Result
let client = LlmClient::ollama()?;          // Returns Result
```

##### **Method Renames**
```rust
// V1 → V2
client.fetch_models()  → client.models()
client.protocol_name() → client.provider_name()
```

##### **Parameter Changes**
- **OpenAI**: Removed optional second parameter, use dedicated methods
  - `openai(key, Some(url))` → `openai_with_base_url(key, url)?`
- **Ollama**: Removed optional parameter
  - `ollama(Some(url))` → `ollama_with_url(url)?`

#### 🆕 **New Features**

##### **Additional Client Creation Methods**
```rust
// Azure OpenAI support
LlmClient::azure_openai("key", "endpoint", "version")?

// OpenAI-compatible services
LlmClient::openai_compatible("key", "url", "name")?

// Zhipu GLM OpenAI-compatible mode
LlmClient::zhipu_openai_compatible("key")?

// Enhanced configuration options
LlmClient::openai_with_config("key", url, timeout, proxy)?
```

##### **Enhanced Ollama Support**
```rust
// Direct access to Ollama-specific features
if let Some(ollama) = client.as_ollama() {
    ollama.pull_model("llama2").await?;
    let models = ollama.models().await?;
}
```

#### 📦 **Protocol Support**
- **OpenAI Protocol** - Complete OpenAI API specification
- **Anthropic Protocol** - Full Claude API support with Vertex AI and Bedrock
- **Aliyun Protocol** - DashScope API with regional support
- **Zhipu Protocol** - Native and OpenAI-compatible formats
- **Ollama Provider** - Custom implementation with model management

#### 🔄 **Migration Guide**

##### **Option 1: Backward Compatibility**
```toml
# Cargo.toml
[features]
v1-legacy = []
```

```rust
// Use V1 API
#[cfg(feature = "v1-legacy")]
use llm_connector::v1::LlmClient;

// Use V2 API (default)
#[cfg(not(feature = "v1-legacy"))]
use llm_connector::LlmClient;
```

##### **Option 2: Direct Migration**
1. Add `?` to handle `Result` return types
2. Update method names: `fetch_models()` → `models()`, `protocol_name()` → `provider_name()`
3. Replace parameter patterns with dedicated methods
4. Update imports if using internal traits

#### 🏛️ **Architecture Benefits**
- **Extensibility** - Easy to add new protocols and providers
- **Type Safety** - Compile-time guarantees for all operations
- **Performance** - Optimized for speed and memory efficiency
- **Clarity** - Clear separation of concerns between protocols and providers
- **Maintainability** - Reduced code duplication and cleaner abstractions

## [0.3.6] - 2025-10-14

### ✨ Added

#### Ollama Streaming Support
- Implemented line-delimited JSON streaming for Ollama protocol
  - Added non-SSE parser for JSON lines stream
  - Integrated into core streaming pipeline with protocol switch
  - Normalized to `StreamingResponse` with `get_content()` for output
- Added `examples/ollama_streaming.rs` demonstrating `chat_stream()` usage

### 📝 Updated
- README and examples already standardized to use `get_content()` for streaming output

## [0.2.3] - 2025-01-06

### ✨ Added

#### Improved Error Messages
- **Cleaned up authentication error messages** for OpenAI-compatible providers
  - Removes OpenAI-specific URLs (like "platform.openai.com") from error messages
  - Adds helpful context: "Please verify your API key is correct and has the necessary permissions"
  - Makes errors more generic and applicable to all OpenAI-compatible providers (DeepSeek, Zhipu, Moonshot, etc.)

#### New Debug Tools
- **Added `debug_deepseek.rs` example** for troubleshooting authentication issues
  - Validates API key format
  - Tests model fetching and chat requests
  - Provides specific troubleshooting guidance based on error type
  - Can accept API key from command line or environment variable

#### Documentation
- **Added `TROUBLESHOOTING.md`** - Comprehensive troubleshooting guide
  - Authentication errors and solutions
  - Connection errors and debugging steps
  - Rate limit handling
  - Model not found errors
  - Provider-specific instructions for DeepSeek, OpenAI, Zhipu, Moonshot
  - Example code for common scenarios

### 🔧 Changed

#### Simplified API - Removed `supported_models()`
- **Removed `supported_models()` method** from all traits and implementations
  - Removed from `Provider` trait
  - Removed from `ProviderAdapter` trait
  - Removed from `LlmClient`
  - Removed from all protocol implementations (OpenAI, Anthropic, Aliyun, Ollama)
- **Removed `supports_model()` method** from `Provider` trait (was dependent on `supported_models()`)
- **Removed hardcoded model lists** from protocol structs
  - Removed `supported_models` field from `AnthropicProtocol`
  - Removed `supported_models` field from `AliyunProtocol`
  - Removed `supported_models` field from `OllamaProtocol`

#### Rationale
- `supported_models()` returned empty `[]` for most protocols (OpenAI, Anthropic, Aliyun)
- Only Ollama had hardcoded models, which were outdated
- Users should use `fetch_models()` for real-time model discovery
- Simplifies the API by removing confusion between two similar methods

#### Migration Guide

**Before:**
```rust
let client = LlmClient::openai("sk-...");
let models = client.supported_models(); // Returns []
```

**After:**
```rust
let client = LlmClient::openai("sk-...");
let models = client.fetch_models().await?; // Returns actual models from API
```

For protocols that don't support `fetch_models()` (Anthropic, Aliyun, Ollama), you can use any model name directly in your requests.

### 📝 Updated

- Updated tests to remove `supported_models()` usage
- Updated examples to demonstrate only `fetch_models()`
- Updated README.md to remove references to `supported_models()`
- Simplified documentation and examples

## [0.2.2] - 2025-01-06

Same as 0.2.1 - version bump for crates.io publication.

## [0.2.1] - 2025-01-06

### ✨ Added

#### Online Model Discovery
- **New `fetch_models()` method** for retrieving available models from API
  - Added to `Provider` trait, `LlmClient`, and `GenericProvider`
  - Makes GET request to `/v1/models` endpoint for OpenAI-compatible providers
  - Returns `Vec<String>` of available model IDs
  - Returns `UnsupportedOperation` error for protocols without model listing support

#### HTTP Transport Enhancement
- Added `get()` method to `HttpTransport` for GET requests
- Supports custom headers and authentication

#### Error Handling
- Added `UnsupportedOperation` error variant for unsupported operations
- Returns HTTP 501 status code for unsupported operations

#### Examples
- `examples/test_fetch_models.rs` - Comprehensive test with all providers
- `examples/fetch_models_simple.rs` - Simple comparison example
- `examples/test_with_keys.rs` - Test with keys.yaml configuration

#### Documentation
- `FETCH_MODELS_FEATURE.md` - Complete feature documentation
- `TEST_RESULTS.md` - Test results and verification
- Updated README.md with model discovery section
- Added comparison table for `supported_models()` vs `fetch_models()`

### 🔧 Changed

#### OpenAI Protocol
- **Removed hardcoded model lists** from `OpenAIProtocol`
- `supported_models()` now returns empty `[]` instead of hardcoded models
- Users can now use **any model name** without restrictions
- Implemented `models_endpoint_url()` to support `/v1/models` endpoint

#### Documentation Cleanup
- Removed references to third-party providers (DeepSeek, Zhipu, Moonshot, etc.) from OpenAI protocol docs
- Updated examples to focus on OpenAI instead of third-party providers
- Simplified documentation to emphasize protocol-first approach

#### Provider Type Aliases
- Removed provider-specific type aliases:
  - `DeepSeekProvider`
  - `ZhipuProvider`
  - `MoonshotProvider`
  - `VolcEngineProvider`
  - `TencentProvider`
  - `MiniMaxProvider`
  - `StepFunProvider`

### 🐛 Fixed

#### Configuration
- Fixed `keys.yaml` model names:
  - Removed invalid `qwen3-turbo` model
  - Updated to valid Aliyun models: `qwen-turbo`, `qwen-plus`, `qwen-max`
  - Updated Qwen2 models to Qwen2.5 versions

#### Dependencies
- Added `serde_yaml` to `[dev-dependencies]` for examples
- Fixed `serde_yaml` resolution in test examples

#### Code Quality
- Removed unused imports (`HttpTransport`, `LlmConnectorError` from openai.rs)
- Fixed struct field issues (removed incorrect `transport` field)

### 📊 Test Results

#### Successfully Tested Providers (Online Model Fetching)

| Provider | Status | Models Found | Example Models |
|----------|--------|--------------|----------------|
| DeepSeek | ✅ | 2 | `deepseek-chat`, `deepseek-reasoner` |
| Zhipu (GLM) | ✅ | 3 | `glm-4.5`, `glm-4.5-air`, `glm-4.6` |
| Moonshot | ✅ | 12 | `moonshot-v1-32k`, `kimi-latest`, `kimi-thinking-preview` |
| LongCat | ❌ | - | `/models` endpoint not available |
| VolcEngine | ❌ | - | `/models` endpoint not available |
| Aliyun | ℹ️ | - | Protocol doesn't support model listing |
| Anthropic | ℹ️ | - | Protocol doesn't support model listing |

### 📝 Migration Guide

#### For Users Relying on Hardcoded Models

**Before (v0.2.0):**
```rust
let client = LlmClient::openai("sk-...");
let models = client.supported_models();
// Returns: ["gpt-4", "gpt-3.5-turbo", "gpt-4-turbo"]
```

**After (v0.2.1):**
```rust
let client = LlmClient::openai("sk-...");

// Option 1: Use any model name directly (recommended)
let request = ChatRequest {
    model: "gpt-4o".to_string(), // Any model name works
    // ...
};

// Option 2: Fetch models online
let models = client.fetch_models().await?;
// Returns: actual models from OpenAI API
```

#### For OpenAI-Compatible Providers

**Before:**
```rust
// Had to check hardcoded list
let models = client.supported_models();
```

**After:**
```rust
// Fetch real-time models from provider
let client = LlmClient::openai_compatible(
    "sk-...",
    "https://api.deepseek.com/v1"
);
let models = client.fetch_models().await?;
// Returns: ["deepseek-chat", "deepseek-reasoner"]
```

### 🎯 Benefits

1. **No Model Restrictions**: Use any model name without being limited by hardcoded lists
2. **Always Up-to-Date**: Get the latest models directly from the API
3. **Backward Compatible**: Existing code continues to work
4. **Flexible**: Providers can opt-in to model listing support
5. **Clear Errors**: Explicit error messages when operations aren't supported

### 🔗 Related Issues

- Fixed errors in `src/protocols/openai.rs`
- Removed hardcoded `supported_models`
- Implemented online model fetching (Option 3)
- Updated documentation to reflect changes

### 📚 Documentation

- README.md: Added "Key Features" section
- README.md: Added "Model Discovery" section with comparison table
- README.md: Added "Recent Changes" section
- README.md: Updated error handling examples
- README.md: Updated examples section

### 🧪 Testing

All tests passing:
```bash
cargo check --lib                    # ✅ Success
cargo run --example test_openai_only # ✅ All tests passed
cargo run --example test_with_keys   # ✅ 6/6 providers tested
cargo run --example test_fetch_models # ✅ Online fetching works
```

---

## [0.2.0] - Previous Release

Initial release with 4 protocol support and basic functionality.

---

## Future Enhancements

Potential improvements for future releases:

1. **Model Caching**: Cache fetched models to reduce API calls
2. **Model Metadata**: Return full model objects with capabilities, not just IDs
3. **Model Filtering**: Add parameters to filter models by capability
4. **Extended Protocol Support**: Implement model listing for other protocols if available
5. **Pagination Support**: Handle paginated model responses
## [0.3.3] - 2025-10-14

### ✨ Added
- README: Added “Reasoning Synonyms” section with normalized keys and usage examples (`reasoning_any()`), covering non-streaming and streaming.

### 🔧 Changed
- Examples: Removed outdated examples using deprecated `openai_compatible` (`examples/test_fetch_models.rs`, `examples/test_with_keys.rs`).
- Examples: Updated DeepSeek and fetch models example to use `LlmClient::openai(api_key, Some(base_url))`.
- Docs: Fixed doctests across `lib.rs`, `protocols/core.rs`, `protocols/openai.rs`, `protocols/aliyun.rs`, `protocols/anthropic.rs` to match current API.
- Docs: Replaced obsolete imports like `protocols::aliyun::qwen` and `protocols::anthropic::claude` with `LlmClient::aliyun(...)` and `LlmClient::anthropic(...)`.
- Docs: Standardized message initialization to `Message::user(...)` or `Role` enums where appropriate.

### ✅ Validation
- `cargo build --examples` passes.
- `cargo test` (library and integration with `streaming` feature) passes.
- `cargo test --doc` passes (13 passed, 0 failed, 4 ignored).
## 0.3.4 - 2025-10-14

Updates
- Add compatibility alias `types::ChatMessage = Message` to ease migration.
- Add `ChatResponse::get_usage_safe()` returning `(prompt, completion, total)`.
- Add `ChatResponse::get_content()` returning the first choice content as `Option<&str>`.
- README install snippet updated to `0.3.4`.

Notes
- `ChatRequest::new(model)` remains as minimal constructor.
- Use `ChatRequest::new_with_messages(model, messages)` to pass initial message list.
- `Message::user/assistant/system` are preferred constructors; reasoning fields are auto-populated.

## 0.3.5 - 2025-10-14

Updates
- Add `StreamingResponse::get_content()` for convenience and API symmetry with `ChatResponse::get_content()`.

Notes
- No breaking changes; existing code continues to work. You can still access `choices[0].delta.content` directly.

