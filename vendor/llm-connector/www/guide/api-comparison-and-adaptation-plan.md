# OpenAI API vs Anthropic API: Comparison and llm-connector Compatibility Audit

## 1. API Comparison

### 1.1 Interface Design

| Dimension | OpenAI Chat Completions | Anthropic Messages |
|-----------|-------------------------|--------------------|
| **Message model** | `messages` array with role/content per item | `messages` array + separate `system` field |
| **Roles** | user, assistant, system, tool | user, assistant (system in separate field) |
| **Content format** | content as string or content blocks array | Always content blocks array |
| **Endpoint** | `/v1/chat/completions` | `/v1/messages` |
| **Style** | Mature, many parameters, broad compatibility | Clean structure, XML support, consistent output |

### 1.2 Authentication

| Dimension | OpenAI | Anthropic |
|-----------|--------|-----------|
| **Primary auth** | `Authorization: Bearer {api_key}` | `x-api-key: {api_key}` |
| **Version header** | None required | `anthropic-version: 2023-06-01` |
| **Azure variant** | `api-key` + `api-version` | Bearer via Vertex/Bedrock proxy |

### 1.3 Request/Response Format

| Field | OpenAI | Anthropic |
|-------|--------|-----------|
| Model | `model` | `model` |
| Max tokens | `max_tokens` optional | `max_tokens` required |
| Temperature | `temperature` | `temperature` |
| Output | `choices[]` with `message` | Single `content[]`, no choices |
| Token stats | `usage.prompt_tokens` / `completion_tokens` | `usage.input_tokens` / `output_tokens` |
| Stop reason | `finish_reason` | `stop_reason` |

### 1.4 Error Codes

| HTTP | OpenAI | Anthropic |
|------|--------|-----------|
| 400 | `invalid_request_error` | `invalid_request_error` |
| 401 | `authentication_error` | `authentication_error` |
| 403 | `permission_error` | `permission_error` |
| 429 | `rate_limit_error` | `rate_limit_error` |
| 500+ | Server error | Server error |
| Special | `context_length_exceeded` (code) | Detected via message content |

### 1.5 Streaming

| Dimension | OpenAI | Anthropic |
|-----------|--------|-----------|
| **Protocol** | SSE | SSE |
| **Events** | `data:` lines, `choices[0].delta` | `message_start`, `content_block_delta`, `message_delta`, `message_stop` |
| **Structure** | Incremental delta | Block-level delta + metadata |
| **tool_calls** | Incremental accumulation by index | Parse `content_block_delta` for tool_use |

### 1.6 Function Calling

| Dimension | OpenAI | Anthropic |
|-----------|--------|-----------|
| **Definition** | `tools[].function` (name/description/parameters) | `tools[].input_schema` (JSON Schema) |
| **Calls** | `tool_calls[]` with id/function/name/arguments | `tool_use` blocks in `content` |
| **Results** | `tool` role + `tool_call_id` | `tool_result` in `user` role |
| **tool_choice** | auto/none/required/function | No direct equivalent, prompt-based control |

### 1.7 Multimodal

| Dimension | OpenAI | Anthropic |
|-----------|--------|-----------|
| **Image input** | `image_url` (URL or base64) | `image` (base64, type: image/source) |
| **Blocks** | text, image_url | text, image |
| **Formats** | URL and base64 | Primarily base64 |

### 1.8 Rate Limits

Both use RPM/TPM, per-model; OpenAI exposes `x-ratelimit-*` headers; both return 429 with Retry-After behavior.

### 1.9 Pricing

Both charge by token; input/output rates differ. llm-connector does not implement billing.

### 1.10 SDK Ecosystem

Both provide official Python and TypeScript SDKs. llm-connector offers a unified Rust layer.

---

## 2. Version Evolution (since 2022)

### OpenAI

| Date | Milestone |
|------|-----------|
| 2022 | Chat Completions release |
| 2023 | Function calling, Vision, GPT-4 Turbo |
| 2024 | o1 reasoning, structured outputs, reasoning_content |
| 2025-03 | **Responses API** released, recommended for new projects |
| 2025-05 | MCP, Code Interpreter, Image generation, Background mode |
| 2026-08 | Assistants API planned sunset |

### Anthropic

| Date | Milestone |
|------|-----------|
| 2023 | Messages API, `anthropic-version: 2023-06-01` |
| 2024 | Tool use (function calling), extended thinking |
| 2025 | Version updates, built-in tools |
| 2026-02 | Built-in tools GA |

---

## 3. llm-connector Compatibility Audit

### 3.1 OpenAI Coverage

| Endpoint/Feature | Supported | Notes |
|------------------|-----------|-------|
| **POST /chat/completions** | ✓ | |
| └ model, messages, temperature, max_tokens, top_p | ✓ | |
| └ frequency_penalty, presence_penalty | ✓ | |
| └ stream, tools, tool_choice, response_format | ✓ | |
| └ stop, logit_bias, user, seed | ✗ | Not mapped to OpenAIRequest |
| **GET /models** | ✓ | |
| **Streaming** | ✓ | delta.content, tool_calls accumulation, reasoning_content |

**Unmapped (ChatRequest → OpenAIRequest):** stop, logit_bias, user, seed

### 3.2 Anthropic Coverage

| Endpoint/Feature | Supported | Notes |
|------------------|-----------|-------|
| **POST /v1/messages** | ✓ | |
| └ model, max_tokens, messages, system, temperature, top_p, stream | ✓ | |
| └ tools, tool_choice, response_format | ✗ | Not implemented |
| **Streaming** | ✓ | message_start, content_block_delta, message_delta |
| **tool_use streaming** | ✗ | Tool results converted to user text, no real tool support |
| **models list** | ✗ | models_endpoint returns None |

### 3.3 Gap Matrix and Priority

| ID | Gap | Type | Priority | Description |
|----|-----|------|----------|-------------|
| D1 | Anthropic tools | Missing | **P0** | No function calling |
| D2 | Anthropic tool streaming | Missing | **P0** | tool_use not parsed |
| D3 | OpenAI stop/logit_bias/user/seed | Unmapped | P1 | Affects advanced usage |
| D4 | Anthropic models endpoint | Missing | P1 | fetch_models unsupported |
| D5 | OpenAI logprobs | Unmapped | P2 | Low priority |
| D6 | response_format (Anthropic) | Missing | P2 | Limited structured output |
| D7 | Tool role → user text | Semantic mismatch | P1 | Tool results treated as plain user messages |

---

## 4. Adaptation Plan

### 4.1 New Modules

- **OpenAI Responses Protocol**: Add `OpenAIResponsesProtocol` for `/v1/responses`; see [OpenAI Responses API Analysis](./openai-responses-api-analysis-and-plan)
- Optional: Anthropic tools adapter for `input_schema` ↔ `function.parameters` mapping

### 4.2 Refactoring

| Module | Changes |
|--------|---------|
| `anthropic.rs` | Add tools, tool_choice in build_request; tool_use in parse_response / parse_stream_response |
| `openai.rs` | Optionally add stop, logit_bias, user, seed to OpenAIRequest |
| `types/request.rs` | Add or extend `ResponsesRequest` if adopting Response API |

### 4.3 Integration Tests

- `openai_tools_e2e`: Non-streaming + streaming tool calls
- `anthropic_tools_e2e`: Anthropic tool use (requires API key or mock)
- `anthropic_stream_tool_use`: Anthropic streaming tool_use parsing
- `openai_stop_sequences`: stop parameter behavior
- `responses_api_migrate`: Response API basic request/response (once implemented)

### 4.4 Documentation and Examples

- README: Anthropic tools support
- [Providers](./providers): Anthropic capability matrix
- [Tools](./tools): Anthropic tool use example
- `examples/`: Add `anthropic_tool_use.rs` if implemented

### 4.5 Milestones and Acceptance

| Milestone | Scope | Acceptance |
|-----------|-------|------------|
| **M1** | Anthropic tools (P0) | ≥95% unit, ≥90% integration, zero P0 defects |
| **M2** | OpenAI unmapped fields (P1) | Same |
| **M3** | OpenAI Responses API | Same + dedicated Response API tests |
| **M4** | Docs and examples | All above updates done |
