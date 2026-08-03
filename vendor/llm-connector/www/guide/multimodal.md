# Multi-modal

`llm-connector` provides native support for multi-modal content (text + images + documents).

## Image from URL

```rust
use llm_connector::{LlmClient, types::{ChatRequest, Message, MessageBlock}};

let client = LlmClient::openai("sk-...", "https://api.openai.com/v1")?;

let request = ChatRequest::new("gpt-4o")
    .add_message(Message::new(
        llm_connector::types::Role::User,
        vec![
            MessageBlock::text("What's in this image?"),
            MessageBlock::image_url("https://example.com/image.jpg"),
        ],
    ));

let response = client.chat(&request).await?;
println!("Response: {}", response.content);
```

## Image from Base64

```rust
let base64_data = "iVBORw0KGgoAAAANSUhEUgAAAAUA...";

let request = ChatRequest::new("gpt-4o")
    .add_message(Message::new(
        llm_connector::types::Role::User,
        vec![
            MessageBlock::text("Analyze this image"),
            MessageBlock::image_base64("image/jpeg", base64_data),
        ],
    ));
```

## Image from File

```rust
use llm_connector::types::MessageBlock;

let block = MessageBlock::image_file("path/to/image.jpg").await?;
```

## Anthropic-Style Image (URL with detail)

```rust
let block = MessageBlock::image_url_with_detail(
    "https://example.com/image.jpg",
    "high"
);
```

## Provider Support

| Provider | Text | Images | Documents |
|----------|------|--------|-----------|
| OpenAI | ✅ | ✅ | ❌ |
| Anthropic | ✅ | ✅ | ✅ |
| Google Gemini | ✅ | ✅ | ✅ |
| Aliyun | ✅ | ✅ | ❌ |
| Zhipu | ✅ | ✅ | ❌ |
| Other | ✅ | ❌ | ❌ |

## Examples

```bash
cargo run --example multi_modal    # Basic image understanding
cargo run --example zhipu_vision   # Zhipu image analysis
```
