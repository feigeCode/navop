use std::sync::Arc;

use agent_runtime::tools::builtin::EchoTool;
use agent_runtime::{
    ResourceContext, ResourceKind, ResourceRef, ResourceScope, TaskKind, TaskOutcome,
    ToolRegistry,
};
use one_core::llm::{ProviderConfig, ProviderType};
use one_core::llm::storage::ProviderRepository;
use one_core::storage::StorageManager;
use one_core::storage::traits::Repository;

#[tokio::test]
#[ignore = "uses local provider credentials from ~/.config/one-hub/one-hub.db and calls a real model"]
async fn real_model_agent_function_calling_uses_local_provider() {
    let config = load_local_provider_config().expect("local enabled provider config");
    let runtime = ai_chat_view::build_runtime_from_provider_config(
        &config,
        config.model.clone(),
        ToolRegistry::new().with_tool(Arc::new(EchoTool)),
    )
    .expect("build runtime from local provider config");
    let resources = ResourceContext::new().with_resource(
        ResourceRef::new("db-1", ResourceKind::Postgres, "prod analytics")
            .with_scope(ResourceScope::new("database", "Database", "ai_app"))
            .with_scope(ResourceScope::new("schema", "Schema", "public")),
    );
    let session = runtime.create_session(resources);

    let outcome = runtime
        .run_turn_blocking(
            session.id(),
            "请调用 echo 工具,参数 message 必须等于 db-1,然后用一句中文说明已经收到。"
                .to_string()
                .into(),
            TaskKind::Agent,
        )
        .await
        .expect("real model agent turn should run");

    assert!(
        matches!(outcome, TaskOutcome::Completed { .. }),
        "real model should finish after using echo; provider_type={:?}, model={}, outcome={:?}",
        config.provider_type,
        config.model,
        outcome
    );
    let history = session.history_snapshot();
    let observations = history
        .items()
        .iter()
        .filter_map(|item| match item {
            agent_runtime::HistoryItem::Observation(observation) => Some(observation),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(
        observations
            .iter()
            .any(|observation| observation.success
                && observation.tool_name.as_str() == "echo"
                && observation.summary.contains("db-1")),
        "real model should call echo with the resource id; provider_type={:?}, model={}, history={}",
        config.provider_type,
        config.model,
        summarize_history(&history)
    );
}

fn load_local_provider_config() -> anyhow::Result<ProviderConfig> {
    let storage = StorageManager::new()?;
    let repo = ProviderRepository::new(storage.connection());
    let configs = repo
        .list()?
        .into_iter()
        .filter(|config| config.enabled)
        .collect::<Vec<_>>();
    configs
        .iter()
        .filter(|config| supports_function_calling(config))
        .find(|config| config.is_default)
        .or_else(|| configs.iter().find(|config| supports_function_calling(config)))
        .cloned()
        .map(|mut config| {
            if config.provider_type == ProviderType::Ollama {
                config.provider_type = ProviderType::OpenAICompatible;
                config.api_base = Some(String::from("http://127.0.0.1:11434/v1"));
                config.api_key = Some(String::from("ollama"));
            }
            config
        })
        .ok_or_else(|| anyhow::anyhow!("no enabled function-calling capable provider found"))
}

fn supports_function_calling(config: &ProviderConfig) -> bool {
    if matches!(config.provider_type, ProviderType::OnetCli) {
        return false;
    }
    let model = config.model.to_ascii_lowercase();
    let is_deepseek = config.provider_type == ProviderType::DeepSeek || model.contains("deepseek");
    let is_thinking_model = model.contains("v4")
        || model.contains("reasoner")
        || model.contains("r1")
        || model.contains("thinking");
    !(is_deepseek && is_thinking_model)
}

fn summarize_history(history: &agent_runtime::RuntimeHistory) -> String {
    history
        .items()
        .iter()
        .map(|item| match item {
            agent_runtime::HistoryItem::User { text, .. } => {
                format!("user:{}", truncate(text))
            }
            agent_runtime::HistoryItem::Assistant(text) => {
                format!("assistant:{}", truncate(text))
            }
            agent_runtime::HistoryItem::System(text) => {
                format!("system:{}", truncate(text))
            }
            agent_runtime::HistoryItem::ToolCall(call) => {
                format!("tool_call:{} {}", call.tool_name, call.arguments)
            }
            agent_runtime::HistoryItem::Observation(observation) => format!(
                "observation:{} success={} {}",
                observation.tool_name,
                observation.success,
                truncate(&observation.summary)
            ),
        })
        .collect::<Vec<_>>()
        .join(" | ")
}

fn truncate(text: &str) -> String {
    const LIMIT: usize = 240;
    let value = text.replace('\n', "\\n");
    if value.chars().count() <= LIMIT {
        return value;
    }
    let mut out = value.chars().take(LIMIT).collect::<String>();
    out.push_str("...");
    out
}
