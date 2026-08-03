use dotenvy::dotenv;
use llm_connector::{
    LlmClient, Message,
    types::{Tool, ToolChoice},
};
use serde_json::json;
use std::env;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenv().ok();

    let openai_url =
        env::var("OPENAI_BASE_URL").unwrap_or_else(|_| "https://api.openai.com/v1".to_string());
    let openai_key = env::var("OPENAI_API_KEY").unwrap_or_default();

    println!("--- Testing OpenAI (Tool Calling) ---");
    if let Err(e) = test_openai(&openai_url, &openai_key).await {
        println!("❌ OpenAI Error: {:?}", e);
    }

    let anthropic_url =
        env::var("ANTHROPIC_BASE_URL").unwrap_or_else(|_| "https://api.anthropic.com".to_string());
    let anthropic_key = env::var("ANTHROPIC_API_KEY").unwrap_or_default();

    println!("\n--- Testing Anthropic (Thinking) ---");
    if let Err(e) = test_anthropic(&anthropic_url, &anthropic_key).await {
        println!("❌ Anthropic Error: {:?}", e);
    }

    if let Ok(zhipu_key) = env::var("ZHIPU_API_KEY")
        && !zhipu_key.contains("your_")
    {
        println!("\n--- Testing Zhipu Native ---");
        if let Err(e) = test_zhipu(&zhipu_key).await {
            println!("❌ Zhipu Error: {:?}", e);
        }
    }

    if let Ok(aliyun_key) = env::var("ALIYUN_API_KEY")
        && !aliyun_key.contains("your_")
    {
        let aliyun_url = env::var("ALIYUN_BASE_URL")
            .unwrap_or_else(|_| "https://dashscope.aliyuncs.com".to_string());
        println!("\n--- Testing Aliyun Native ---");
        if let Err(e) = test_aliyun(&aliyun_key, &aliyun_url).await {
            println!("❌ Aliyun Error: {:?}", e);
        }
    }

    if let Ok(tencent_id) = env::var("TENCENT_SECRET_ID")
        && let Ok(tencent_key) = env::var("TENCENT_SECRET_KEY")
        && !tencent_id.contains("your_")
    {
        let tencent_url = env::var("TENCENT_BASE_URL")
            .unwrap_or_else(|_| "hunyuan.tencentcloudapi.com".to_string());
        println!("\n--- Testing Tencent Native ---");
        if let Err(e) = test_tencent(&tencent_id, &tencent_key, &tencent_url).await {
            println!("❌ Tencent Error: {:?}", e);
        }
    }

    // DeepSeek via OpenAI compatible client
    // DeepSeek via OpenAI compatible client
    if let Ok(deepseek_key) = env::var("DEEPSEEK_API_KEY") {
        let deepseek_url = env::var("DEEPSEEK_BASE_URL")
            .unwrap_or_else(|_| "https://api.deepseek.com".to_string());
        println!("\n--- Testing DeepSeek (Reasoning) ---");
        if let Err(e) = test_reasoning(&deepseek_url, &deepseek_key, "deepseek-reasoner").await {
            println!("❌ DeepSeek Error: {:?}", e);
        }
    }

    // MiniMax via OpenAI compatible client
    if let Ok(minimax_key) = env::var("MINIMAX_API_KEY") {
        let minimax_url = env::var("MINIMAX_BASE_URL")
            .unwrap_or_else(|_| "https://api.minimax.io/v1".to_string());
        println!("\n--- Testing MiniMax (Reasoning Extraction) ---");
        if let Err(e) = test_openai_compatible(&minimax_url, &minimax_key, "MiniMax-M2.5").await {
            println!("❌ MiniMax Error: {:?}", e);
        }
    }

    // Moonshot
    if let Ok(moonshot_key) = env::var("MOONSHOT_API_KEY") {
        let moonshot_url = env::var("MOONSHOT_BASE_URL")
            .unwrap_or_else(|_| "https://api.moonshot.ai/v1".to_string());
        println!("\n--- Testing Moonshot (Tool Calling) ---");
        if let Err(e) = test_tool_calling(&moonshot_url, &moonshot_key, "kimi-k2.5").await {
            println!("❌ Moonshot Error: {:?}", e);
        }
    }

    Ok(())
}

async fn test_openai(url: &str, key: &str) -> Result<(), Box<dyn std::error::Error>> {
    let client = LlmClient::openai(key, url)?;
    let weather_tool = Tool::function(
        "get_weather",
        Some("Get current weather".to_string()),
        json!({
            "type": "object",
            "properties": { "location": { "type": "string" } },
            "required": ["location"]
        }),
    );
    let request = llm_connector::ChatRequest::new("gpt-5.2-2025-12-11")
        .add_message(Message::user("Hello, what's the weather in Seattle?"))
        .with_tools(vec![weather_tool])
        .with_tool_choice(ToolChoice::Mode("auto".to_string()));

    let response = client.chat(&request).await?;
    if let Some(choice) = response.choices.first() {
        if let Some(tool_calls) = &choice.message.tool_calls {
            println!(
                "🛠️ OpenAI requested {} tool call(s). Tool Name: {}",
                tool_calls.len(),
                tool_calls[0].function.name
            );
        } else {
            println!("Response: {}", response.content);
        }
    }
    Ok(())
}

async fn test_anthropic(url: &str, key: &str) -> Result<(), Box<dyn std::error::Error>> {
    let client = LlmClient::openai(key, url)?;
    let request = llm_connector::ChatRequest::new("claude-opus-4-5-20251101-thinking")
        .add_message(Message::user(
            "Which is heavier, 1kg of feathers or 1kg of steel? Please think out loud.",
        ))
        .with_enable_thinking(true);

    let response = client.chat(&request).await?;
    if let Some(reasoning) = response.reasoning_content {
        println!("🧠 Anthropic Thinking:\n{}\n", reasoning);
    }
    println!("Response: {}", response.content);
    Ok(())
}

async fn test_zhipu(key: &str) -> Result<(), Box<dyn std::error::Error>> {
    let url = env::var("ZHIPU_BASE_URL")
        .unwrap_or_else(|_| "https://open.bigmodel.cn/api/paas/v4".to_string());
    let client = LlmClient::zhipu(key, &url)?;
    let request = llm_connector::ChatRequest::new("glm-4.5-flash")
        .add_message(Message::user("Hello Zhipu, are you there?"));

    let response = client.chat(&request).await?;
    println!("Response: {}", response.content);
    Ok(())
}

async fn test_aliyun(key: &str, url: &str) -> Result<(), Box<dyn std::error::Error>> {
    let client = LlmClient::aliyun(key, url)?;
    let request = llm_connector::ChatRequest::new("qwen-max")
        .add_message(Message::user("Hello Qwen, say something nice."));

    let response = client.chat(&request).await?;
    println!("Response: {}", response.content);
    Ok(())
}

async fn test_tencent(id: &str, key: &str, url: &str) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(feature = "tencent")]
    {
        let client = LlmClient::tencent(id, key, url)?;
        let request = llm_connector::ChatRequest::new("hunyuan-lite").add_message(Message::user(
            "Hello Tencent, describe the city of Shenzhen.",
        ));

        let response = client.chat(&request).await?;
        println!("Response: {}", response.content);
    }
    #[cfg(not(feature = "tencent"))]
    {
        let _ = (id, key, url);
        println!("Skipping Tencent test (feature not enabled)");
    }
    Ok(())
}

async fn test_openai_compatible(
    url: &str,
    key: &str,
    model: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let client = LlmClient::openai(key, url)?;
    let request = llm_connector::ChatRequest::new(model)
        .add_message(Message::user("Please think clearly: what is 25 * 4?"));

    let response = client.chat(&request).await?;
    if let Some(reasoning) = response.reasoning_content {
        println!("{}: 🧠 Thinking:\n{}\n", model, reasoning);
    }
    println!("{}: Response: {}", model, response.content);
    Ok(())
}

async fn test_reasoning(
    url: &str,
    key: &str,
    model: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let client = LlmClient::openai(key, url)?;
    let request = llm_connector::ChatRequest::new(model)
        .add_message(Message::user(
            "Think about this logically: Is 9.11 larger than 9.9?",
        ))
        .with_enable_thinking(true);

    let response = client.chat(&request).await?;
    if let Some(reasoning) = response.reasoning_content {
        println!("{}: 🧠 Reasoning:\n{}\n", model, reasoning);
    } else {
        println!("{}: ⚠️ No reasoning content returned", model);
    }
    println!("{}: Response: {}", model, response.content);
    Ok(())
}

async fn test_tool_calling(
    url: &str,
    key: &str,
    model: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let client = LlmClient::openai(key, url)?;
    let weather_tool = Tool::function(
        "get_weather",
        Some("Get current weather".to_string()),
        json!({
            "type": "object",
            "properties": {
                "location": { "type": "string" }
            },
            "required": ["location"]
        }),
    );

    let request = llm_connector::ChatRequest::new(model)
        .add_message(Message::user("What's the weather in Seattle?"))
        .with_tools(vec![weather_tool])
        .with_tool_choice(ToolChoice::Mode("auto".to_string()));

    let response = client.chat(&request).await?;
    if let Some(choice) = response.choices.first() {
        if let Some(tool_calls) = &choice.message.tool_calls {
            println!(
                "{}: 🛠️ AI requested {} tool call(s). Tool Name: {}",
                model,
                tool_calls.len(),
                tool_calls[0].function.name
            );
        } else {
            println!(
                "{}: ⚠️ AI didn't use tools. Response: {}",
                model, response.content
            );
        }
    }
    Ok(())
}
