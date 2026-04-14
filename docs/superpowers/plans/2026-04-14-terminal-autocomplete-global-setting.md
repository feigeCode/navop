# Terminal Autocomplete Global Setting Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add one global application setting that can disable all terminal autocomplete behavior and keep the terminal sidebar in sync with that global state.

**Architecture:** Reuse existing `AppSettings` terminal preference flow instead of introducing a new config file. Gate all terminal autocomplete behavior through a single pure helper in `terminal_view::view`, expose the setting in both the global settings page and terminal sidebar, then fan the change out to all live terminal views through the existing event synchronization path.

**Tech Stack:** Rust, GPUI, `AppSettings`, `terminal_view`, `rust-i18n`

---

### Task 1: Add failing tests for the new global autocomplete setting

**Files:**
- Modify: `crates/terminal_view/src/view.rs`
- Modify: `main/src/setting_tab.rs`

- [ ] **Step 1: Add a pure autocomplete gate test in `view.rs`**

```rust
#[test]
fn history_prompt_requires_global_autocomplete_switch() {
    let mode = TermMode::empty();

    assert!(history_prompt_available(
        true,
        TerminalConnectionKind::Local,
        mode,
    ));
    assert!(!history_prompt_available(
        false,
        TerminalConnectionKind::Local,
        mode,
    ));
}
```

- [ ] **Step 2: Add an `AppSettings` default-value test in `setting_tab.rs`**

```rust
#[test]
fn app_settings_enable_terminal_autocomplete_by_default() {
    let settings = AppSettings::default();

    assert!(settings.terminal_enable_autocomplete);
}
```

- [ ] **Step 3: Run the focused tests and verify they fail for the expected reason**

Run: `cargo test -p terminal_view history_prompt_requires_global_autocomplete_switch -- --nocapture`
Expected: FAIL because `history_prompt_available` does not exist yet

Run: `cargo test -p main app_settings_enable_terminal_autocomplete_by_default -- --nocapture`
Expected: FAIL because `terminal_enable_autocomplete` does not exist yet

### Task 2: Add the new global setting to `AppSettings` and global settings UI

**Files:**
- Modify: `main/src/setting_tab.rs`
- Modify: `main/locales/main.yml`

- [ ] **Step 1: Extend `AppSettings` with a default-true terminal autocomplete flag**

```rust
#[serde(default = "default_true")]
pub terminal_enable_autocomplete: bool,
```

- [ ] **Step 2: Initialize the new field in `AppSettings::default()`**

```rust
terminal_enable_autocomplete: default_true(),
```

- [ ] **Step 3: Add a terminal settings switch in the global settings page**

```rust
SettingItem::new(
    t!("Settings.General.Terminal.autocomplete"),
    SettingField::switch(
        |cx: &App| AppSettings::global(cx).terminal_enable_autocomplete,
        |val: bool, cx: &mut App| {
            let settings = AppSettings::global_mut(cx);
            settings.terminal_enable_autocomplete = val;
            settings.save();
            let settings_snapshot = settings.clone();
            sync_terminal_settings_to_all(settings_snapshot, cx);
        },
    )
    .default_value(default_settings.terminal_enable_autocomplete),
)
.description(t!("Settings.General.Terminal.autocomplete_desc").to_string())
```

- [ ] **Step 4: Add locale strings for the new global setting**

```yaml
autocomplete:
  en: Terminal autocomplete
  zh-CN: 终端自动补全
  zh-HK: 終端自動補全
autocomplete_desc:
  en: Show command history suggestions and search completion in supported terminals
  zh-CN: 在支持的终端中显示命令历史建议和搜索补全
  zh-HK: 在支援的終端中顯示命令歷史建議和搜尋補全
```

### Task 3: Wire the setting into `TerminalView` runtime behavior

**Files:**
- Modify: `crates/terminal_view/src/view.rs`
- Modify: `main/src/home/home_tabs.rs`

- [ ] **Step 1: Add a new `autocomplete_enabled` field on `TerminalView`**

```rust
autocomplete_enabled: bool,
```

- [ ] **Step 2: Replace the inline availability logic with a pure helper**

```rust
fn history_prompt_available(
    autocomplete_enabled: bool,
    connection_kind: TerminalConnectionKind,
    mode: TermMode,
) -> bool {
    autocomplete_enabled
        && matches!(
            connection_kind,
            TerminalConnectionKind::Local | TerminalConnectionKind::Ssh
        )
        && !mode.contains(TermMode::ALT_SCREEN)
        && !mode.contains(TermMode::VI)
}
```

- [ ] **Step 3: Make `history_prompt_enabled()` call the helper**

```rust
history_prompt_available(self.autocomplete_enabled, connection_kind, mode)
```

- [ ] **Step 4: Expand `apply_terminal_settings()` so global sync applies the autocomplete flag**

```rust
pub fn apply_terminal_settings(
    &mut self,
    font_size: f32,
    auto_copy: bool,
    middle_click_paste: bool,
    sync_path: bool,
    autocomplete_enabled: bool,
    window: &mut Window,
    cx: &mut Context<Self>,
)
```

- [ ] **Step 5: When autocomplete is turned off at runtime, clear active prompt UI**

```rust
self.autocomplete_enabled = enabled;
if !enabled {
    self.suggestion_debounce.take();
    self.dismiss_history_prompt();
    self.hide_history_prompt_dropdown();
    self.dismiss_history_prompt_matches();
}
```

- [ ] **Step 6: Emit and handle a new `TerminalViewEvent::AutocompleteChanged`**

```rust
TerminalViewEvent::AutocompleteChanged { enabled }
```

### Task 4: Add a synchronized switch to the terminal sidebar settings panel

**Files:**
- Modify: `crates/terminal_view/src/sidebar/settings_panel.rs`
- Modify: `crates/terminal_view/src/sidebar/mod.rs`
- Modify: `crates/terminal_view/locales/terminal_view.yml`

- [ ] **Step 1: Add `AutocompleteChanged(bool)` events to the settings panel and sidebar**

```rust
SettingsPanelEvent::AutocompleteChanged(bool)
TerminalSidebarEvent::AutocompleteChanged(bool)
```

- [ ] **Step 2: Track sidebar state with a new `autocomplete_enabled` field and setter**

```rust
pub fn set_autocomplete_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
    self.autocomplete_enabled = enabled;
    cx.notify();
}
```

- [ ] **Step 3: Add a switch to the sidebar safety/input section**

```rust
Switch::new("terminal-autocomplete-switch")
    .checked(autocomplete_enabled)
    .small()
    .on_click(cx.listener(|this, checked: &bool, _window, cx| {
        this.autocomplete_enabled = *checked;
        cx.emit(SettingsPanelEvent::AutocompleteChanged(*checked));
    }))
```

- [ ] **Step 4: Add sidebar locale strings**

```yaml
autocomplete:
  en: Terminal autocomplete
  zh-CN: 终端自动补全
  zh-HK: 終端自動補全
```

### Task 5: Verify the focused behavior end to end

**Files:**
- Modify: `main/src/home/home_tabs.rs`
- Modify: `crates/terminal_view/src/view.rs`

- [ ] **Step 1: Persist the new event into `AppSettings` and sync all live terminal views**

```rust
TerminalViewEvent::AutocompleteChanged { enabled } => {
    cx.update_global::<AppSettings, _>(|s, _| {
        s.terminal_enable_autocomplete = *enabled;
        s.save();
    });
    let settings = AppSettings::global(cx).clone();
    this.apply_terminal_settings_to_all(&settings, window, cx);
}
```

- [ ] **Step 2: Run focused tests for the new helper and settings defaults**

Run: `cargo test -p terminal_view history_prompt_requires_global_autocomplete_switch -- --nocapture`
Expected: PASS

Run: `cargo test -p main app_settings_enable_terminal_autocomplete_by_default -- --nocapture`
Expected: PASS

- [ ] **Step 3: Run broader regression coverage for the touched crates**

Run: `cargo test -p terminal_view -- --nocapture`
Expected: PASS with 0 failures

Run: `cargo test -p main -- --nocapture`
Expected: PASS with 0 failures
