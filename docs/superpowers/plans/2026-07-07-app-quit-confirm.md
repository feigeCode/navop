# App Quit Confirmation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an application-level quit confirmation and route confirmed app quit through every pane's `TabContainer::close_all_tabs()` so each tab's `try_close()` contract is respected.

**Architecture:** `OnetCliApp` owns the quit prompt/in-progress state and handles `QuitApp`, main-window should-close, and Linux custom close control through one `request_quit` path. `SplitTabContainer` owns cross-pane close orchestration and delegates each pane to existing `TabContainer::close_all_tabs()`.

**Tech Stack:** Rust 2024, GPUI, gpui-component dialogs, `one-core` tab/split container APIs, rust-i18n locale YAML.

---

### Task 1: Quit State Contract

**Files:**
- Modify: `main/src/onetcli_app.rs`

- [x] **Step 1: Write failing tests for the pure quit-state helper**

Add a small helper enum and tests before implementation:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum QuitRequestDecision {
    OpenPrompt,
    Ignore,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct QuitRequestState {
    prompt_open: bool,
    in_progress: bool,
}

#[cfg(test)]
mod tests {
    #[test]
    fn quit_state_opens_prompt_for_first_request() {
        let mut state = super::QuitRequestState::default();
        assert_eq!(super::QuitRequestDecision::OpenPrompt, state.request());
        assert!(state.prompt_open);
    }

    #[test]
    fn quit_state_ignores_duplicate_prompt_and_in_progress_requests() {
        let mut prompt_state = super::QuitRequestState::default();
        prompt_state.prompt_open = true;
        assert_eq!(super::QuitRequestDecision::Ignore, prompt_state.request());

        let mut running_state = super::QuitRequestState::default();
        running_state.in_progress = true;
        assert_eq!(super::QuitRequestDecision::Ignore, running_state.request());
    }

    #[test]
    fn quit_state_resets_after_cancel_or_failed_close() {
        let mut state = super::QuitRequestState {
            prompt_open: true,
            in_progress: false,
        };
        state.cancel_prompt();
        assert_eq!(super::QuitRequestState::default(), state);

        state.prompt_open = true;
        state.confirm_prompt();
        assert!(state.in_progress);
        state.finish_close(false);
        assert_eq!(super::QuitRequestState::default(), state);
    }
}
```

- [x] **Step 2: Run test and verify RED**

Run:

```bash
rtk cargo test -p main quit_state
```

Expected: fails because `QuitRequestState::request`, `cancel_prompt`, `confirm_prompt`, and `finish_close` are not implemented.

- [x] **Step 3: Implement the helper minimally**

Add methods:

```rust
impl QuitRequestState {
    fn request(&mut self) -> QuitRequestDecision {
        if self.prompt_open || self.in_progress {
            return QuitRequestDecision::Ignore;
        }
        self.prompt_open = true;
        QuitRequestDecision::OpenPrompt
    }

    fn cancel_prompt(&mut self) {
        self.prompt_open = false;
    }

    fn confirm_prompt(&mut self) -> bool {
        if self.in_progress {
            return false;
        }
        self.prompt_open = false;
        self.in_progress = true;
        true
    }

    fn finish_close(&mut self, closed: bool) {
        if !closed {
            self.in_progress = false;
        }
    }
}
```

- [x] **Step 4: Run GREEN**

Run:

```bash
rtk cargo test -p main quit_state
```

Expected: the three `quit_state_*` tests pass.

### Task 2: Split Pane Close Orchestration

**Files:**
- Modify: `crates/core/src/split_tab_container.rs`
- Modify: `crates/core/src/tab_container_split_tests.rs`

- [x] **Step 1: Write failing pure traversal tests**

Add tests that define and verify a pure traversal helper:

```rust
#[test]
fn split_close_order_visits_all_leaf_panes_left_to_right() {
    let tree = test_tree(vec![
        SplitCloseTestNode::Leaf("primary"),
        SplitCloseTestNode::Split(vec![
            SplitCloseTestNode::Leaf("right_top"),
            SplitCloseTestNode::Leaf("right_bottom"),
        ]),
    ]);
    assert_eq!(
        vec!["primary", "right_top", "right_bottom"],
        split_close_leaf_order(&tree)
    );
}

#[test]
fn split_close_sequence_stops_after_first_rejection() {
    assert_eq!(
        (false, vec!["primary", "right"]),
        close_sequence_until_rejected(vec![
            ("primary", true),
            ("right", false),
            ("skipped", true),
        ])
    );
}
```

- [x] **Step 2: Run test and verify RED**

Run:

```bash
rtk cargo test -p one-core split_close
```

Expected: fails because the traversal/sequence helpers do not exist.

- [x] **Step 3: Implement traversal and orchestration**

Implement:

```rust
impl SplitNode {
    fn collect_panes(&self, panes: &mut Vec<Entity<TabContainer>>) {
        match self {
            SplitNode::Leaf(pane) => panes.push(pane.clone()),
            SplitNode::Split { children, .. } => {
                for child in children {
                    child.collect_panes(panes);
                }
            }
        }
    }
}

impl SplitTabContainer {
    pub fn close_all_tabs(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<bool> {
        let mut panes = Vec::new();
        self.root.collect_panes(&mut panes);
        let window_id = cx.active_window();

        cx.spawn(async move |_handle, cx| {
            for pane in panes {
                let task = cx.update_window(window_id.expect("No active window"), |_, window, cx| {
                    pane.update(cx, |pane, cx| pane.close_all_tabs(window, cx))
                });
                match task {
                    Ok(task) if task.await => {}
                    Ok(_) | Err(_) => return false,
                }
            }
            true
        })
    }
}
```

Keep helper tests focused on deterministic order and early stop. Do not duplicate `TabContainer::close_all_tabs()` tests.

- [x] **Step 4: Run GREEN**

Run:

```bash
rtk cargo test -p one-core split_close
```

Expected: new split close tests pass.

### Task 3: Main App Quit Wiring

**Files:**
- Modify: `main/src/onetcli_app.rs`
- Modify: `crates/core/src/tab_container.rs`
- Modify: `main/locales/main.yml`

- [x] **Step 1: Write failing structural tests**

Add tests in `main/src/onetcli_app.rs`:

```rust
#[test]
fn quit_action_does_not_call_cx_quit_directly() {
    let source = include_str!("onetcli_app.rs");
    let quit_fn = function_source(source, "fn quit_app");
    assert!(!quit_fn.contains("cx.quit()"));
    assert!(quit_fn.contains("request_active_window_quit"));
}

#[test]
fn onetcli_app_registers_window_close_guard() {
    let source = include_str!("onetcli_app.rs");
    assert!(source.contains("on_window_should_close"));
    assert!(source.contains("request_quit"));
}
```

Add a structural test in `crates/core/src/tab_container.rs` or existing split/tab tests to assert the close control has an injected callback path:

```rust
#[test]
fn linux_close_control_uses_injected_window_close_callback() {
    let source = include_str!("tab_container.rs");
    assert!(source.contains("on_close_window"));
    assert!(source.contains("with_window_close_action"));
}
```

- [x] **Step 2: Run tests and verify RED**

Run:

```bash
rtk cargo test -p main quit_action
rtk cargo test -p one-core linux_close_control
```

Expected: tests fail because direct quit and direct Linux close are still present.

- [x] **Step 3: Implement app quit request path**

Add `quit_state: QuitRequestState` to `OnetCliApp`.

Add methods:

```rust
fn request_active_window_quit(cx: &mut App) {
    let Some(active_window) = cx.active_window() else {
        cx.quit();
        return;
    };
    let Some(app) = cx.try_global::<GlobalOnetCliApp>().map(|global| global.app.clone()) else {
        cx.quit();
        return;
    };
    cx.defer(move |cx| {
        let _ = active_window.update(cx, |_, window, cx| {
            app.update(cx, |app, cx| {
                app.request_quit(window, cx);
            });
        });
    });
}

fn request_quit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if self.quit_state.request() == QuitRequestDecision::OpenPrompt {
        self.show_quit_confirmation(window, cx);
    }
}

fn show_quit_confirmation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let app = cx.entity().downgrade();
    window.open_dialog(cx, move |dialog, _window, _cx| {
        let app = app.clone();
        dialog
            .title(t!("Quit.confirm_title").to_string())
            .child(t!("Quit.confirm_message").to_string())
            .confirm()
            .on_ok(move |_, window, cx| {
                let _ = app.update(cx, |app, cx| app.confirm_quit(window, cx));
                true
            })
    });
}

fn confirm_quit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if !self.quit_state.confirm_prompt() {
        return;
    }
    let close_task = self
        .split_container
        .update(cx, |split, cx| split.close_all_tabs(window, cx));
    cx.spawn(async move |this, cx| {
        let can_quit = close_task.await;
        let _ = this.update(cx, |app, cx| {
            app.quit_state.finish_close(can_quit);
            if can_quit {
                cx.quit();
            }
        });
    })
    .detach();
}
```

`show_quit_confirmation` uses `window.open_dialog(cx, move |dialog, _window, _cx| { dialog })`, `.confirm()`, `DialogButtonProps::default().ok_text(t!("Quit.confirm_action").to_string()).cancel_text(t!("Common.cancel").to_string())`, and the new locale keys.

`confirm_quit` runs `self.split_container.update(cx, |split, cx| split.close_all_tabs(window, cx))`, awaits the task, and calls `cx.quit()` only when it resolves to `true`.

- [x] **Step 4: Wire entry points**

Update:

```rust
fn quit_app(cx: &mut App) {
    request_active_window_quit(cx);
}
```

Register `window.on_window_should_close()` in `OnetCliApp::new`, calling `request_quit` and returning `false`.

Pass a Linux close callback into primary `TabContainer` via a new `with_window_close_action` builder.

- [x] **Step 5: Add locale keys**

Add `Quit.confirm_title`, `Quit.confirm_message`, and `Quit.confirm_action` to `main/locales/main.yml`.

- [x] **Step 6: Run GREEN**

Run:

```bash
rtk cargo test -p main quit
rtk cargo test -p one-core linux_close_control
```

Expected: new wiring tests pass.

### Task 4: Final Verification

**Files:**
- Verify all files touched in Tasks 1-3.

- [x] **Step 1: Format**

Run:

```bash
rtk cargo fmt
```

- [x] **Step 2: Targeted tests**

Run:

```bash
rtk cargo test -p one-core split_close
rtk cargo test -p main quit
```

- [x] **Step 3: Compile main app**

Run:

```bash
rtk cargo check -p main
```

- [x] **Step 4: Inspect diff**

Run:

```bash
rtk git diff --stat
rtk git diff
```

Confirm the implementation only touches the quit confirmation feature, locale text, and tests.
