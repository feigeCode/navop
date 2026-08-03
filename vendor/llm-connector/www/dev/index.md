# Development Status

Here you can find the current development status and recent updates for `llm-connector`.

## Current Version: 0.7.2

### [0.7.2] - 2026-02-24

#### 📝 Documentation

- Added comprehensive changelog and development status to website
- Updated website configuration with new development section

### [0.7.1] - 2026-02-23

#### 🔧 Maintenance

- Resolved clippy warnings and stabilized tests
- Fixed cargo fmt issues in CI

### [0.7.0] - 2026-02-23

#### Added

- **Per-Request Overrides (Multi-Tenant / Gateway)**
  - `ChatRequest` now supports `api_key`, `base_url`, and `extra_headers` for per-request overrides
  - `with_api_key()`, `with_base_url()`, `with_header()`, `with_extra_headers()` builder methods
  - Supports multi-tenant routing without creating a new client per tenant
  - Custom headers (e.g. `X-Trace-Id`, `anthropic-version`) override default provider headers
  - Works with all `GenericProvider`-based providers (OpenAI, Anthropic, DeepSeek, Moonshot, Volcengine, etc.)

### [0.6.1] - 2026-02-20

#### 🔧 Build / Compatibility

- Rust 2024 edition (MSRV: Rust 1.85+)
- Reqwest uses `rustls-tls` by default (better Android cross-compilation compatibility)

### [0.6.0] - 2026-02-15

#### 🚀 New Features

- **Rust 2024 edition** — MSRV is now Rust 1.85+

- **Builder Pattern for LlmClient** — `LlmClient::builder()` provides a fluent API for client construction with optional `base_url()`, `timeout()`, `proxy()` configuration. Supports all 12+ providers.
  ```rust
  let client = LlmClient::builder()
      .deepseek("sk-...")
      .timeout(60)
      .build()?;
  ```

- **Zhipu Multimodal Support** — Zhipu protocol now supports image URLs and base64 images via `MessageBlock::image_url()` and `MessageBlock::image_base64()`. Works with `glm-4v-flash` and other vision models.

#### ⚡ Breaking Changes (minor)

- **Streaming now enabled by default** — `streaming` feature is included in `default` features, so `chat_stream()`, `ChatStream`, `StreamingResponse` etc. are available without extra configuration. Downstream libraries no longer need `features = ["streaming"]`.

## Recent History

### [0.5.17] - 2026-02-14

#### 🚀 New Features

- **Mock Client for Testing**
  - New `MockProvider` for unit testing without real API calls
  - `MockProviderBuilder` with fluent API for fine-grained control
  - `LlmClient::mock("content")` one-liner for simple cases
  - Sequential response mode for multi-turn test scenarios
  - Error simulation support for testing error handling paths
  - Request tracking via `as_mock().request_count()` / `get_requests()`
  - Tool call simulation via `MockProviderBuilder::with_tool_calls()`

### [0.5.16] - 2026-02-14

#### 🚀 New Features

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

#### 📝 Documentation

- Updated README with new Tool Calling and Structured Output examples
- Updated docs/TOOLS.md with convenience API usage
- Added integration test example: `examples/test_wishlist.rs`

For more details, please refer to the [Full Changelog](https://github.com/lipish/llm-connector/blob/main/CHANGELOG.md).
