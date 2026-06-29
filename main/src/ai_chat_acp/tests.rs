use super::{acp_agent_config_from_extension_agent, normalize_acp_agent_config_ids};
use ai_chat_view::{AcpAgentConfig, AcpTransport};
use extension_runtime::extension::AcpAgentExtensionAgent;
use std::collections::BTreeMap;
use std::path::PathBuf;

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
    let agent = AcpAgentExtensionAgent::stdio(
        "com.example.codex",
        "codex",
        "Codex",
        PathBuf::from("/tmp/onetcli-acp/codex"),
        "bin/codex-acp",
        vec!["--stdio".to_string()],
        env,
    );

    let config = acp_agent_config_from_extension_agent(&agent).expect("extension agent config");

    assert_eq!("com.example.codex.codex", config.id.as_ref());
    assert_eq!("Codex", config.name.as_ref());
    match config.transport {
        AcpTransport::Stdio { command, args, env } => {
            assert_eq!("/tmp/onetcli-acp/codex/bin/codex-acp", command);
            assert_eq!(vec!["--stdio".to_string()], args);
            assert_eq!(
                vec![("CODEX_HOME".to_string(), "test-home".to_string())],
                env
            );
        }
        AcpTransport::Http { .. } => panic!("expected stdio transport"),
    }
}
