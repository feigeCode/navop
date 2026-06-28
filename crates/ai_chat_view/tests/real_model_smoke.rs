use std::sync::Arc;
use std::time::Duration;

use agent_runtime::tools::builtin::EchoTool;
use agent_runtime::{
    ResourceContext, ResourceKind, ResourceRef, ResourceScope, StepStatus, TaskKind, TaskOutcome,
    ToolRegistry,
};
use one_core::llm::storage::ProviderRepository;
use one_core::llm::{ProviderConfig, ProviderType};
use one_core::storage::StorageManager;
use one_core::storage::traits::Repository;

#[tokio::test]
#[ignore = "uses local DeepSeek credentials from ~/.config/one-hub/one-hub.db and calls a real model"]
async fn real_model_deepseek_agent_updates_plan_and_calls_tool() {
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

    let outcome = tokio::time::timeout(
        Duration::from_secs(120),
        runtime.run_turn_blocking(
            session.id(),
            concat!(
                "你必须使用工具完成这个测试,不要只用自然语言回答。",
                "第一步先调用 update_plan,创建两个 todo:",
                "1. 调用 echo 工具验证 db-1,status=in_progress;",
                "2. 用一句中文总结结果,status=pending。",
                "然后调用 echo 工具,参数 message 必须等于 db-1。",
                "拿到工具结果后,再用一句中文总结。"
            )
            .to_string()
            .into(),
            TaskKind::Agent,
        ),
    )
    .await
    .expect("real model agent turn should finish within timeout")
    .expect("real model agent turn should run");

    assert!(
        matches!(outcome, TaskOutcome::Completed { .. }),
        "DeepSeek should finish after using update_plan and echo; provider_type={:?}, model={}, outcome={:?}",
        config.provider_type,
        config.model,
        outcome
    );

    let plan = session.current_plan().unwrap_or_else(|| {
        panic!(
            "DeepSeek should call update_plan; provider_type={:?}, model={}, history={}",
            config.provider_type,
            config.model,
            summarize_history(&session.history_snapshot())
        )
    });
    assert!(
        plan.steps.len() >= 2,
        "DeepSeek update_plan should create at least two steps; provider_type={:?}, model={}, plan={:?}",
        config.provider_type,
        config.model,
        plan
    );
    assert!(
        plan.steps
            .iter()
            .any(|step| step.title.contains("db-1") && step.title.contains("echo")),
        "DeepSeek update_plan should include an echo/db-1 step; provider_type={:?}, model={}, plan={:?}",
        config.provider_type,
        config.model,
        plan
    );
    assert!(
        plan.steps
            .iter()
            .any(|step| matches!(step.status, StepStatus::Running | StepStatus::Completed)),
        "DeepSeek update_plan should mark progress; provider_type={:?}, model={}, plan={:?}",
        config.provider_type,
        config.model,
        plan
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
        observations.iter().any(|observation| observation.success
            && observation.tool_name.as_str() == "echo"
            && observation.summary.contains("db-1")),
        "DeepSeek should call echo with the resource id; provider_type={:?}, model={}, history={}",
        config.provider_type,
        config.model,
        summarize_history(&history)
    );
}

#[tokio::test]
#[ignore = "uses local DeepSeek credentials from ~/.config/one-hub/one-hub.db and calls a real model"]
async fn real_model_deepseek_plan_prompt_reaches_update_plan() {
    let config = load_local_provider_config().expect("local enabled provider config");
    let runtime = ai_chat_view::build_runtime_from_provider_config(
        &config,
        config.model.clone(),
        ToolRegistry::new(),
    )
    .expect("build runtime from local provider config");
    let session = runtime.create_session(ResourceContext::new());

    let outcome = tokio::time::timeout(
        Duration::from_secs(120),
        runtime.run_turn_blocking(
            session.id(),
            concat!(
                "先创建一个包含几个步骤的计划清单。",
                "必须通过 update_plan 工具维护计划,不要只用自然语言列项目。",
                "计划至少包含两个步骤。"
            )
            .to_string()
            .into(),
            TaskKind::Agent,
        ),
    )
    .await
    .expect("real model plan turn should finish within timeout")
    .expect("real model plan turn should run");

    assert!(
        matches!(outcome, TaskOutcome::Completed { .. }),
        "DeepSeek should complete plan prompt; provider_type={:?}, model={}, outcome={:?}, history={}",
        config.provider_type,
        config.model,
        outcome,
        summarize_history(&session.history_snapshot())
    );
    let plan = session.current_plan().unwrap_or_else(|| {
        panic!(
            "DeepSeek plan prompt should reach update_plan; provider_type={:?}, model={}, history={}",
            config.provider_type,
            config.model,
            summarize_history(&session.history_snapshot())
        )
    });
    assert!(
        plan.steps.len() >= 2,
        "DeepSeek update_plan should create at least two plan steps; provider_type={:?}, model={}, plan={:?}",
        config.provider_type,
        config.model,
        plan
    );
}

#[test]
fn deepseek_provider_selection_keeps_v4_and_ignores_ollama() {
    let ollama = ProviderConfig {
        id: 1,
        name: String::from("本地ollama"),
        provider_type: ProviderType::Ollama,
        model: String::from("deepseek-v4-flash"),
        enabled: true,
        is_default: true,
        api_base: Some(String::from("http://127.0.0.1:11434")),
        ..ProviderConfig::default()
    };
    let deepseek = ProviderConfig {
        id: 2,
        name: String::from("DeepSeek"),
        provider_type: ProviderType::DeepSeek,
        model: String::from("deepseek-v4-flash"),
        enabled: true,
        is_default: false,
        ..ProviderConfig::default()
    };

    let selected = select_deepseek_provider_config(&[ollama, deepseek])
        .expect("DeepSeek-v4 provider should be selected");

    assert_eq!(selected.provider_type, ProviderType::DeepSeek);
    assert_eq!(selected.model, "deepseek-v4-flash");
}

fn load_local_provider_config() -> anyhow::Result<ProviderConfig> {
    let storage = StorageManager::new()?;
    let repo = ProviderRepository::new(storage.connection());
    let configs = repo
        .list()?
        .into_iter()
        .filter(|config| config.enabled)
        .collect::<Vec<_>>();
    select_deepseek_provider_config(&configs)
        .ok_or_else(|| anyhow::anyhow!("no enabled DeepSeek provider found"))
}

fn select_deepseek_provider_config(configs: &[ProviderConfig]) -> Option<ProviderConfig> {
    configs
        .iter()
        .filter(|config| config.enabled && config.provider_type == ProviderType::DeepSeek)
        .find(|config| config.is_default)
        .or_else(|| {
            configs
                .iter()
                .find(|config| config.enabled && config.provider_type == ProviderType::DeepSeek)
        })
        .cloned()
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
