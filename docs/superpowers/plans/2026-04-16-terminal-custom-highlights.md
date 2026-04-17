# Terminal Custom Highlights Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add one global custom highlight rules feature for all terminal instances, editable from the terminal sidebar settings panel and applied through the terminal addon decoration pipeline.

**Architecture:** Extend `terminal_view::settings::TerminalSettings` with persisted custom highlight rules, compile those rules inside a new `CustomHighlightAddon`, and wire the sidebar settings panel to update the global terminal settings store. Each terminal view keeps its addon in sync by reacting to `TerminalSettingsEvent::Changed`.

**Tech Stack:** Rust, GPUI entities/events, serde/serde_json, `regex`, existing terminal addon decoration system, existing terminal settings persistence

---

### Task 1: Add failing tests for settings and highlight rule compilation

**Files:**
- Modify: `crates/terminal_view/src/settings.rs`
- Modify: `crates/terminal_view/src/addon.rs`

- [ ] **Step 1: Add a terminal settings round-trip test that includes custom highlight rules**

```rust
#[test]
fn terminal_settings_round_trip_preserves_custom_highlights() {
    let path = temp_file_path("terminal-settings-highlights-round-trip");
    let settings = TerminalSettings {
        custom_highlights: vec![TerminalHighlightRule {
            id: "rule-1".into(),
            enabled: true,
            pattern: "\\berror\\b".into(),
            foreground: Some("#ff0000".into()),
            background: Some("#1f1f1f".into()),
            priority: 42,
            note: "Errors".into(),
        }],
        ..TerminalSettings::default()
    };

    save_settings_to_path(&path, &settings).expect("应写入 terminal settings");
    let loaded = load_settings_from_path(&path).expect("应读回 terminal settings");

    assert_eq!(loaded.custom_highlights, settings.custom_highlights);
}
```

- [ ] **Step 2: Add addon-focused tests for rule validation and visible-line matching**

```rust
#[test]
fn custom_highlight_rule_requires_pattern_and_color() {
    let rule = TerminalHighlightRule {
        id: "rule-1".into(),
        enabled: true,
        pattern: "".into(),
        foreground: None,
        background: None,
        priority: 1,
        note: String::new(),
    };

    assert!(rule.validate().is_err());
}

#[test]
fn custom_highlight_compiler_skips_disabled_rules() {
    let rules = vec![TerminalHighlightRule {
        id: "rule-1".into(),
        enabled: false,
        pattern: "root".into(),
        foreground: Some("#ff0000".into()),
        background: None,
        priority: 10,
        note: String::new(),
    }];

    let compiled = compile_custom_highlight_rules(&rules);

    assert!(compiled.is_empty());
}
```

- [ ] **Step 3: Run focused tests and verify they fail for the expected reason**

Run: `cargo test -p terminal_view terminal_settings_round_trip_preserves_custom_highlights -- --nocapture`
Expected: FAIL because `custom_highlights` does not exist yet

Run: `cargo test -p terminal_view custom_highlight_ -- --nocapture`
Expected: FAIL because custom highlight validation/compilation helpers do not exist yet

### Task 2: Implement persisted custom highlight rules and addon compilation

**Files:**
- Modify: `crates/terminal_view/src/settings.rs`
- Modify: `crates/terminal_view/src/addon.rs`
- Modify: `crates/terminal_view/src/lib.rs`

- [ ] **Step 1: Add `TerminalHighlightRule` plus `custom_highlights` on `TerminalSettings`**

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalHighlightRule {
    pub id: String,
    pub enabled: bool,
    pub pattern: String,
    pub foreground: Option<String>,
    pub background: Option<String>,
    pub priority: u8,
    pub note: String,
}
```

- [ ] **Step 2: Add lightweight validation helpers on `TerminalHighlightRule`**

```rust
impl TerminalHighlightRule {
    pub fn validate(&self) -> Result<(), String> {
        if self.pattern.trim().is_empty() {
            return Err("正则不能为空".into());
        }
        if self.foreground.is_none() && self.background.is_none() {
            return Err("至少设置一种颜色".into());
        }
        Ok(())
    }
}
```

- [ ] **Step 3: Add `CustomHighlightAddon` plus pure compile helpers**

```rust
fn compile_custom_highlight_rules(
    rules: &[TerminalHighlightRule],
) -> Vec<CompiledHighlightRule> {
    // filter enabled + valid + regex-compilable + color-parsable rules
}
```

- [ ] **Step 4: Register the addon in `register_default_addons()` and export rule types**

Run: `cargo test -p terminal_view terminal_settings_round_trip_preserves_custom_highlights custom_highlight_ -- --nocapture`
Expected: PASS

### Task 3: Wire runtime settings changes into every terminal view

**Files:**
- Modify: `crates/terminal_view/src/view.rs`

- [ ] **Step 1: Add a helper that pushes current custom highlight rules into the addon**

```rust
fn apply_custom_highlight_rules(
    &mut self,
    rules: &[TerminalHighlightRule],
    cx: &mut Context<Self>,
) {
    if let Some(addon) = self
        .addon_manager
        .get_as_mut::<CustomHighlightAddon>("custom_highlights")
    {
        addon.set_rules(rules);
        cx.notify();
    }
}
```

- [ ] **Step 2: Apply the rules from `apply_settings_snapshot()`**

```rust
self.apply_custom_highlight_rules(&settings.custom_highlights, cx);
```

- [ ] **Step 3: Keep construction-time and runtime settings updates in sync**

Run: `cargo test -p terminal_view custom_highlight_ -- --nocapture`
Expected: PASS

### Task 4: Add sidebar rule management UI

**Files:**
- Modify: `crates/terminal_view/src/sidebar/settings_panel.rs`
- Modify: `crates/terminal_view/src/sidebar/mod.rs`

- [ ] **Step 1: Add settings panel/sidebar events for add, edit, toggle, and delete**

```rust
SettingsPanelEvent::CustomHighlightsChanged(Vec<TerminalHighlightRule>)
TerminalSidebarEvent::CustomHighlightsChanged(Vec<TerminalHighlightRule>)
```

- [ ] **Step 2: Track `custom_highlights` in `SettingsPanel` state and render a new section**

```rust
fn render_custom_highlight_section(&self, cx: &mut Context<Self>) -> impl IntoElement {
    // add button + list rows + toggle/edit/delete actions
}
```

- [ ] **Step 3: Add a simple dialog editor for one rule with regex/color validation before save**

Run: `cargo check -p terminal_view`
Expected: PASS

### Task 5: Persist sidebar changes through the global terminal settings store

**Files:**
- Modify: `crates/terminal_view/src/view.rs`
- Modify: `crates/terminal_view/src/sidebar/mod.rs`

- [ ] **Step 1: Handle `CustomHighlightsChanged` in `TerminalView::handle_sidebar_event()`**

```rust
TerminalSidebarEvent::CustomHighlightsChanged(rules) => {
    let rules = rules.clone();
    let _ = update_settings(cx, move |settings| {
        settings.custom_highlights = rules;
    });
}
```

- [ ] **Step 2: Ensure sidebar state stays synchronized after any global settings change**

Run: `cargo test -p terminal_view terminal_settings -- --nocapture`
Expected: PASS

### Verification

- [ ] `cargo test -p terminal_view terminal_settings -- --nocapture`
- [ ] `cargo test -p terminal_view custom_highlight_ -- --nocapture`
- [ ] `cargo check -p terminal_view`
