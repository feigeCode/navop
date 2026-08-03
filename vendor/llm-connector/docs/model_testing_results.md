# Model Testing Results & Characteristics

This document details the testing results and specific characteristics for various LLM providers integrated into `llm-connector`.

## Global Providers Comparison

All tests were performed using global endpoints.

| Provider | Model | Connectivity | Tool Calling | Reasoning (Thinking) | Reasoning Format |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **Zhipu (Z.ai)** | `glm-5` | ✅ Stable | ✅ Success | ✅ Success | Dedicated `reasoning_content` field |
| **Moonshot** | `kimi-k2.5` | ✅ Stable | ✅ Success | ✅ Success | Dedicated `reasoning_content` field |
| **Minimax** | `MiniMax-M2.5` | ✅ Stable | — | ✅ Success | Inline `<think>` tags (Auto-extracted) |
| **Gemini 2.x** | `gemini-2.5-flash` | ✅ Stable | ✅ Success | ✅ Success | `thought: true` part |
| **Gemini 3.x** | `gemini-3-flash-preview` | ✅ Stable | ✅ Success | ✅ Success | `thought: true` + mandatory `thoughtSignature` |

---

### 1. Zhipu GLM-5
- **Endpoint**: `https://api.z.ai/api/paas/v4`
- **Characteristics**:
    - High-quality reasoning; full OpenAI compatibility for tools and chat.
    - Reasoning is clearly separated into the dedicated `reasoning_content` field.

### 2. Moonshot (Kimi)
- **Endpoint**: `https://api.moonshot.ai/v1`
- **Characteristics**:
    - `kimi-k2.5` demonstrates strong reasoning capabilities.
    - **Note**: `reasoning_content` must be preserved in assistant messages during multi-turn tool calling to avoid 400 errors. `llm-connector` handles this automatically.

### 3. Minimax
- **Endpoint**: `https://api.minimax.io/v1`
- **Characteristics**:
    - `MiniMax-M2.5` embeds reasoning within `<think>...<think>` tags in the main content stream.
    - **Auto-Extraction**: `llm-connector` automatically moves these to `reasoning_content` and cleans `content`.

### 4. Google Gemini
- **Endpoint**: `https://generativelanguage.googleapis.com/v1beta`
- **Gemini 2.x** (`gemini-2.5-flash`, `gemini-2.0-flash`):
    - Full support for **Streaming, Tools, and Thinking**.
    - Thoughts returned as parts with `thought: true`, mapped to `reasoning_content`.
    - Function responses require the `user` role; `llm-connector` handles the mapping.
- **Gemini 3.x** (`gemini-3-flash-preview`, `gemini-3.1-pro-preview`):
    - Introduces a **mandatory `thoughtSignature`** field (sibling of `functionCall` in the response payload).
    - This signature **must be echoed back** in the conversation history for subsequent tool calls; omitting it causes a 400 error.
    - `llm-connector` automatically captures and replays `thoughtSignature` across turns.

---

## Implementation Details: Reasoning Extraction

To ensure a unified interface, `llm-connector` implements a post-processor in the `OpenAICompatible` protocol that:
1. Checks for dedicated reasoning fields (`reasoning_content`, `reasoning`, `thought`, `thinking`).
2. If absent, scans for `<think>` tags in the main text content.
3. Synchronizes all variants into `choice.message.reasoning_content`.
4. Trims the `<think>` block from `choice.message.content` to keep the final answer clean.

---
*Last Updated: 2026-03-03*

