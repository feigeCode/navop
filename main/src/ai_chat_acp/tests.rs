use super::{
    acp_agent_configs_from_settings, normalize_acp_agent_config_ids, parse_json_string_array,
    parse_json_string_object,
};
use ai_chat_view::{AcpAgentConfig, AcpTransport};
use one_core::settings::{
    AcpAgentSettings, AcpAgentTransportSettings, AiChatSettings, AppSettings,
};

#[test]
fn parses_json_string_array_env() {
    unsafe {
        std::env::set_var("ONETCLI_TEST_ACP_ARGS", r#"["-y","agent"]"#);
    }

    assert_eq!(
        vec!["-y".to_string(), "agent".to_string()],
        parse_json_string_array("ONETCLI_TEST_ACP_ARGS")
    );
}

#[test]
fn parses_json_string_object_env() {
    unsafe {
        std::env::set_var("ONETCLI_TEST_ACP_ENV", r#"{"API_KEY":"sk"}"#);
    }

    assert_eq!(
        vec![("API_KEY".to_string(), "sk".to_string())],
        parse_json_string_object("ONETCLI_TEST_ACP_ENV")
    );
}

#[test]
fn builds_http_acp_agent_configs_from_settings() {
    let settings = AppSettings {
        ai_chat: AiChatSettings {
            acp_agents: vec![AcpAgentSettings {
                enabled: true,
                id: "local".to_string(),
                name: "Local ACP".to_string(),
                transport: AcpAgentTransportSettings::Http {
                    url: " http://127.0.0.1:9876 ".to_string(),
                },
            }],
        },
        ..AppSettings::default()
    };

    let configs = acp_agent_configs_from_settings(&settings);

    assert_eq!(1, configs.len());
    assert_eq!("local", configs[0].id.as_ref());
    match &configs[0].transport {
        AcpTransport::Http { url } => assert_eq!("http://127.0.0.1:9876", url),
        AcpTransport::Stdio { .. } => panic!("expected http transport"),
    }
}

#[test]
fn skips_disabled_and_incomplete_acp_agent_settings() {
    let settings = AppSettings {
        ai_chat: AiChatSettings {
            acp_agents: vec![
                AcpAgentSettings {
                    enabled: false,
                    id: "disabled".to_string(),
                    name: "Disabled".to_string(),
                    transport: AcpAgentTransportSettings::Http {
                        url: "http://127.0.0.1:9876".to_string(),
                    },
                },
                AcpAgentSettings {
                    enabled: true,
                    id: "empty".to_string(),
                    name: "Empty".to_string(),
                    transport: AcpAgentTransportSettings::Stdio {
                        command: " ".to_string(),
                        args: Vec::new(),
                        env: Vec::new(),
                    },
                },
            ],
        },
        ..AppSettings::default()
    };

    assert!(acp_agent_configs_from_settings(&settings).is_empty());
}

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
