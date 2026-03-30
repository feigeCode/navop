# Global HTTP Proxy Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a structured global HTTP proxy setting with popup configuration UI and immediate application-wide effect for all requests that use the shared GPUI HTTP client.

**Architecture:** Store proxy settings in `AppSettings`, expose a popup window editor from the settings page, and centralize HTTP client rebuilding in one helper so startup initialization, save/apply, and connection testing all use the same path. Keep scope limited to HTTP traffic that goes through `cx.http_client()` and do not change SSH/SFTP/database transport proxy flows.

**Tech Stack:** Rust, GPUI, `reqwest_client::ReqwestClient`, serde settings persistence, Tokio-backed HTTP tasks

---

### Task 1: Model proxy settings and centralized client rebuild

**Files:**
- Modify: `main/src/setting_tab.rs`
- Modify: `main/src/onetcli_app.rs`
- Modify: `crates/reqwest_client/src/reqwest_client.rs`

- [x] Add `GlobalProxySettings` and supporting enum/default/serde fields to `AppSettings`.
- [x] Add helpers to validate fields and convert the structured proxy config into a proxy URL.
- [x] Add one shared helper that builds a `ReqwestClient` from settings and user agent, then use it both at startup and when applying new settings.
- [x] Keep failure behavior safe: invalid proxy config must not replace the current client.

### Task 2: Add failing tests for proxy modeling and client construction

**Files:**
- Modify: `main/src/setting_tab.rs`
- Modify: `crates/reqwest_client/src/reqwest_client.rs`

- [x] Add tests for `GlobalProxySettings` validation and URL generation.
- [x] Add tests for authenticated and unauthenticated proxy URL construction.
- [x] Run the targeted tests first and confirm they fail for the expected missing behavior.

### Task 3: Build the global proxy settings popup and settings entry

**Files:**
- Modify: `main/src/setting_tab.rs`
- Modify: `main/locales/main.yml`

- [x] Add a settings entry near the existing update group that opens a popup window for global proxy settings.
- [x] Implement popup form state and fields: enable, type, host, port, username, password.
- [x] Add local validation and disabled-field behavior when proxy is turned off.
- [x] Add `Test Connection`, `Cancel`, and `Save` actions.

### Task 4: Wire save/apply/test behavior

**Files:**
- Modify: `main/src/setting_tab.rs`
- Modify: `main/src/onetcli_app.rs`

- [x] On save, validate form input, rebuild a temporary client, and only persist + replace the global client on success.
- [x] On test, build a temporary client from the unsaved form state and issue a lightweight request through Tokio.
- [x] Surface success/failure via window notifications and keep the popup open on failure.

### Task 5: Verify integrated behavior

**Files:**
- Modify: `main/src/setting_tab.rs` (tests if needed)
- Modify: `main/src/onetcli_app.rs` (tests if needed)

- [x] Run `cargo test -p main` and confirm proxy-related tests plus existing update tests pass.
- [x] Run `cargo build -p main` and confirm startup wiring compiles cleanly.
- [x] Run `cargo clippy -p main -- -D warnings` and record whether failures are new or pre-existing workspace issues.

## Verification Notes

- `cargo build -p main`: passed on 2026-03-30
- `cargo test -p main`: passed on 2026-03-30, `17 passed`
- `cargo clippy -p main -- -D warnings`: still fails due pre-existing warnings in `crates/core` and `crates/one_ui`, not caused by this proxy feature
- Manual verification still recommended for actual proxy routing after saving settings in the UI
