# OpenAI Responses API Analysis and llm-connector Adaptation Plan

## 1. Overview

The OpenAI Responses API was released in March 2025 as the successor to Chat Completions and is recommended for new projects.

- **Endpoint**: `POST /v1/responses` (Chat Completions uses `POST /v1/chat/completions`)
- **Purpose**: Unified, agent-oriented interface with native tools, multi-turn conversation, and state management

---

## 2. Key Format Differences

### 2.1 Input

| Chat Completions | Responses API |
|------------------|---------------|
| `messages` array | `input`: string / message array / Items array |
| No dedicated instructions | `instructions`: top-level system guidance |
| No state | `previous_response_id`: reference to prior response |
| - | `store`: persist context |

### 2.2 Output

| Chat Completions | Responses API |
|------------------|---------------|
| `choices[]` with `message` | `output[]`: Items array |
| `choices[0].message.content` | `output_text`: convenience field |
| No response-level id | `response` object has `id` |
| `n` for parallel generations | `n` removed, single generation per request |

### 2.3 Items Model

Items are a union type including:
- `message`: text or multimodal message
- `function_call` / `function_call_output`: tool calls and results
- Others: reasoning, image, etc.

Unlike Chat Completions (tool_calls embedded in messages), Responses uses separate Item types for tool calls and messages.

### 2.4 Function Calling

- Chat Completions: externally tagged, non-strict by default
- Responses: internally tagged, strict by default
- Tool calls correlated via `call_id`, not embedded in message

### 2.5 Structured Outputs

- Chat Completions: `response_format`
- Responses: `text.format` (inside top-level `text` object)

---

## 3. Responses API Benefits

| Capability | Description |
|------------|-------------|
| **Cost** | Better cache usage, ~40–80% cost reduction vs Chat Completions |
| **State** | `store: true` keeps reasoning and tool context |
| **Agent** | Multiple tools in one request (web_search, file_search, code_interpreter, MCP, etc.) |
| **ZDR** | `reasoning.encrypted_content` + `store: false` for zero data retention |
| **Reasoning** | Improved behavior on reasoning models (e.g. ~3% on SWE-bench) |

---

## 4. llm-connector Adaptation Strategy

### 4.1 Approach

**Option A: New Responses Protocol** (recommended)
- Add `OpenAIResponsesProtocol` implementing `Protocol`
- Add `OpenAIResponsesRequest` / `OpenAIResponsesResponse`
- Keep existing `OpenAIProtocol` (Chat Completions); users choose per use case

**Option B: Parameter Toggle**
- Switch endpoint in `OpenAIProtocol` via config
- Request/response structures differ significantly; mixed implementation is complex; not recommended

### 4.2 Scope

| Phase | Scope | Description |
|-------|-------|-------------|
| **Phase 1** | Basic text | `input` (string/messages), `instructions`, `output_text` |
| **Phase 2** | Multi-turn & state | `previous_response_id`, `store` |
| **Phase 3** | Items & tools | Parse `output` Items, function_call / function_call_output |
| **Phase 4** | Structured Outputs | Map `text.format` |
| **Phase 5** | Streaming | Responses streaming format parsing |

### 4.3 Type Design Draft

```rust
// New types
pub struct ResponsesRequest {
    pub model: String,
    pub input: ResponsesInput,      // String | Vec<Message> | Vec<Item>
    pub instructions: Option<String>,
    pub previous_response_id: Option<String>,
    pub store: Option<bool>,
    pub text: Option<TextConfig>,   // format, etc.
    pub tools: Option<Vec<ResponsesTool>>,
    // ...
}

pub enum ResponsesInput {
    String(String),
    Messages(Vec<Message>),
    Items(Vec<ResponseItem>),
}

pub struct ResponsesResponse {
    pub id: String,
    pub output: Vec<ResponseItem>,
    pub output_text: Option<String>,  // Convenience
    pub usage: Option<Usage>,
    // ...
}

pub enum ResponseItem {
    Message { role: String, content: Vec<ContentBlock> },
    FunctionCall { call_id: String, name: String, arguments: String },
    FunctionCallOutput { call_id: String, output: String },
    // ...
}
```

### 4.4 Mapping to Unified Types

- `ChatRequest` → `ResponsesRequest`: `messages` → `input` (or `instructions` + `input`)
- `ResponsesResponse` → `ChatResponse`: `output` Items → `choices[0].message`, `output_text` → `content`

Consider:
1. `ChatRequest` → `ResponsesRequest` conversion for gradual migration
2. `ResponsesResponse` → `ChatResponse` conversion for existing consumers

---

## 5. Implementation Plan

### 5.1 Milestones

| Milestone | Scope | Estimate |
|-----------|-------|----------|
| **M1** | Design `ResponsesRequest` / `ResponsesResponse` | 1–2 days |
| **M2** | Implement `OpenAIResponsesProtocol` (Phase 1) | 2–3 days |
| **M3** | Integration tests (text) | 1 day |
| **M4** | Phase 2 multi-turn & store | 1–2 days |
| **M5** | Phase 3 Items & tools | 2–3 days |
| **M6** | Docs and examples | 1 day |

### 5.2 Dependencies and Compatibility

- Requires `OPENAI_API_KEY` or Responses API–compatible proxy
- Fully compatible with Chat Completions; no breaking changes
- Optional feature: `openai-responses`, off by default to avoid impact on non-users

### 5.3 Acceptance Criteria

- Unit tests for `build_request` / `parse_response`
- Integration tests for `/v1/responses` text generation
- Docs on when to use Chat Completions vs Responses
- Example: `examples/openai_responses.rs`

---

## 6. References

- [Migrate to the Responses API](https://platform.openai.com/docs/guides/migrate-to-responses)
- [Responses API Reference](https://platform.openai.com/docs/api-reference/responses)
- [Function calling in Responses](https://developers.openai.com/api/docs/guides/function-calling)
- [Structured Outputs in Responses](https://developers.openai.com/api/docs/guides/structured-outputs)
