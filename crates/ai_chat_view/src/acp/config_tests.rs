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
fn constructs_http_transport_config() {
    let config = AcpAgentConfig::new_http("onetcli-mcp", "Navop Tools", "http://127.0.0.1:3100/");
    assert_eq!(config.id.as_ref(), "onetcli-mcp");
    assert_eq!(config.name.as_ref(), "Navop Tools");
    match config.transport {
        AcpTransport::Http { url } => assert_eq!(url, "http://127.0.0.1:3100/"),
        _ => panic!("期望 Http 传输"),
    }
}

#[test]
fn stdio_config_exposes_skill_context_to_external_acp_agent() {
    let context = SkillContext::new().with_skill(SkillRef::new(
        "ops",
        "Run operational playbooks",
        "/tmp/skills/ops/SKILL.md",
    ));

    let config = AcpAgentConfig::new("codex", "Codex", "codex-acp").with_skill_context(&context);

    match config.transport {
        AcpTransport::Stdio { env, .. } => {
            assert!(env.iter().any(|(name, value)| {
                name == "ONETCLI_SKILLS" && value.contains("Run operational playbooks")
            }));
            assert!(
                env.iter()
                    .any(|(name, value)| { name == "ONETCLI_SELECTED_SKILLS" && value == "ops" })
            );
        }
        _ => panic!("期望 Stdio 传输"),
    }
}
