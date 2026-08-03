# Protocol V2: Stage-based Decomposition & Abstraction

## Overview

The current `Protocol` trait is a monolithic abstraction where each implementation handles the entire lifecycle of a request and response. To improve maintainability and support diverse protocols (OpenAI, Anthropic, Gemini, etc.), we will decompose the process into distinct, reusable stages.

## Protocol Lifecycle Stages

### 1. Schema Mapping (Request Assembly)
Convert the unified `ChatRequest` into a protocol-specific payload.

- **OpenAI Assembler**: Standard `content` field handling (String/Array).
- **Anthropic Assembler**: Moves `system` messages to a top-level field, requires `messages` to start with a user role.
- **Gemini Assembler**: Maps to `contents` array with `parts`.

### 2. Transport & Auth (Metadata Management)
Define endpoints, headers, and authentication strategies.

- **Bearer Auth**: Standard `Authorization: Bearer <key>`.
- **API Key Header**: `x-api-key: <key>`.
- **Signature V4**: AWS style complex signing (future).

### 3. Stream Interpretation (Real-time Parsing)
Transform raw bytes/SSE chunks into unified `ChatResponse` chunks.

- **SSE Interpreter**: Standard `data: ` line parsing.
- **Anthropic Streamer**: Handles specific event types (`message_start`, `content_block_delta`).
- **Raw JSON Streamer**: Handles direct JSON chunks (Ollama/Gemini).

### 4. Response Normalization (Post-processing)
Map the final JSON response back to `ChatResponse`.

- **OpenAI Mapper**: Standard `choices` indexing.
- **Anthropic Mapper**: Extracts from `content` array.

## Stage Refinement Requirements

The four stages remain a useful top-level model, but they are still too coarse for providers that are mostly compatible with a standard protocol while keeping provider-specific differences in endpoint rules, capability support, or response quirks. The next iteration should preserve the four stages while refining each stage into smaller, explicit responsibilities.

### 1. Schema Mapping (Request Assembly) Refinement

This stage should no longer be implemented as a single provider-specific `build_request()` function that mixes schema conversion, capability checks, and provider extensions.

Required sub-responsibilities:

- **Message Mapper**: Converts unified messages into protocol message arrays or blocks.
- **Tool Mapper**: Converts tool definitions and `tool_choice` into protocol-specific shape.
- **Response Format Mapper**: Converts structured output and `response_format` fields.
- **Capability Downgrade Strategy**: Applies provider/model capability checks before final request assembly, such as downgrading content parts to plain text.
- **Provider Extension Field Mapper**: Injects provider-specific request fields without polluting common mappers.

Current design needs addressed by this refinement:

- Avoid string-based provider/model capability checks being scattered inside common OpenAI-compatible builders.
- Separate standard request mapping from provider-specific downgrade logic.
- Allow providers like Moonshot, Minimax-like OpenAI-compatible variants, and future Zhipu modes to share most of the request assembly path while overriding only capability or extension rules.

### 2. Transport & Auth (Metadata Management) Refinement

This stage should distinguish request routing and metadata concerns instead of treating them as a single provider responsibility.

Required sub-responsibilities:

- **Endpoint Resolver**: Resolves the final request URL from `base_url`, API mode, operation type, and optionally model.
- **Auth Strategy**: Encapsulates Bearer auth, API key headers, query auth, or signature-based auth.
- **Header Policy**: Adds provider-specific fixed headers and compatibility headers.
- **Request Metadata Policy**: Handles query parameters, API versions, region/deployment metadata, and similar transport metadata.

Current design needs addressed by this refinement:

- Keep endpoint-specific behavior such as Zhipu path resolution out of request mappers.
- Avoid coupling auth rules with provider parsing logic.
- Make mode-specific differences show up first in transport metadata, where they usually belong.

### 3. Stream Interpretation (Real-time Parsing) Refinement

This stage should be split further so that streaming differences are not represented only as a few large parser modes.

Required sub-responsibilities:

- **Frame Decoder**: Splits raw bytes into SSE frames, JSON lines, or other chunk units.
- **Event Classifier**: Identifies content deltas, tool deltas, reasoning deltas, completion markers, and error events.
- **Delta Mapper**: Maps provider-specific streaming payloads into unified streaming chunks.
- **Stream Finalizer**: Handles done markers, final usage, and end-of-stream cleanup.

Current design needs addressed by this refinement:

- Distinguish framing from semantic event interpretation.
- Support providers that are "OpenAI-like" at the transport level but still differ in delta structure.
- Make reasoning and tool-call streaming normalization explicit rather than incidental.

### 4. Response Normalization (Post-processing) Refinement

This stage should distinguish raw parsing from semantic normalization and provider-specific post-processing.

Required sub-responsibilities:

- **Raw Response Parser**: Deserializes raw JSON into protocol-family-specific intermediate structs.
- **Choice / Message Mapper**: Maps raw choices and messages into unified `ChatResponse` choices.
- **Usage Mapper**: Normalizes usage blocks across providers and response wrappers.
- **Reasoning Extractor**: Extracts reasoning from standard fields or provider-specific encodings such as embedded tags.
- **Tool Call Mapper**: Normalizes tool call structures into unified tool call objects.
- **Provider Post-Processor**: Handles the remaining true provider-specific quirks after common normalization.

Current design needs addressed by this refinement:

- Prevent raw JSON parsing from becoming a catch-all for provider-specific cleanup.
- Turn ad-hoc reasoning extraction into an explicit reusable concern.
- Keep provider-specific post-processing as the last and smallest customization point.

## Design Principles For Current Migration

To make this refinement practical for the current codebase, the following migration rules should be applied:

- **Keep the four stages as the orchestration model** rather than replacing them with a larger number of top-level phases.
- **Move common logic into reusable stage components** and keep provider adapters thin.
- **Model provider differences as strategies, capabilities, and mappers**, not as increasingly thick `Protocol` implementations.
- **Prefer protocol-family reuse first**, especially for OpenAI-compatible providers.
- **Reserve provider-specific code for endpoint/auth/error/quirk handling only**.

Applied to current providers, this means:

- **Zhipu** should keep a provider adapter for endpoint/auth/error behavior and API mode selection, while reusing shared request, stream, and response components wherever possible.
- **Moonshot** should continue to live under OpenAI-compatible abstractions, with capability downgrade handled explicitly instead of by scattered model-name checks.
- **Minimax-like compatibility behavior** should be represented as capability and post-processing strategies instead of growing a fully separate provider stack unless a truly distinct native protocol appears.

## Implementation Plan

### Phase 1: Directory Reorganization
Move from a flat `protocols/` structure to a component-based structure.

```text
src/protocols/
├── common/              # Shared logic (traits, auth, base types)
├── openai/              # OpenAI implementation
├── anthropic/           # Anthropic implementation
└── ...
```

### Phase 2: Trait Decomposition
Introduce granular traits if necessary, or at least structure the modules to expose these stages as decoupled functions.

### Phase 3: Migration
Migrate existing protocols (Aliyun, Zhipu, etc.) to use the `common` OpenAI components.
