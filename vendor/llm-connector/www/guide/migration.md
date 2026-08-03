# Migration

## Migrating to v1.0.x

### All Constructors Require Explicit `base_url`

In v1.0.0 the API was redesigned so **every** provider constructor takes an explicit `base_url`.
This removes hidden defaults and makes multi-tenant / proxy routing obvious at the call site.

```rust
// Before (v0.x)
let client = LlmClient::openai("sk-...")?;
let client = LlmClient::ollama()?;
let client = LlmClient::openai_with_base_url("sk-...", "https://...")?;

// After (v1.x)
let client = LlmClient::openai("sk-...", "https://api.openai.com/v1")?;
let client = LlmClient::ollama("http://localhost:11434")?;
// openai_with_base_url is removed — just use openai(key, url)
```

All affected constructors:

| Provider | Old | New |
|----------|-----|-----|
| `openai` | `openai(key)` | `openai(key, base_url)` |
| `anthropic` | `anthropic(key)` | `anthropic(key, base_url)` |
| `aliyun` | `aliyun(key)` | `aliyun(key, base_url)` |
| `zhipu` | `zhipu(key)` | `zhipu(key, base_url)` |
| `ollama` | `ollama()` | `ollama(base_url)` |
| `deepseek` | `deepseek(key)` | `deepseek(key, base_url)` |
| `moonshot` | `moonshot(key)` | `moonshot(key, base_url)` |
| `volcengine` | `volcengine(key)` | `volcengine(key, base_url)` |
| `xiaomi` | `xiaomi(key)` | `xiaomi(key, base_url)` |
| `google` | `google(key)` | `google(key, base_url)` |
| `tencent` | `tencent(id, key)` | `tencent(id, key, base_url)` |

### Deserialize on Protocol Request Types (v1.0.3)

`OpenAIRequest`, `AnthropicRequest`, `GoogleRequest`, `OllamaChatRequest` and all their nested
structs now derive `Deserialize` in addition to `Serialize`. This enables **reverse proxy /
middleware / observability** use cases where incoming wire-format bodies need to be parsed.

No breaking change — purely additive.

---

## Migrating to v0.5.0

### Multi-Modal Content

v0.5.0 introduced native multi-modal support, changing the `Message.content` field.

**Breaking Change**:
- `Message.content` is now `Vec<MessageBlock>` internally.
- Use helper constructors instead of direct field access.

```rust
// Before
let msg = Message { role: Role::User, content: "hello".to_string(), .. };

// After
let msg = Message::user("hello");
// or
let msg = Message::text(Role::User, "hello");
```
