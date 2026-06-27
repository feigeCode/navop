use super::ai_chat_agent_json::{format_acp_agents_json_for_test, parse_acp_agents_json_for_test};
use super::ai_chat_settings::{
    ai_chat_setting_item_ids, ai_chat_transport_option_values, parse_stdio_env_json_for_test,
};
use one_core::settings::{AcpAgentSettings, AcpAgentTransportSettings, AiChatSettings};

#[test]
fn ai_chat_settings_exposes_primary_acp_agent_fields() {
    assert_eq!(
        vec![
            "acp_enabled",
            "acp_name",
            "acp_transport",
            "acp_http_url",
            "acp_stdio_command",
            "acp_stdio_args",
            "acp_stdio_env",
            "acp_agents_json",
        ],
        ai_chat_setting_item_ids()
    );
}

#[test]
fn transport_options_match_persisted_values() {
    assert_eq!(vec!["http", "stdio"], ai_chat_transport_option_values());
}

#[test]
fn parses_stdio_env_json_as_string_pairs() {
    let env = parse_stdio_env_json_for_test(r#"{"API_KEY":"sk","IGNORED":123}"#).unwrap();

    assert_eq!(vec![("API_KEY".to_string(), "sk".to_string())], env);
}

#[test]
fn acp_agents_json_round_trips_multiple_agents() {
    let settings = AiChatSettings {
        acp_agents: vec![
            AcpAgentSettings {
                enabled: true,
                id: "http".to_string(),
                name: "HTTP Agent".to_string(),
                transport: AcpAgentTransportSettings::Http {
                    url: "http://127.0.0.1:9876".to_string(),
                },
            },
            AcpAgentSettings {
                enabled: true,
                id: "stdio".to_string(),
                name: "Stdio Agent".to_string(),
                transport: AcpAgentTransportSettings::Stdio {
                    command: "agent".to_string(),
                    args: vec!["acp".to_string()],
                    env: vec![("API_KEY".to_string(), "sk".to_string())],
                },
            },
        ],
    };

    let json = format_acp_agents_json_for_test(&settings);
    let parsed = parse_acp_agents_json_for_test(&json).expect("agents json should parse");

    assert_eq!(settings.acp_agents, parsed);
}
