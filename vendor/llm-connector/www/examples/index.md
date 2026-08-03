# Examples

All examples live in [`examples/`](https://github.com/lipish/llm-connector/tree/main/examples).

## Setup

```bash
# Copy and fill in your API keys
cp .env.example .env

# Run any example
cargo run --example <name>
```

---

## OpenAI

**`examples/openai.rs`** — Chat · Streaming · Responses API · Embeddings

```bash
cargo run --example openai
```

Covers five scenarios in one shot:
1. Basic chat (`gpt-4o`)
2. Streaming (`with_stream(true)`) with real-time token output
3. Responses API (`invoke_responses`)
4. Responses streaming (`invoke_responses_stream`)
5. Embeddings (`text-embedding-3-small`) — returns vector size

```
OPENAI_API_KEY=sk-...
OPENAI_BASE_URL=https://api.openai.com/v1   # optional override
```

---

## Anthropic Claude

**`examples/anthropic.rs`** — System messages · Streaming

```bash
cargo run --example anthropic
```

1. Chat with a `system` message (persona: speaks like a pirate)
2. Streaming response for a long-form question

```
ANTHROPIC_API_KEY=sk-ant-...
ANTHROPIC_BASE_URL=https://api.anthropic.com   # optional
```

---

## Google Gemini

**`examples/google.rs`** — Chat · Streaming · Native Thinking

```bash
cargo run --example google
```

1. Basic chat with Gemini 1.5 Flash
2. Streaming chat
3. Reasoning / thinking mode (`with_enable_thinking(true)`) — exposes separate `reasoning_content`

**`examples/google_tools_thinking.rs`** — Tools + Thinking combined

```bash
cargo run --example google_tools_thinking
```

Full 3-step flow: tool call request → mock tool execution → tool result → final answer, all with `enable_thinking` active. Uses Gemini 3 Flash Preview / 2.0.

```
GOOGLE_API_KEY=...
GOOGLE_REGION=global   # or cn
GOOGLE_MODEL=gemini-3-flash-preview
```

**`examples/list_google_models.rs`** — List available models

```bash
cargo run --example list_google_models
```

---

## Aliyun DashScope (Qwen)

**`examples/aliyun.rs`** — Chat · Streaming · Reasoning

```bash
cargo run --example aliyun
```

1. Basic chat (`qwen-max`)
2. Streaming chat
3. Reasoning / thinking mode (`qwen-plus` + `with_enable_thinking(true)`)

```
ALIYUN_API_KEY=sk-...
ALIYUN_BASE_URL=https://dashscope.aliyuncs.com   # optional
```

---

## Zhipu GLM

**`examples/zhipu.rs`** — Multi-region · Chat · Streaming

```bash
cargo run --example zhipu
```

Supports domestic (`cn` / bigmodel.cn) and global (`global` / z.ai) endpoints via `ZHIPU_REGION`. Optional proxy via `ZHIPU_PROXY`.

```
ZHIPU_API_KEY=...
ZHIPU_REGION=cn           # or global
ZHIPU_MODEL=glm-4.5-flash
ZHIPU_PROXY=http://127.0.0.1:7890   # optional
```

**`examples/zhipu_tools.rs`** — Tool Calling (GLM-5, Global)

```bash
cargo run --example zhipu_tools
```

Full 2-step tool call flow with reasoning output before the tool call decision.

**`examples/zhipu_thinking.rs`** — Reasoning / Thinking (GLM-5)

```bash
cargo run --example zhipu_thinking
```

Uses `with_enable_thinking(true)` for step-by-step maths reasoning; displays `reasoning_content` separately from the final answer.

**`examples/zhipu_vision.rs`** — Image Understanding (GLM-4V)

```bash
cargo run --example zhipu_vision
```

Sends a URL image + text question using `MessageBlock::image_url(...)`. Works with `glm-4v` and `glm-5`.

---

## Moonshot (Kimi)

**`examples/moonshot.rs`** — Multi-region · Chat · Streaming

```bash
cargo run --example moonshot
```

Selects endpoint via `MOONSHOT_REGION` (`cn` → api.moonshot.cn, `global` → api.moonshot.ai).

```
MOONSHOT_API_KEY=sk-...
MOONSHOT_REGION=cn
MOONSHOT_MODEL=kimi-k2.5
```

**`examples/moonshot_tools.rs`** — Tool Calling

```bash
cargo run --example moonshot_tools
```

2-step weather tool flow: request tool → parse typed args → return result → final answer.

**`examples/moonshot_thinking.rs`** — Reasoning / Thinking

```bash
cargo run --example moonshot_thinking
```

Logic puzzle solved with `with_enable_thinking(true)`; `reasoning_any()` extracts the chain-of-thought across multiple field spellings (`reasoning_content`, `reasoning`, `thought`, `thinking`).

---

## DeepSeek

**`examples/deepseek.rs`** — Basic Chat

```bash
cargo run --example deepseek
```

Minimal example using `LlmClient::openai(key, base_url)` pointed at `api.deepseek.com`. DeepSeek uses the OpenAI wire format.

```
DEEPSEEK_API_KEY=sk-...
DEEPSEEK_BASE_URL=https://api.deepseek.com   # optional
```

---

## Minimax

**`examples/minimax.rs`** — Multi-region · Chat · Streaming

```bash
cargo run --example minimax
```

Uses `openai_compatible` builder, selects `global` endpoint by default. Minimax M2.5 via OpenAI-compatible API.

```
MINIMAX_API_KEY=...
MINIMAX_REGION=global
MINIMAX_MODEL=MiniMax-M2.5
```

---

## Tencent Hunyuan

**`examples/tencent.rs`** — Chat · Streaming (TC3-HMAC-SHA256)

```bash
cargo run --example tencent --features tencent
```

Requires the `tencent` Cargo feature. Native Tencent Cloud API v3 signing (`SecretId` + `SecretKey`).

```
TENCENT_SECRET_ID=AKID...
TENCENT_SECRET_KEY=...
TENCENT_BASE_URL=hunyuan.tencentcloudapi.com
```

---

## Ollama (Local Models)

**`examples/ollama.rs`** — Model listing · Chat · Streaming

```bash
cargo run --example ollama
```

No API key required. Connects to `http://localhost:11434` by default.

1. List locally pulled models
2. Chat with `llama3`
3. Streaming chat

Make sure Ollama is running and the model is pulled:
```bash
ollama pull llama3
```

---

## Cross-Provider Examples

**`examples/tool_calling.rs`** — Full 2-step Tool Call (OpenAI)

```bash
cargo run --example tool_calling
```

The reference tool-calling example. Defines a `get_current_weather` function, sends it via OpenAI `gpt-4o-mini`, executes the mock tool, and returns the result for a final answer.

**`examples/multi_modal.rs`** — Image Analysis + Embeddings (OpenAI)

```bash
cargo run --example multi_modal
```

Combines multi-modal streaming (image URL → `MessageBlock::image_url(...)`) with embeddings in a single run.

**`examples/check_reasoning_formats.rs`** — Reasoning Field Inspection

```bash
cargo run --example check_reasoning_formats
```

Inspects all reasoning-related field variants (`reasoning_content`, `reasoning`, `thought`, `thinking`) across multiple providers to verify extraction.

**`examples/real_world_connectivity_test.rs`** — Batch Connectivity Test

```bash
cargo run --example real_world_connectivity_test
```

Tests every configured provider in sequence and prints a pass/fail summary. Useful as a smoke test before deployment.

---

## Environment Variables Reference

| Provider | Variable(s) |
|----------|------------|
| OpenAI | `OPENAI_API_KEY`, `OPENAI_BASE_URL` |
| Anthropic | `ANTHROPIC_API_KEY`, `ANTHROPIC_BASE_URL` |
| Google | `GOOGLE_API_KEY`, `GOOGLE_REGION`, `GOOGLE_MODEL`, `GOOGLE_BASE_URL` |
| Aliyun | `ALIYUN_API_KEY`, `ALIYUN_BASE_URL` |
| Zhipu | `ZHIPU_API_KEY`, `ZHIPU_REGION`, `ZHIPU_MODEL`, `ZHIPU_PROXY` |
| Moonshot | `MOONSHOT_API_KEY`, `MOONSHOT_REGION`, `MOONSHOT_MODEL` |
| DeepSeek | `DEEPSEEK_API_KEY`, `DEEPSEEK_BASE_URL` |
| Minimax | `MINIMAX_API_KEY`, `MINIMAX_REGION`, `MINIMAX_MODEL` |
| Tencent | `TENCENT_SECRET_ID`, `TENCENT_SECRET_KEY`, `TENCENT_BASE_URL` |
| Ollama | _(none — defaults to localhost:11434)_ |
