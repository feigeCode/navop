# Local Shell Autosuggestion Handoff Implementation Plan

> Superseded on 2026-04-15: local terminal app-native autocomplete was removed in favor of keeping autocomplete only for SSH terminals. This document is retained as historical context for the discarded approach.

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** On local zsh terminals, automatically hand off autocomplete ownership to shell-side autosuggestion plugins when the user prefers shell autosuggestions.

**Architecture:** Add a lightweight local zsh configuration detector in `terminal`, store the detected state on the `Terminal` model, add a new global `AppSettings` flag to prefer shell autosuggestions, and update the `TerminalView` availability gate so app-native history suggestions disable themselves only for local shells that already provide autosuggestions.

**Tech Stack:** Rust, terminal, terminal_view, main settings

---

### Task 1: Add failing tests for local shell autosuggestion detection and gating

**Files:**
- Modify: `crates/terminal/src/terminal.rs`
- Modify: `crates/terminal_view/src/view.rs`
- Modify: `main/src/setting_tab.rs`

- [ ] **Step 1: Add a pure detector test for zsh-autosuggestions config**

```rust
#[test]
fn detect_zsh_autosuggestions_from_plugins_config() {
    assert!(zsh_autosuggestions_configured(
        "plugins=(git zsh-autosuggestions)"
    ));
}
```

- [ ] **Step 2: Add a pure history gate test for shell handoff**

```rust
#[test]
fn history_prompt_yields_to_local_shell_autosuggestions_when_preferred() {
    let mode = TermMode::empty();

    assert!(!history_prompt_available(
        true,
        true,
        true,
        TerminalConnectionKind::Local,
        mode,
    ));
}
```

- [ ] **Step 3: Add an `AppSettings` default test**

```rust
#[test]
fn app_settings_prefer_shell_autosuggestions_by_default() {
    let settings = AppSettings::default();

    assert!(settings.terminal_prefer_shell_autosuggestions);
}
```

### Task 2: Implement local zsh autosuggestion detection and global preference

**Files:**
- Modify: `crates/terminal/src/terminal.rs`
- Modify: `crates/terminal_view/src/view.rs`
- Modify: `main/src/setting_tab.rs`
- Modify: `main/locales/main.yml`
- Modify: `main/src/home/home_tabs.rs`

- [ ] **Step 1: Detect `zsh-autosuggestions` from local zsh config**
- [ ] **Step 2: Store the detected state on `Terminal`**
- [ ] **Step 3: Add `terminal_prefer_shell_autosuggestions` to `AppSettings`**
- [ ] **Step 4: Sync the new preference into all terminal views**
- [ ] **Step 5: Make `TerminalView` disable app-native autocomplete only when all conditions match**

### Task 3: Verify the end-to-end behavior

- [ ] **Step 1: Run targeted tests**

Run: `cargo test -p terminal zsh_autosuggestions -- --nocapture`
Expected: PASS

Run: `cargo test -p terminal_view history_prompt_yields_to_local_shell_autosuggestions_when_preferred -- --nocapture`
Expected: PASS

Run: `cargo test -p main prefer_shell_autosuggestions -- --nocapture`
Expected: PASS

- [ ] **Step 2: Run broader regression coverage**

Run: `cargo test -p terminal -- --nocapture`
Expected: PASS

Run: `cargo test -p terminal_view -- --nocapture`
Expected: PASS

Run: `cargo test -p main -- --nocapture`
Expected: PASS
