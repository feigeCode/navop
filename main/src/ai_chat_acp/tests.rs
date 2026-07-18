use super::{
    acp_agent_config_from_extension_agent, acp_agent_entries_from_agents,
    normalize_acp_agent_config_ids,
};
use ai_chat_view::{AcpAgentConfig, AcpTransport};
use extension_runtime::extension::AcpAgentExtensionAgent;
use std::collections::BTreeMap;
use std::path::PathBuf;

use super::user_config::{parse_user_config, resolve_override};

#[test]
fn normalizes_duplicate_acp_agent_config_ids() {
    let mut configs = vec![
        AcpAgentConfig::new_http("agent", "One", "http://127.0.0.1:1"),
        AcpAgentConfig::new_http("agent", "Two", "http://127.0.0.1:2"),
        AcpAgentConfig::new_http("", "Three", "http://127.0.0.1:3"),
    ];

    normalize_acp_agent_config_ids(&mut configs);

    let ids = configs
        .iter()
        .map(|config| config.id.to_string())
        .collect::<Vec<_>>();
    assert_eq!(vec!["agent", "agent-2", "acp-agent-3"], ids);
}

#[test]
fn builds_acp_agent_config_from_extension_agent() {
    let mut env = BTreeMap::new();
    env.insert("CODEX_HOME".to_string(), "test-home".to_string());
    let manifest_dir = std::env::temp_dir().join("onetcli-acp").join("codex");
    let agent = AcpAgentExtensionAgent::stdio(
        "com.example.codex",
        "codex",
        "Codex",
        manifest_dir.clone(),
        "bin/codex-acp",
        vec!["--stdio".to_string()],
        env,
    );

    let config = acp_agent_config_from_extension_agent(&agent).expect("extension agent config");

    assert_eq!("com.example.codex.codex", config.id.as_ref());
    assert_eq!("Codex", config.name.as_ref());
    match config.transport {
        AcpTransport::Stdio { command, args, env } => {
            let expected_command = manifest_dir
                .join("bin/codex-acp")
                .components()
                .collect::<PathBuf>()
                .display()
                .to_string();
            assert_eq!(expected_command, command);
            assert_eq!(vec!["--stdio".to_string()], args);
            assert_eq!(
                vec![("CODEX_HOME".to_string(), "test-home".to_string())],
                env
            );
        }
        AcpTransport::Http { .. } => panic!("expected stdio transport"),
    }
}

#[test]
fn resolves_environment_reference_without_storing_secret() {
    let parsed = parse_user_config(
        r#"{
            "version": 1,
            "agents": {
                "codex.codex": {
                    "env": {"OPENAI_API_KEY": "${env:OPENAI_API_KEY}"}
                }
            }
        }"#,
    )
    .unwrap();

    let resolved = resolve_override(&parsed.agents["codex.codex"], |name| {
        (name == "OPENAI_API_KEY").then(|| "secret-value".to_string())
    })
    .unwrap();

    assert_eq!(
        Some("secret-value"),
        resolved.env.get("OPENAI_API_KEY").map(String::as_str)
    );
}

#[test]
fn rejects_plaintext_sensitive_value() {
    let error = parse_user_config(
        r#"{
            "version": 1,
            "agents": {
                "codex.codex": {
                    "env": {"OPENAI_API_KEY": "plaintext"}
                }
            }
        }"#,
    )
    .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("OPENAI_API_KEY must use ${env:NAME}")
    );
}

#[test]
fn extension_auth_and_timeouts_reach_runtime_config() {
    let mut agent = AcpAgentExtensionAgent::stdio(
        "com.example.codex",
        "codex",
        "Codex",
        PathBuf::from("/tmp/onetcli-acp/codex"),
        "bin/codex-acp",
        Vec::new(),
        BTreeMap::new(),
    );
    agent.auth.preferred_method = Some("api-key".to_string());
    agent
        .auth
        .methods
        .push(extension_runtime::extension::AcpAgentExtensionAuthMethod {
            id: "api-key".to_string(),
            env_any: vec!["OPENAI_API_KEY".to_string()],
            env_all: Vec::new(),
            interactive: false,
        });
    agent.timeouts.prompt_seconds = 42;

    let config = acp_agent_config_from_extension_agent(&agent).unwrap();

    assert_eq!(Some("api-key"), config.auth.preferred_method.as_deref());
    assert_eq!(42, config.timeouts.prompt.as_secs());
}

#[test]
fn one_invalid_override_does_not_hide_other_agents() {
    let agents = vec![extension_agent("first"), extension_agent("second")];
    let config = parse_user_config(
        r#"{
            "version": 1,
            "agents": {
                "test.first": {
                    "env": {"OPENAI_API_KEY": "${env:MISSING_KEY}"}
                }
            }
        }"#,
    )
    .unwrap();

    let entries = acp_agent_entries_from_agents(&agents, &config, |_| None);

    assert!(entries[0].config.is_none());
    assert!(entries[0].diagnostic.is_some());
    assert!(entries[1].config.is_some());
    assert!(entries[1].diagnostic.is_none());
}

fn extension_agent(id: &str) -> AcpAgentExtensionAgent {
    AcpAgentExtensionAgent::stdio(
        "test",
        id,
        format!("Test {id}"),
        PathBuf::from("/tmp/onetcli-acp/test"),
        "bin/test-acp",
        Vec::new(),
        BTreeMap::new(),
    )
}
