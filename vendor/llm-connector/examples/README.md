# LLM Connector Examples

This directory contains curated usage examples for the `llm-connector` library.

## 📚 Example List

### Basic Examples (7)

| Example File | Description | Run Command |
|---------|------|----------|
| `openai_basic.rs` | OpenAI basic chat example | `cargo run --example openai_basic` |
| `aliyun_basic.rs` | Aliyun Qwen basic example | `cargo run --example aliyun_basic` |
| `zhipu_basic.rs` | Zhipu GLM basic example | `cargo run --example zhipu_basic` |
| `tencent_basic.rs` | Tencent Hunyuan basic example | `cargo run --example tencent_basic` |
| `ollama_basic.rs` | Ollama local model basic example | `cargo run --example ollama_basic` |
| `anthropic_streaming.rs` | Anthropic streaming response example | `cargo run --example anthropic_streaming --features streaming` |
| `volcengine_streaming.rs` | Volcengine streaming example (reasoning models supported) | `cargo run --example volcengine_streaming --features streaming -- <api-key> <endpoint>` |

### New Features (0.6.1+)

| Example File | Description | Run Command |
|---------|------|----------|
| `reasoning_thinking.rs` | Reasoning/Thinking model example (Claude 3.7 / o1) | `cargo run --example reasoning_thinking` |
| `file_upload.rs` | File upload helper example | `cargo run --example file_upload` |
| `dynamic_routing.rs` | Dynamic service resolution example | `cargo run --example dynamic_routing` |

### Special Features (4)

| Example File | Description | Run Command |
|---------|------|----------|
| `multimodal_basic.rs` | Multi-modal content example (text + image) | `cargo run --example multimodal_basic` |
| `ollama_model_management.rs` | Ollama model management (CRUD) | `cargo run --example ollama_model_management` |
| `ollama_streaming.rs` | Ollama streaming response example | `cargo run --example ollama_streaming --features streaming` |
| `aliyun_thinking.rs` | Aliyun thinking feature example | `cargo run --example aliyun_thinking` |

### Tool Calling (2)

| Example File | Description | Run Command |
|---------|------|----------|
| `zhipu_tools.rs` | Zhipu GLM tool calling example for cn/global endpoints | `cargo run --example zhipu_tools` |
| `zhipu_multiround_tools.rs` | Zhipu GLM multi-round tool calling example | `cargo run --example zhipu_multiround_tools` |

## 🔧 Environment Variables

### OpenAI
```bash
export OPENAI_API_KEY="your-openai-api-key"
```

### Aliyun DashScope
```bash
export DASHSCOPE_API_KEY="your-dashscope-api-key"
```

### Zhipu GLM
```bash
export ZHIPU_API_KEY="your-zhipu-api-key"
export ZHIPU_REGION="global"   # or cn
export ZHIPU_MODEL="glm-5"
```

### Moonshot (Kimi)
```bash
export MOONSHOT_API_KEY="your-moonshot-api-key"
export MOONSHOT_REGION="cn"   # or global
export MOONSHOT_MODEL="kimi-k2.5"
```

Note: Moonshot cn/global keys may differ. Use a key that is valid for the selected region.

### Tencent Hunyuan
```bash
export TENCENT_API_KEY="your-tencent-api-key"
```

### Anthropic
```bash
export ANTHROPIC_API_KEY="your-anthropic-api-key"
```

### Ollama
```bash
# Ollama runs on localhost:11434 by default; no API key required
# Optional: specify a model
export OLLAMA_MODEL="llama2"
```

### Volcengine
```bash
export VOLCENGINE_API_KEY="your-volcengine-api-key"
export VOLCENGINE_ENDPOINT="ep-20250118155555-xxxxx"  # Reasoning endpoint ID
```

## 📋 Features

### Supported Providers

- **OpenAI** - GPT models
- **Aliyun DashScope** - Qwen models
- **Zhipu GLM** - GLM models
- **Tencent Hunyuan** - Hunyuan models
- **Anthropic** - Claude models
- **Ollama** - local open-source models
- **Volcengine** - Doubao models (including reasoning models such as Doubao-Seed-Code)

### Core Capabilities

- ✅ Unified chat API
- ✅ Streaming response support
- ✅ Multi-modal content (text + image)
- ✅ Tool calling (Function Calling)
- ✅ Model listing
- ✅ Token usage statistics
- ✅ Error handling and retries

## 🎯 Quick Start

1. **Start with a basic example**:
   ```bash
   cargo run --example ollama_basic
   ```

2. **Try multi-modal content**:
   ```bash
   cargo run --example multimodal_basic
   ```

3. **Try tool calling**:
   ```bash
   cargo run --example zhipu_tools
   ```

4. **Run provider examples without local proxy interference**:
   ```bash
   EXAMPLE_NAME=zhipu_tools PROVIDER=zhipu REGION=global scripts/run_provider_example_no_proxy.sh
   ```

5. **Run Moonshot examples without local proxy interference**:
   ```bash
   EXAMPLE_NAME=moonshot_tools PROVIDER=moonshot REGION=cn scripts/run_provider_example_no_proxy.sh
   ```

## 💡 Notes

- Most examples require the corresponding API key
- Ollama examples require a local Ollama service
- Streaming examples require enabling the `streaming` feature
- Multi-modal examples require models with vision support (e.g., gpt-4o, claude-3-5-sonnet)
- For Zhipu real-world verification, prefer running without local proxy interference
- For Moonshot real-world verification, prefer running without local proxy interference

## 🔗 Links

- [Project Homepage](https://github.com/lipish/llm-connector)
- [API Docs](https://docs.rs/llm-connector)
- [Crates.io](https://crates.io/crates/llm-connector)
