# Parameter Mapping

`llm-connector` provides a unified `ChatRequest` struct. However, different LLM providers often use varying parameter names for the same underlying concepts. The library automatically maps these standard parameters to the correct fields required by each provider's native API.

## Generation Parameters

This table shows how parameters in `ChatRequest` are mapped to the underlying provider JSON payload.

| `ChatRequest` Property | OpenAI / Compatible | Anthropic | Google Gemini (`generationConfig`) |
| :--- | :--- | :--- | :--- |
| `temperature` | `temperature` | `temperature` | `temperature` |
| `max_tokens` | `max_tokens` (or `max_completion_tokens`) | `max_tokens` | `maxOutputTokens` |
| `top_p` | `top_p` | `top_p` | `topP` |
| `presence_penalty` | `presence_penalty` | *Ignored* | `presencePenalty` |
| `frequency_penalty` | `frequency_penalty` | *Ignored* | `frequencyPenalty` |
| `stop` | `stop` | `stop_sequences` | `stopSequences` |

*Note: For providers using the `GenericProvider` / `OpenAIProtocol` (DeepSeek, Qwen, Moonshot, Zhipu, etc.), the mapping strictly follows the "OpenAI / Compatible" column.*

## Token Usage Mapping

When you receive a `ChatResponse` or consume a `StreamingResponse` stream, the `Usage` struct unifies the token consumption report.

| `Usage` Property | OpenAI / Compatible | Anthropic | Google Gemini (`usageMetadata`) |
| :--- | :--- | :--- | :--- |
| `prompt_tokens` | `prompt_tokens` | `input_tokens` | `promptTokenCount` |
| `completion_tokens` | `completion_tokens` | `output_tokens` | `candidatesTokenCount` |
| `total_tokens` | `total_tokens` | *(Computed)* | `totalTokenCount` |
| `prompt_tokens_details` | `prompt_tokens_details` | `cache_creation_input_tokens` / `cache_read_input_tokens` | *Mapped to internal representations* |
| `completion_tokens_details` | `completion_tokens_details` | *N/A* | *N/A* |

## Tool Calling / Function Calling

`llm-connector` natively uses the OpenAI tool calling format.

| `ChatRequest` Property | OpenAI / Compatible | Anthropic | Google Gemini (`tools`) |
| :--- | :--- | :--- | :--- |
| `tools` (Array of Functions)| `tools` | `tools` (mapped to `input_schema`) | `functionDeclarations` |
| `tool_choice` | `tool_choice` | `tool_choice` | `toolConfig` |

By maintaining this single, unified abstraction, `llm-connector` allows you to seamlessly switch between providers without altering the shape of your application's logic or prompt configuration.
