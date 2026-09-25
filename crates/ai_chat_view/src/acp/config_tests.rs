use super::{
    AcpAgentConfig, AcpStderrLevel, AcpTransport, classify_acp_stderr, strip_ansi_escapes,
};
use agent_runtime::{SkillContext, SkillRef};

#[test]
fn acp_stderr_debug_and_info_are_debug_level() {
    assert_eq!(
        AcpStderrLevel::Debug,
        classify_acp_stderr("DEBUG codex_config::loader: managed config not found")
    );
    assert_eq!(
        AcpStderrLevel::Debug,
        classify_acp_stderr("INFO codex_client::custom_ca: using system root certificates")
    );
}

#[test]
fn acp_stderr_warn_and_error_are_preserved() {
    assert_eq!(
        AcpStderrLevel::Warn,
        classify_acp_stderr("WARN retrying request")
    );
    assert_eq!(
        AcpStderrLevel::Error,
        classify_acp_stderr("ERROR authentication failed")
    );
}

#[test]
fn strips_ansi_color_sequences_from_acp_logs() {
    let line = "\x1b[2m2026-06-09T05:48:33Z\x1b[0m \x1b[34mDEBUG\x1b[0m codex_core::goals";

    assert_eq!(
        strip_ansi_escapes(line),
        "2026-06-09T05:48:33Z DEBUG codex_core::goals"
    );
}

#[test]
fn stdio_transport_maps_command_args_and_env_onto_the_sdk_agent_config() {
    let config = AcpAgentConfig::new("codex", "Codex", "codex-acp")
        .with_args(vec!["--stdio".to_string()])
        .with_env(vec![("CODEX_PATH".to_string(), "/opt/codex".to_string())]);

    let launch = config.to_acp_agent().into_config();

    assert_eq!(std::path::Path::new("codex-acp"), launch.command());
    assert_eq!(["--stdio".to_string()], launch.arguments());
    assert_eq!(
        Some(&"/opt/codex".to_string()),
        launch.environment().get("CODEX_PATH")
    );
}

#[test]
fn stdio_config_exposes_skill_context_to_external_acp_agent() {
    let context = SkillContext::new().with_skill(SkillRef::new(
        "ops",
        "Run operational playbooks",
        "/tmp/skills/ops/SKILL.md",
    ));

    let config = AcpAgentConfig::new("codex", "Codex", "codex-acp").with_skill_context(&context);

    let AcpTransport::Stdio { env, .. } = &config.transport;
    assert!(env.iter().any(|(name, value)| {
        name == "ONETCLI_SKILLS" && value.contains("Run operational playbooks")
    }));
    assert!(
        env.iter()
            .any(|(name, value)| { name == "ONETCLI_SELECTED_SKILLS" && value == "ops" })
    );
}
