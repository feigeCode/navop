# Anthropic extended thinking: signature preservation (llm-connector)

This document is the internal development spec for preserving Anthropic extended-thinking **signature** values in the shared `Message` / `MessageBlock` model and in Anthropic adapters. It aligns with the cross-repo design agreed with xrouter.

## Problem

Anthropic assistant messages may contain `content` blocks of type `thinking` with a server-issued **signature**. That value is required for correct multi-turn extended-thinking continuation when the next request echoes the prior assistant content. It cannot be minted by clients or gateways.

Prior to structured blocks, thinking was approximated with fields such as `Message.thinking`, which drops `signature` and block order relative to other blocks. Also, Anthropic **`build_request`** for assistant messages serializes `Message.content` (and appends `tool_use` from `tool_calls`); it did **not** serialize `Message.thinking` into the Anthropic `content` array, so a `signature` sibling of `Message.thinking` could not produce a valid thinking block.

## Model

- **`MessageBlock::Thinking { thinking, signature }`**
  - `signature` is `Option<String>`. `None` means unsigned / degraded (e.g. OpenAI-style reasoning mapped for shape only). **Never** synthesize a fake Anthropic signature.
  - Serde JSON matches Anthropic: `{"type":"thinking","thinking":"...","signature":"..."}` with `signature` omitted when `None`.
- **`Message.thinking`**
  - Retained as an optional **compatibility aggregate** (concatenation of thinking block bodies for parsers that fill it). When both structured blocks and `Message.thinking` are present, **structured `content` blocks are authoritative** for serialization to Anthropic.

## Anthropic non-streaming

- **`parse_response`**: For each `content[]` with `type == "thinking"`, push `MessageBlock::Thinking` and merge text into `Message.thinking` for backward compatibility.
- **`build_request`**: Assistant `content` is still `serde_json::to_value(&msg.content)`; `Thinking` blocks round-trip to Anthropic JSON automatically. No fake signatures for legacy reasoning-only fields.

## Ordering vs `tool_use`

`tool_calls` are still appended after serialized `content` blocks in `build_request`. Interleaving of `thinking`, `text`, and **`tool_use`** in the exact Anthropic order is **not** fully modeled unless `tool_use` is also represented as ordered blocks or metadata is added. This limitation is documented in tests / CHANGELOG until addressed.

## Streaming (what llm-connector does)

- **`interpret_anthropic_event`** maps `content_block_delta` events: `thinking_delta` → `Delta.thinking`, `signature_delta` → `Delta.thinking_signature` (plus existing `text` and tool `partial_json` handling).
- The library does **not** assemble a final assistant `Message` with ordered `MessageBlock::Thinking` / `Text` from a stream. It only emits **per-event** `StreamingResponse` chunks.

## Integrator responsibilities (e.g. xrouter)

Gateways that need **streaming** assistant turns to be **replayable** on the next Anthropic request with a real **thinking `signature`** must implement aggregation **in the integration layer** (this crate does not do it yet). Below is an explicit contract so xrouter (or any host) can implement it without guessing.

### 1. You must retain Anthropic `content_block` index

Anthropic `content_block_delta` carries an **`index`** field: which assistant `content` block is being updated. Many deltas for the same logical block share the same `index` until that block ends.

In `interpret_anthropic_event`, for text / thinking / signature deltas, the mapped `StreamingResponse` uses `StreamingChoice { index: 0, ... }` for the OpenAI-style **choice** slot; that is **not** the Anthropic block index. Tool `partial_json` paths **do** copy the event `index` onto `ToolCall.index`.

**Therefore:** either parse **raw** Anthropic SSE JSON and read `index` on every `content_block_delta`, or wrap the interpreter so every emitted chunk carries the source event's `index` (and ideally `delta.type`). Without the Anthropic block `index`, you cannot reconstruct interleaved blocks (e.g. text block 0, thinking block 1, text block 2).

### 2. Stateful aggregation per block index

Recommended state machine (aligned with Anthropic streaming):

1. **`content_block_start`**: record `content_block.type` for this `index` (`thinking`, `text`, `tool_use`, …).
2. **`content_block_delta`**:
   - `thinking_delta`: append `delta.thinking` to a string buffer for this `index`.
   - `signature_delta`: store `delta.signature` as the **final** signature for the thinking block at this `index` (opaque; do not rewrite).
   - `text_delta`: append `delta.text` to a text buffer for this `index`.
   - `input_json_delta` / `partial_json` (tool): accumulate tool JSON per `index` as you already do for tools.
3. **`content_block_stop`** (and/or `message_stop`, per Anthropic rules): **finalize** slot `index`:
   - `thinking` → `MessageBlock::Thinking { thinking: concatenated_deltas, signature: Some(...) }` if the API delivered a signature; otherwise `None` only on **degraded** paths (never a fake "continuation" signature).
   - `text` → `MessageBlock::Text { ... }`.
   - `tool_use` → map to your internal model; see ordering caveat below.
4. After the stream finishes: build assistant **`Message.content`** as blocks sorted by **`index` ascending** (0, 1, 2, …).

`Delta.thinking` and `Delta.thinking_signature` on each chunk are **pieces** of this picture; they are not themselves a full block until merged using the steps above.

### 3. Round-trip through `build_request` vs true Anthropic order

When the stored assistant `Message` is later sent via **`AnthropicProtocol::build_request`**:

- All `MessageBlock` entries in `content` keep their **relative** order among text/thinking/image/etc.
- **`Message.tool_calls` are still appended after** the serialized `content` array.

So a **native** Anthropic order like `thinking → tool_use → text` cannot always be reproduced exactly from the unified model today. **Integrator mitigations:**

- If your traffic is "tools always after" text/thinking, you may be fine.
- For strict Anthropic ↔ Anthropic replay, you may need to keep **provider-native** assistant JSON for that leg, or extend llm-connector later with ordered `tool_use` blocks / explicit ordering metadata.

### 4. Non-streaming vs streaming

- **Non-streaming:** `parse_response` already yields `MessageBlock::Thinking` with `signature`; you can persist that `Message` (or equivalent JSON) for the next turn **without** SSE aggregation.
- **Streaming:** a correct `signature` in history **requires** sections 1–2; section 3 still applies when you rebuild the Anthropic request through this library.

## OpenAI-compatible export

`protocols/common/request.rs` maps `Thinking` blocks out of the serialized `content` array and merges block bodies into **`reasoning_content`** on the wire object when building OpenAI-style messages, so providers do not see unknown `type: "thinking"` parts. Downgrade (text-only) paths concatenate thinking into `reasoning_content` and keep visible text from `Text` blocks only.

## Other providers

- **Google**: Thinking blocks map to plain text parts (unsigned degradation).
- **Ollama / Tencent** (text-only): Prepend concatenated thinking block text ahead of `content_as_text()` so bodies are not silently dropped.

## Acceptance

- Non-streaming: Anthropic JSON with `thinking` + `signature` + `text` parses into `MessageBlock::Thinking` + text; `build_request` reproduces the same `thinking` and `signature`.
- Streaming: `thinking_delta` / `signature_delta` yield `Delta` fields; **final** signed blocks are the integrator's responsibility (see above).
- No placeholder Anthropic signatures in llm-connector core types or Anthropic adapter request building.

## Related

External integration and xrouter emulator follow-up are documented in the consuming repo (`thinking-signature.md` there).
