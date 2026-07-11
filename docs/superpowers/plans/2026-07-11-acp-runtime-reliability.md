# ACP Runtime Reliability Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Restore Claude, Codex, and OpenCode ACP usability with explicit configuration, deterministic authentication, structured lifecycle/errors, and correct empty-turn semantics.

**Architecture:** Extend ACP extension manifests with backward-compatible auth/timeout metadata, merge them with a safe user override file in `main`, and keep the resolved runtime contract in `ai_chat_view::acp`. Split the oversized connection module into pure auth/error/turn/state units plus a thin lifecycle orchestrator, then surface those phases through the existing Agent chat UI and verify the protocol with a fake stdio ACP agent.

**Tech Stack:** Rust 2024, GPUI, Tokio, `agent-client-protocol` 0.14, Serde/serde_json, anyhow, tracing, workspace Cargo tests.

---

## File map

- `crates/extension-runtime/src/extension/acp_agent_provider.rs`: parse and validate manifest transport, auth, and timeout defaults.
- `crates/extension-runtime/src/extension/acp_agent_provider_tests.rs`: focused manifest compatibility and validation tests extracted from the broad provider test file.
- `crates/ai_chat_view/src/acp/config.rs`: resolved runtime config, agent entry, auth and timeout types, plus transport construction.
- `crates/ai_chat_view/src/acp/error.rs`: structured ACP errors, JSON-RPC detail extraction, ANSI removal, and secret redaction.
- `crates/ai_chat_view/src/acp/auth.rs`: pure auth selection and asynchronous authenticate request.
- `crates/ai_chat_view/src/acp/turn.rs`: active-turn state and completion classification.
- `crates/ai_chat_view/src/acp/client.rs`: ACP handlers and initialize/session setup helpers extracted from `connection.rs`.
- `crates/ai_chat_view/src/acp/connection.rs`: public ready/pending handles and lifecycle orchestration only.
- `crates/ai_chat_view/src/acp/state.rs`: session metadata plus connection phase snapshot.
- `crates/ai_chat_view/src/acp/mod.rs`: module declarations and public exports.
- `crates/ai_chat_view/src/agent_view.rs`: consume agent entries, connection outcomes, phases, auth actions, and diagnostic states.
- `crates/ai_chat_view/src/agent_transcript.rs`: replaceable ACP status/error card contract.
- `main/src/ai_chat_acp.rs`: load extension agents and merge user overrides.
- `main/src/ai_chat_acp/user_config.rs`: parse versioned `acp-agents.json`, resolve env references, and reject plaintext secrets.
- `main/src/ai_chat_acp/tests.rs`: config merge and isolation tests.
- `crates/ai_chat_view/tests/fixtures/fake_acp_agent.rs`: deterministic stdio ACP process for protocol integration.
- `crates/ai_chat_view/tests/acp_connection.rs`: connection, empty response, timeout, and process-exit integration tests.
- `docs/superpowers/specs/2026-07-11-acp-runtime-reliability-design.md`: authoritative design; update only if implementation reveals a contract correction.
- `AGENTS.md`: record only new ACP debugging/verification experience proven useful during implementation.

## Task 1: Extend the ACP extension manifest contract

**Files:**
- Modify: `crates/extension-runtime/src/extension/acp_agent_provider.rs`
- Create: `crates/extension-runtime/src/extension/acp_agent_provider_tests.rs`
- Modify: `crates/extension-runtime/src/extension/mod.rs`

- [ ] **Step 1: Add failing compatibility and validation tests**

Add tests that deserialize an old manifest and a new manifest through a test-visible loader:

```rust
#[test]
fn legacy_manifest_uses_safe_auth_and_timeout_defaults() {
    let agent = load_single_agent(legacy_manifest());
    assert!(agent.auth.allow_unauthenticated_fallback);
    assert!(agent.auth.methods.is_empty());
    assert_eq!(30, agent.timeouts.connect_seconds);
    assert_eq!(120, agent.timeouts.authenticate_seconds);
    assert_eq!(600, agent.timeouts.prompt_seconds);
}

#[test]
fn manifest_parses_explicit_auth_requirements() {
    let agent = load_single_agent(manifest_with_auth());
    assert_eq!(Some("api-key"), agent.auth.preferred_method.as_deref());
    assert_eq!(vec!["OPENAI_API_KEY", "CODEX_API_KEY"], agent.auth.methods[0].env_any);
    assert!(agent.auth.methods[1].interactive);
}

#[test]
fn manifest_rejects_timeout_outside_supported_range() {
    let error = load_manifest_text(manifest_with_prompt_timeout(0)).unwrap_err();
    assert!(error.to_string().contains("prompt_seconds must be between 1 and 3600"));
}
```

- [ ] **Step 2: Run the new tests and confirm RED**

Run:

```bash
rtk cargo test -p extension-runtime acp_agent_provider_tests -- --nocapture
```

Expected: compilation fails because auth/timeout fields and the test module do not exist.

- [ ] **Step 3: Add manifest auth and timeout types**

Implement public, cloneable values with Serde defaults:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct AcpAgentExtensionAuth {
    #[serde(default)]
    pub preferred_method: Option<String>,
    #[serde(default = "default_true")]
    pub allow_unauthenticated_fallback: bool,
    #[serde(default)]
    pub methods: Vec<AcpAgentExtensionAuthMethod>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct AcpAgentExtensionAuthMethod {
    pub id: String,
    #[serde(default)]
    pub env_any: Vec<String>,
    #[serde(default)]
    pub env_all: Vec<String>,
    #[serde(default)]
    pub interactive: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct AcpAgentExtensionTimeouts {
    #[serde(default = "default_connect_seconds")]
    pub connect_seconds: u64,
    #[serde(default = "default_authenticate_seconds")]
    pub authenticate_seconds: u64,
    #[serde(default = "default_prompt_seconds")]
    pub prompt_seconds: u64,
}
```

Add `auth` and `timeouts` to `AcpAgentExtensionAgent`, validate non-empty/unique method ids, non-empty env names, and every timeout in `1..=3600`. Re-export the new types from `extension/mod.rs`.

- [ ] **Step 4: Keep the provider file under 300 lines**

Move the provider-specific tests out of `provider_tests.rs` and, if production code still exceeds 300 lines, extract validation into `extension/acp_agent_provider/validation.rs`. Run:

```bash
rtk wc -l crates/extension-runtime/src/extension/acp_agent_provider.rs crates/extension-runtime/src/extension/acp_agent_provider/*.rs
```

Expected: every listed Rust file is at most 300 lines.

- [ ] **Step 5: Run focused and crate tests**

```bash
rtk cargo test -p extension-runtime acp_agent -- --nocapture
rtk cargo test -p extension-runtime provider_tests -- --nocapture
```

Expected: PASS.

- [ ] **Step 6: Commit the manifest contract**

```bash
rtk git add crates/extension-runtime/src/extension
rtk git commit -m "feat(acp): extend agent manifest contract"
```

## Task 2: Add resolved ACP config and safe user overrides

**Files:**
- Modify: `crates/ai_chat_view/src/acp/config.rs`
- Modify: `crates/ai_chat_view/src/acp/mod.rs`
- Modify: `crates/ai_chat_view/src/lib.rs`
- Modify: `main/src/ai_chat_acp.rs`
- Create: `main/src/ai_chat_acp/user_config.rs`
- Modify: `main/src/ai_chat_acp/tests.rs`

- [ ] **Step 1: Write failing user-config tests**

Add tests with an injected environment lookup closure rather than mutating process-global environment:

```rust
#[test]
fn resolves_environment_reference_without_storing_secret() {
    let parsed = parse_user_config(r#"{
        "version": 1,
        "agents": {"codex.codex": {"env": {"OPENAI_API_KEY": "${env:OPENAI_API_KEY}"}}}
    }"#).unwrap();
    let resolved = resolve_override(&parsed.agents["codex.codex"], |name| {
        (name == "OPENAI_API_KEY").then(|| "secret-value".to_string())
    }).unwrap();
    assert_eq!(Some("secret-value"), resolved.env.get("OPENAI_API_KEY").map(String::as_str));
}

#[test]
fn rejects_plaintext_sensitive_value() {
    let error = parse_user_config(r#"{
        "version": 1,
        "agents": {"codex.codex": {"env": {"OPENAI_API_KEY": "plaintext"}}}
    }"#).unwrap_err();
    assert!(error.to_string().contains("OPENAI_API_KEY must use ${env:NAME}"));
}

#[test]
fn one_invalid_override_does_not_hide_other_agents() {
    let entries = merge_agents(two_extension_agents(), config_with_one_missing_env(), |_| None);
    assert!(entries[0].config.is_none());
    assert!(entries[0].diagnostic.is_some());
    assert!(entries[1].config.is_some());
}
```

- [ ] **Step 2: Run the tests and confirm RED**

```bash
rtk cargo test -p onetcli ai_chat_acp::tests -- --nocapture
```

Expected: compilation fails because user-config parsing and `AcpAgentEntry` do not exist.

- [ ] **Step 3: Define resolved config types**

In `config.rs`, add defaults and builders without breaking existing constructors:

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AcpAuthConfig {
    pub requested_method: Option<String>,
    pub preferred_method: Option<String>,
    pub allow_unauthenticated_fallback: bool,
    pub methods: Vec<AcpAuthMethodConfig>,
}

impl Default for AcpAuthConfig {
    fn default() -> Self {
        Self {
            requested_method: None,
            preferred_method: None,
            allow_unauthenticated_fallback: true,
            methods: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AcpTimeoutConfig {
    pub connect: Duration,
    pub authenticate: Duration,
    pub prompt: Duration,
}

#[derive(Clone, Debug)]
pub struct AcpAgentEntry {
    pub id: SharedString,
    pub name: SharedString,
    pub config: Option<AcpAgentConfig>,
    pub diagnostic: Option<AcpConfigDiagnostic>,
}
```

Change the provider contract to return `Vec<AcpAgentEntry>`. Provide `AcpAgentEntry::ready(config)` and `AcpAgentEntry::invalid(id, name, diagnostic)` constructors that enforce the one-of invariant.

- [ ] **Step 4: Implement versioned user config parsing**

Use these Serde types in `user_config.rs`:

```rust
#[derive(Deserialize)]
pub(super) struct AcpUserConfig {
    pub version: u32,
    #[serde(default)]
    pub agents: BTreeMap<String, AcpUserAgentOverride>,
}

#[derive(Clone, Default, Deserialize)]
pub(super) struct AcpUserAgentOverride {
    #[serde(default)]
    pub auth_method: Option<String>,
    #[serde(default)]
    pub args: Option<Vec<String>>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub timeouts: AcpUserTimeoutOverride,
}
```

Accept only `version == 1`. Recognize an env reference only when the entire value matches `${env:NAME}`. Treat names ending in `KEY`, `TOKEN`, `SECRET`, `PASSWORD`, or `CREDENTIAL` as sensitive and reject literal values.

- [ ] **Step 5: Merge extension defaults and user overrides**

Load `get_config_dir()?.join("acp-agents.json")`; missing file means version 1 with an empty map. For each extension agent:

1. Build the extension-derived `AcpAgentConfig`.
2. Apply args, env, requested auth method, and bounded timeout overrides.
3. Resolve env references through `std::env::var` in production and an injected closure in tests.
4. Return an invalid entry for that agent only when its override is invalid.

Do not permit a user override to alter stdio command or HTTP URL.

- [ ] **Step 6: Verify config tests and file limits**

```bash
rtk cargo test -p onetcli ai_chat_acp::tests -- --nocapture
rtk cargo test -p ai_chat_view acp::config -- --nocapture
rtk wc -l main/src/ai_chat_acp.rs main/src/ai_chat_acp/*.rs crates/ai_chat_view/src/acp/config.rs
```

Expected: tests PASS and every Rust file is at most 300 lines.

- [ ] **Step 7: Commit user configuration support**

```bash
rtk git add crates/ai_chat_view/src/acp/config.rs crates/ai_chat_view/src/acp/mod.rs crates/ai_chat_view/src/lib.rs main/src/ai_chat_acp.rs main/src/ai_chat_acp
rtk git commit -m "feat(acp): add safe runtime configuration"
```

## Task 3: Implement structured ACP errors

**Files:**
- Create: `crates/ai_chat_view/src/acp/error.rs`
- Modify: `crates/ai_chat_view/src/acp/mod.rs`

- [ ] **Step 1: Write failing redaction and extraction tests**

```rust
#[test]
fn extracts_nested_provider_message_and_http_status() {
    let data = serde_json::json!({
        "message": "unexpected status 401 Unauthorized: Invalid token",
        "codexErrorInfo": {"responseStreamDisconnected": {"httpStatusCode": 401}}
    });
    let detail = extract_rpc_error_detail("Internal error", Some(&data));
    assert!(detail.contains("401"));
    assert!(detail.contains("Invalid token"));
}

#[test]
fn redacts_secret_assignments_and_authorization_headers() {
    let text = "OPENAI_API_KEY=sk-live Authorization: Bearer abc123";
    assert_eq!(
        "OPENAI_API_KEY=[REDACTED] Authorization: [REDACTED]",
        redact_secrets(text)
    );
}

#[test]
fn removes_ansi_before_presenting_error() {
    assert_eq!("authentication failed", sanitize_detail("\u{1b}[31mauthentication failed\u{1b}[0m"));
}
```

- [ ] **Step 2: Run the tests and confirm RED**

```bash
rtk cargo test -p ai_chat_view acp::error -- --nocapture
```

Expected: compilation fails because `error.rs` and its types do not exist.

- [ ] **Step 3: Implement the error model**

Define `AcpErrorKind`, `AcpRecoveryAction`, and `AcpError` with agent id/name, phase label, summary, detail, and action. Implement `Display` so UI output is summary-first and includes detail only when non-empty. Keep redaction pure and deterministic; never log the original unsanitized detail.

- [ ] **Step 4: Implement JSON value traversal**

Recursively find `httpStatusCode` without relying on provider-specific object paths. Prefer `data.message`, then `data.additionalDetails`, then top-level message, and finally sanitized serialized data. Bound displayed detail to 8 KiB.

- [ ] **Step 5: Run tests and commit**

```bash
rtk cargo test -p ai_chat_view acp::error -- --nocapture
rtk git add crates/ai_chat_view/src/acp/error.rs crates/ai_chat_view/src/acp/mod.rs
rtk git commit -m "feat(acp): add structured errors"
```

## Task 4: Implement deterministic auth selection and turn tracking

**Files:**
- Create: `crates/ai_chat_view/src/acp/auth.rs`
- Create: `crates/ai_chat_view/src/acp/turn.rs`
- Modify: `crates/ai_chat_view/src/acp/mod.rs`

- [ ] **Step 1: Write failing auth decision tests**

```rust
#[test]
fn requested_method_wins_when_advertised_and_configured() {
    let decision = select_auth(&advertised(&["api-key", "chat-gpt"]), &requested_api_key(), &env(&["OPENAI_API_KEY"])).unwrap();
    assert_eq!(AuthDecision::Authenticate(AuthMethodId::from("api-key")), decision);
}

#[test]
fn interactive_method_requires_user_action() {
    let decision = select_auth(&advertised(&["opencode-login"]), &interactive_login(), &EnvAvailability::default()).unwrap();
    assert!(matches!(decision, AuthDecision::RequireInteraction { .. }));
}

#[test]
fn missing_credentials_without_fallback_is_an_error() {
    let error = select_auth(&advertised(&["api-key"]), &required_api_key(), &EnvAvailability::default()).unwrap_err();
    assert_eq!(AcpErrorKind::MissingCredentials, error.kind);
}
```

- [ ] **Step 2: Write failing turn classification tests**

```rust
#[test]
fn successful_rpc_without_agent_output_is_empty_response() {
    let tracker = AcpTurnTracker::new(TurnId::from_string("turn"));
    assert_eq!(TurnOutcome::EmptyResponse, tracker.finish_success(StopReason::EndTurn));
}

#[test]
fn tool_activity_makes_the_turn_successful() {
    let mut tracker = AcpTurnTracker::new(TurnId::from_string("turn"));
    tracker.observe(&tool_call_update());
    assert_eq!(TurnOutcome::Completed, tracker.finish_success(StopReason::EndTurn));
}
```

- [ ] **Step 3: Run both modules and confirm RED**

```bash
rtk cargo test -p ai_chat_view acp::auth -- --nocapture
rtk cargo test -p ai_chat_view acp::turn -- --nocapture
```

Expected: compilation fails because auth and turn modules do not exist.

- [ ] **Step 4: Implement pure auth selection**

Use a result enum that separates local fallback from interactive auth:

```rust
pub enum AuthDecision {
    SkipNoMethods,
    UseLocalFallback,
    Authenticate(AuthMethodId),
    RequireInteraction { methods: Vec<AuthMethodId> },
}
```

Apply the exact decision order from the approved spec. Match env availability against `env_all` and `env_any`; do not inspect environment variable values outside the injected availability map.

- [ ] **Step 5: Implement turn observation and terminal classification**

Track one active turn, reject a second turn, and mark output only for non-empty assistant/reasoning content, tool activity, or plan. Keep metadata and user echo outside the output predicate. Represent terminal results as `Completed`, `Cancelled`, `EmptyResponse`, or `Failed(AcpError)`.

- [ ] **Step 6: Run tests, verify line limits, and commit**

```bash
rtk cargo test -p ai_chat_view acp::auth -- --nocapture
rtk cargo test -p ai_chat_view acp::turn -- --nocapture
rtk wc -l crates/ai_chat_view/src/acp/auth.rs crates/ai_chat_view/src/acp/turn.rs
rtk git add crates/ai_chat_view/src/acp/auth.rs crates/ai_chat_view/src/acp/turn.rs crates/ai_chat_view/src/acp/mod.rs
rtk git commit -m "feat(acp): define auth and turn semantics"
```

Expected: tests PASS; each file is at most 300 lines.

## Task 5: Split and harden the ACP connection lifecycle

**Files:**
- Create: `crates/ai_chat_view/src/acp/client.rs`
- Modify: `crates/ai_chat_view/src/acp/connection.rs`
- Modify: `crates/ai_chat_view/src/acp/state.rs`
- Modify: `crates/ai_chat_view/src/acp/mod.rs`

- [ ] **Step 1: Add failing lifecycle contract tests**

Add pure phase-transition tests in `state.rs` and async tests around injected setup operations in `connection.rs`:

```rust
#[test]
fn ready_cannot_be_entered_before_session_creation() {
    let mut state = AcpSessionState::default();
    state.transition(AcpConnectionPhase::Initializing).unwrap();
    let error = state.transition(AcpConnectionPhase::Ready).unwrap_err();
    assert_eq!(AcpErrorKind::InitializeFailed, error.kind);
}

#[tokio::test]
async fn failed_authentication_does_not_create_session() {
    let ops = FakeSetupOps::authentication_failure();
    let result = run_setup(&ops, &config_requiring_api_key()).await;
    assert!(matches!(result, Err(error) if error.kind == AcpErrorKind::AuthenticationFailed));
    assert_eq!(0, ops.new_session_calls());
}
```

- [ ] **Step 2: Run lifecycle tests and confirm RED**

```bash
rtk cargo test -p ai_chat_view acp::connection -- --nocapture
rtk cargo test -p ai_chat_view acp::state -- --nocapture
```

Expected: tests fail because phase transitions and injected setup operations do not exist.

- [ ] **Step 3: Extract protocol handlers into `client.rs`**

Move initialize request construction, permission handler registration, workspace file handlers, session update callback wiring, and new-session request into focused helpers. Preserve all existing list/load/resume/close/delete/logout, mode, and config-option public APIs.

- [ ] **Step 4: Introduce ready and pending outcomes**

Implement:

```rust
pub enum AcpConnectOutcome {
    Ready(AcpConnection),
    AuthenticationRequired(AcpPendingConnection),
}
```

`AcpPendingConnection::authenticate(method_id)` must apply the configured authentication timeout, return a ready connection only after `session/new`, and retain the same transport lifecycle. Cancelling or dropping the pending connection shuts down the child process.

- [ ] **Step 5: Add bounded connect, auth, and prompt operations**

Wrap readiness in `config.timeouts.connect`, auth in `config.timeouts.authenticate`, and prompt in `config.timeouts.prompt`. Prompt timeout must send `CancelNotification`, wait at most two seconds, emit `TurnFailed(PromptTimeout)`, clear the active tracker, and return phase to Ready.

- [ ] **Step 6: Wire turn tracking before sending prompt**

Register the tracker and broadcast `TurnStarted` before the request is sent. The notification handler observes every `SessionUpdate` before translation. On successful response, emit `TurnCompleted` only for `TurnOutcome::Completed`; emit a structured failure for EmptyResponse. On RPC error, sanitize and structure the error before logging or broadcasting.

- [ ] **Step 7: Preserve lifecycle shutdown semantics**

Retain the existing shutdown oneshot plus two-second forced abort. When the connection loop exits unexpectedly, fail the active turn once, transition to Failed, and avoid a duplicate `TurnFailed` from the prompt task.

- [ ] **Step 8: Run connection tests and file-size gate**

```bash
rtk cargo test -p ai_chat_view acp::connection -- --nocapture
rtk cargo test -p ai_chat_view acp::state -- --nocapture
rtk cargo test -p ai_chat_view acp::translate -- --nocapture
rtk wc -l crates/ai_chat_view/src/acp/client.rs crates/ai_chat_view/src/acp/connection.rs crates/ai_chat_view/src/acp/state.rs
```

Expected: PASS; every file is at most 300 lines.

- [ ] **Step 9: Commit lifecycle refactor**

```bash
rtk git add crates/ai_chat_view/src/acp
rtk git commit -m "refactor(acp): harden connection lifecycle"
```

## Task 6: Surface ACP readiness, auth, and diagnostics in the chat UI

**Files:**
- Modify: `crates/ai_chat_view/src/agent_view.rs`
- Modify: `crates/ai_chat_view/src/agent_transcript.rs`
- Modify: `crates/ai_chat_view/src/input/context.rs`
- Modify: `crates/ai_chat_view/src/input/agent_input.rs`
- Modify: `crates/ai_chat_view/src/default_panel.rs`
- Modify: `crates/ai_chat_view/src/default_panel_tests.rs`

- [ ] **Step 1: Add failing UI contract tests**

```rust
#[test]
fn acp_phase_status_replaces_previous_phase() {
    let mut transcript = AgentTranscript::new();
    transcript.set_acp_status("正在启动 Codex");
    transcript.set_acp_status("正在协商 ACP 协议");
    assert_eq!(1, transcript.acp_status_count());
    assert_eq!(Some("正在协商 ACP 协议"), transcript.acp_status_text());
}

#[test]
fn empty_response_replaces_running_status_with_recovery_error() {
    let mut transcript = AgentTranscript::new();
    transcript.set_acp_status("ACP 正在响应…");
    transcript.set_acp_error(&AcpError::empty_response("opencode", "OpenCode"));
    assert_eq!(0, transcript.pending_status_count());
    assert!(transcript.last_message_content().contains("没有返回任何内容"));
}
```

Add composer option tests proving invalid entries remain visible but disabled with their diagnostic.

- [ ] **Step 2: Run UI tests and confirm RED**

```bash
rtk cargo test -p ai_chat_view agent_transcript -- --nocapture
rtk cargo test -p ai_chat_view default_panel_tests -- --nocapture
```

Expected: compilation fails because replaceable ACP status/error APIs and entry-based options do not exist.

- [ ] **Step 3: Convert view configuration from configs to entries**

Change `AgentChatViewConfig` and provider plumbing to hold `Vec<AcpAgentEntry>`. Ready entries remain selectable; invalid entries appear disabled with a subtitle from `AcpConfigDiagnostic`. Selecting a disabled entry must not start a task.

- [ ] **Step 4: Handle connection outcomes explicitly**

When selecting a ready entry:

1. Set Starting status and disable prompt submission.
2. Update the same status as phase changes.
3. Store `AcpPendingConnection` for AuthenticationRequired and render login/cancel actions.
4. Only set backend to ACP after `AcpConnectOutcome::Ready`.
5. On error, keep the requested Agent selected long enough to show its recovery details; do not silently switch the transcript back to Local.

- [ ] **Step 5: Add interactive auth actions**

The login action invokes `pending.authenticate(method_id)`. The cancel action drops the pending connection and returns to the local backend. Do not automatically open a browser during startup; any browser launch remains inside the selected Agent's authenticate request after explicit user action.

- [ ] **Step 6: Add replaceable status and error messages**

Track one ACP lifecycle message id separately from active turn status. Phase changes update it in place. Assistant output removes only the turn status. Connection failures replace lifecycle status with a structured error; prompt failures replace turn status with a structured error.

- [ ] **Step 7: Run UI tests and line gates**

```bash
rtk cargo test -p ai_chat_view agent_transcript -- --nocapture
rtk cargo test -p ai_chat_view default_panel_tests -- --nocapture
rtk cargo test -p ai_chat_view agent_view -- --nocapture
rtk wc -l crates/ai_chat_view/src/agent_view.rs crates/ai_chat_view/src/agent_transcript.rs
```

If an already-oversized existing file remains above 300 lines, extract only the ACP-specific state/actions into `agent_view/acp.rs` and transcript status logic into `agent_transcript/acp.rs`; do not mix unrelated refactoring into this task.

- [ ] **Step 8: Commit UI integration**

```bash
rtk git add crates/ai_chat_view/src
rtk git commit -m "feat(acp): expose connection and auth status"
```

## Task 7: Add protocol-level fake-agent integration coverage

**Files:**
- Create: `crates/ai_chat_view/tests/fixtures/fake_acp_agent.rs`
- Create: `crates/ai_chat_view/tests/acp_connection.rs`
- Modify: `crates/ai_chat_view/Cargo.toml`

- [ ] **Step 1: Implement a deterministic fake ACP executable fixture**

The fixture reads line-delimited JSON-RPC from stdin and accepts one mode argument:

```rust
enum Mode {
    Text,
    Empty,
    AuthRequired,
    PromptError,
    PromptHang,
    ExitAfterInitialize,
}
```

It must respond to initialize and session/new, optionally advertise auth methods, accept authenticate, send one `agent_message_chunk` in Text mode, return `end_turn` without updates in Empty mode, return nested 401 data in PromptError mode, wait for session/cancel in PromptHang mode, and exit after initialize in ExitAfterInitialize mode.

- [ ] **Step 2: Add failing integration tests for all terminal paths**

```rust
#[tokio::test]
async fn empty_agent_response_becomes_turn_failure() {
    let connection = connect_fake(Mode::Empty).await;
    let event = prompt_until_terminal(&connection, "hello").await;
    assert!(matches!(event, RuntimeEvent::TurnFailed { reason, .. } if reason.contains("没有返回任何内容")));
}

#[tokio::test]
async fn prompt_timeout_sends_cancel_and_connection_remains_reusable() {
    let connection = connect_fake_with_timeout(Mode::PromptHang, Duration::from_millis(100)).await;
    let event = prompt_until_terminal(&connection, "hello").await;
    assert!(matches!(event, RuntimeEvent::TurnFailed { reason, .. } if reason.contains("超时")));
    assert_eq!(AcpConnectionPhase::Ready, connection.state().phase());
}
```

Also test successful text, interactive auth, nested 401 extraction, and process exit.

- [ ] **Step 3: Run integration tests and confirm RED**

```bash
rtk cargo test -p ai_chat_view --test acp_connection -- --nocapture
```

Expected: tests fail until the fixture target and lifecycle integration are complete.

- [ ] **Step 4: Wire the fixture binary for tests**

Add a `[[test]]` target for `acp_connection` and use `std::env::current_exe()` or a Cargo-provided binary path to launch the fixture without `npm`, network, or user credentials. Keep fixture messages compliant with ACP protocol version 1 used by the client.

- [ ] **Step 5: Make all integration cases pass**

```bash
rtk cargo test -p ai_chat_view --test acp_connection -- --nocapture
```

Expected: PASS for Text, Empty, AuthRequired, PromptError, PromptHang, and ExitAfterInitialize.

- [ ] **Step 6: Commit protocol integration coverage**

```bash
rtk git add crates/ai_chat_view/Cargo.toml crates/ai_chat_view/tests
rtk git commit -m "test(acp): cover protocol lifecycle"
```

## Task 8: Verify current ACP extensions and document proven operational guidance

**Files:**
- Modify when evidence warrants: `AGENTS.md`
- Modify when contract corrections are required: `docs/superpowers/specs/2026-07-11-acp-runtime-reliability-design.md`

- [ ] **Step 1: Run format and targeted test suites**

```bash
rtk cargo fmt --all -- --check
rtk cargo test -p extension-runtime acp_agent -- --nocapture
rtk cargo test -p onetcli ai_chat_acp::tests -- --nocapture
rtk cargo test -p ai_chat_view acp -- --nocapture
rtk cargo test -p ai_chat_view agent_transcript -- --nocapture
rtk cargo test -p ai_chat_view --test acp_connection -- --nocapture
```

Expected: all commands exit 0.

- [ ] **Step 2: Run compile and lint gates**

```bash
rtk cargo check -p extension-runtime -p ai_chat_view -p onetcli
rtk cargo clippy -p extension-runtime -p ai_chat_view -p onetcli --all-targets -- -D warnings
```

Expected: both commands exit 0 with no warnings promoted to errors.

- [ ] **Step 3: Run file and function-size audit**

```bash
rtk wc -l crates/ai_chat_view/src/acp/*.rs main/src/ai_chat_acp.rs main/src/ai_chat_acp/*.rs crates/extension-runtime/src/extension/acp_agent_provider.rs
rtk rg -n "fn |async fn " crates/ai_chat_view/src/acp main/src/ai_chat_acp crates/extension-runtime/src/extension/acp_agent_provider.rs
```

Expected: every new/modified Rust file is at most 300 lines; inspect each changed function and split any function over 50 lines before proceeding.

- [ ] **Step 4: Smoke-test OpenCode**

Select the installed OpenCode ACP extension and submit a short prompt. Verify one of these evidence-backed outcomes:

- Valid local login/provider: non-empty assistant output and reusable Ready state.
- Missing login/provider: AuthenticationRequired or EmptyResponse with a concrete login/config recovery action, never a blank successful turn.

- [ ] **Step 5: Smoke-test Codex**

Use valid local ChatGPT/Codex login or an `${env:OPENAI_API_KEY}` override. Verify non-empty output. Then use an intentionally invalid disposable test credential only if one is already available for testing; otherwise validate the nested 401 behavior through the fake agent. Confirm no credential value appears in the application log.

- [ ] **Step 6: Smoke-test Claude**

Use the installed Claude ACP extension with its existing local authentication. Verify non-empty output when the provider/model is valid. If the provider reports a retired model, confirm the UI displays the model message instead of only `Internal error`.

- [ ] **Step 7: Record reusable ACP experience**

Only if implementation or smoke testing reveals a repeatable repository-specific constraint, append one minimal experience entry to `AGENTS.md` with title, trigger signal, root cause, correct action, verification, and scope. Do not add speculative guidance.

- [ ] **Step 8: Commit verification-driven documentation**

If `AGENTS.md` or the spec changed:

```bash
rtk git add AGENTS.md docs/superpowers/specs/2026-07-11-acp-runtime-reliability-design.md
rtk git commit -m "docs(acp): record runtime verification guidance"
```

If neither changed, skip this commit.

## Task 9: Review and completion verification

**Files:**
- Review all ACP-related changes since `5ef7df26`.

- [ ] **Step 1: Invoke `superpowers:requesting-code-review`**

Review the diff against the approved spec, focusing on auth fallback safety, secret redaction, duplicate terminal events, process cleanup, path validation, file/function limits, and preservation of session APIs.

- [ ] **Step 2: Apply review findings through `superpowers:receiving-code-review`**

For every accepted finding, reproduce or prove it with a focused test before changing production code. Re-run the affected focused suite after each fix.

- [ ] **Step 3: Invoke `superpowers:verification-before-completion`**

Run the exact Task 8 format, test, check, clippy, size, and smoke-test gates from a clean working tree state. Capture exit codes and distinguish external credential/provider limitations from application failures.

- [ ] **Step 4: Audit acceptance criteria one by one**

Create a checklist from the design's Common, Claude, Codex, OpenCode, and Engineering acceptance sections. For each item, cite a test, command output, log excerpt, or manual observation. Treat missing evidence as incomplete work.

- [ ] **Step 5: Commit any final verified fixes**

```bash
rtk git add crates/extension-runtime crates/ai_chat_view main/src/ai_chat_acp.rs main/src/ai_chat_acp AGENTS.md docs/superpowers/specs/2026-07-11-acp-runtime-reliability-design.md
rtk git commit -m "fix(acp): address final verification findings"
```

Skip the commit if verification required no changes.
