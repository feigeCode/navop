# Providers

`llm-connector` supports 12+ LLM providers with a unified interface.

Most networked constructors require an explicit endpoint input and do not hide default URLs.
For OpenAI-style providers that usually means `api_key + base_url`; `Ollama` only needs `base_url`, and `Azure OpenAI` uses `endpoint + api_version`.
Pass `LlmClient::builder()` if you need timeout or proxy configuration.

## Provider Overview

| Provider | Constructor | API Format |
|----------|-------------|------------|
| **OpenAI** | `LlmClient::openai(key, base_url)` | Native |
| **Azure OpenAI** | `LlmClient::azure_openai(key, endpoint, api_version)` | Native |
| **Anthropic Claude** | `LlmClient::anthropic(key, base_url)` | Native |
| **Google Gemini** | `LlmClient::google(key, base_url)` | Native |
| **Aliyun DashScope** | `LlmClient::aliyun(key, base_url)` | Native |
| **Zhipu GLM** | `LlmClient::zhipu(key, base_url)` | Native |
| **Zhipu (OpenAI compat)** | `LlmClient::zhipu_openai_compatible(key, base_url)` | OpenAI |
| **Tencent Hunyuan** | `LlmClient::tencent(id, key, base_url)` | Native V3 |
| **Volcengine** | `LlmClient::volcengine(key, base_url)` | OpenAI |
| **DeepSeek** | `LlmClient::deepseek(key, base_url)` | OpenAI |
| **Moonshot (Kimi)** | `LlmClient::moonshot(key, base_url)` | OpenAI |
| **Xiaomi MiMo** | `LlmClient::xiaomi(key, base_url)` | OpenAI |
| **Ollama** | `LlmClient::ollama(base_url)` | Native |
| **LongCat** | `LlmClient::longcat_anthropic(key, base_url)` | Anthropic |
| **Any OpenAI-compat** | `LlmClient::openai_compatible(key, base_url, name)` | OpenAI |

---

## OpenAI

```rust
use llm_connector::LlmClient;

let client = LlmClient::openai("sk-...", "https://api.openai.com/v1")?;
```

### Azure OpenAI
```rust
let client = LlmClient::azure_openai(
    "your-api-key",
    "https://your-resource.openai.azure.com",
    "2024-02-15-preview"
)?;
```

---

## Anthropic Claude

```rust
use llm_connector::LlmClient;

let client = LlmClient::anthropic("sk-ant-...", "https://api.anthropic.com")?;
```

### AWS Bedrock / Google Vertex AI
```rust
// AWS Bedrock
let client = LlmClient::anthropic_bedrock("us-east-1", "access_key", "secret_key")?;

// Google Vertex AI
let client = LlmClient::anthropic_vertex("project-id", "us-central1", "access-token")?;
```

---

## Aliyun DashScope (Qwen)

```rust
use llm_connector::LlmClient;

let client = LlmClient::aliyun("sk-...", "https://dashscope.aliyuncs.com")?;
```

### Variants
```rust
// International region
let client = LlmClient::aliyun_international("sk-...", "us-east-1")?;

// Private cloud
let client = LlmClient::aliyun_private("sk-...", "https://your-private.aliyun.com")?;
```

---

## Zhipu GLM

```rust
use llm_connector::LlmClient;

// Native SDK style
let client = LlmClient::zhipu("your-api-key", "https://open.bigmodel.cn")?;

// OpenAI Compatible Mode
let client = LlmClient::zhipu_openai_compatible("your-api-key", "https://open.bigmodel.cn")?;

// Enterprise endpoint
let client = LlmClient::zhipu_enterprise("your-api-key", "https://your-enterprise.bigmodel.cn")?;
```

---

## Google Gemini

```rust
use llm_connector::LlmClient;

let client = LlmClient::google(
    "your-api-key",
    "https://generativelanguage.googleapis.com/v1beta"
)?;
```

---

## Tencent Hunyuan

Native Tencent Cloud API v3 (`TC3-HMAC-SHA256`). Requires the `tencent` feature:

```toml
llm-connector = { version = "1.0.3", features = ["tencent"] }
```

```rust
use llm_connector::LlmClient;

let client = LlmClient::tencent(
    "AKID...",
    "SecretKey...",
    "https://hunyuan.tencentcloudapi.com"
)?;
```

---

## Volcengine

```rust
use llm_connector::LlmClient;

let client = LlmClient::volcengine(
    "your-api-key",
    "https://ark.cn-beijing.volces.com/api/v3"
)?;
```

---

## DeepSeek

```rust
use llm_connector::LlmClient;

let client = LlmClient::deepseek("sk-...", "https://api.deepseek.com")?;
```

---

## Moonshot (Kimi)

```rust
use llm_connector::LlmClient;

let client = LlmClient::moonshot("sk-...", "https://api.moonshot.cn/v1")?;
```

---

## Xiaomi MiMo

```rust
use llm_connector::LlmClient;

let client = LlmClient::xiaomi("your-api-key", "https://api.xiaomimimo.com/v1")?;
```

---

## Ollama

```rust
use llm_connector::LlmClient;

let client = LlmClient::ollama("http://localhost:11434")?;
```

---

## LongCat

LongCat supports both OpenAI and Anthropic wire formats:

```rust
use llm_connector::LlmClient;

// Anthropic format (Bearer auth)
let client = LlmClient::longcat_anthropic("ak_...", "https://api.longcat.chat/anthropic")?;

// OpenAI format
let client = LlmClient::openai_compatible("ak_...", "https://api.longcat.chat/openai", "longcat")?;
```

---

## Generic / Custom OpenAI-Compatible

```rust
use llm_connector::LlmClient;

let client = LlmClient::openai_compatible(
    "api-key",
    "https://api.example.com/v1",
    "my-provider"
)?;
```

---

## Builder Pattern (Timeout / Proxy)

```rust
use llm_connector::LlmClient;

let client = LlmClient::builder()
    .openai("sk-...")
    .base_url("https://api.openai.com/v1")
    .timeout(120)
    .proxy("http://proxy.example.com:8080")
    .build()?;
```

---

## Environment Variables

| Provider | Environment Variable |
|----------|---------------------|
| OpenAI | `OPENAI_API_KEY` |
| Anthropic | `ANTHROPIC_API_KEY` |
| Aliyun | `DASHSCOPE_API_KEY` |
| Zhipu | `ZHIPU_API_KEY` |
| DeepSeek | `DEEPSEEK_API_KEY` |
| Moonshot | `MOONSHOT_API_KEY` |
| Xiaomi | `XIAOMI_API_KEY` |
| Google | `GOOGLE_API_KEY` |
| Tencent | `TENCENT_SECRET_ID`, `TENCENT_SECRET_KEY` |
| Volcengine | `VOLCENGINE_API_KEY` |
