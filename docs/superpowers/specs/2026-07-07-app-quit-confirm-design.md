# App Quit Confirmation Design

## Goal

Prevent accidental application exits by showing an application-level confirmation dialog before quitting, while guaranteeing that every closable work tab still goes through the existing `TabContent::try_close()` contract.

## Non-Goals

- Do not replace per-tab close confirmation logic.
- Do not add new unsaved-state APIs to business tabs.
- Do not force-close tabs during normal application quit.
- Do not change update-install quit behavior that already follows an explicit update flow.
- Do not persist quit-confirmation preferences in this phase.

## Requirements

- Cmd-Q on macOS and Alt-F4 on non-macOS must no longer call `cx.quit()` directly.
- Main-window close requests must be intercepted before the window is removed.
- Linux custom title-bar close control must enter the same quit request path as keyboard quit.
- The first quit request opens one application-level confirmation dialog.
- Repeated quit requests while the confirmation dialog is open or tab closing is already in progress must not open duplicate dialogs or start concurrent close tasks.
- Confirming the dialog must close all regular work tabs through `TabContainer::close_all_tabs()`.
- Split panes must be included. Tabs in secondary panes must not be skipped.
- If any tab's `try_close()` returns `false`, application quit is canceled and the main window remains open.
- If all closable work tabs approve closing, the app quits.
- Pinned tabs such as Home do not block quit.
- Existing per-tab confirmations remain authoritative for dirty data, running sessions, or tab-specific close rules.

## User Experience

When the user requests application quit, show a modal confirmation dialog:

- Title: "Quit application?"
- Message: "All tabs will be closed. Some tabs may ask for additional confirmation before quitting."
- Confirm button: "Quit"
- Cancel button: existing common cancel text.

Canceling the dialog leaves the app unchanged. Confirming starts the tab close sequence. During that sequence, per-tab dialogs may appear in the order selected by the close traversal. If the user cancels any per-tab confirmation, the quit attempt stops.

The application-level confirmation is intentionally shown for every normal app quit request. This protects against accidental Cmd-Q, Alt-F4, or title-bar close even when no dirty tab is currently known.

## Architecture

The quit flow becomes:

```text
QuitApp action
  -> OnetCliApp::request_quit()
     -> confirmation dialog
        -> OnetCliApp::confirm_quit()
           -> SplitTabContainer::close_all_tabs()
              -> each TabContainer::close_all_tabs()
                 -> each TabContent::try_close()
           -> cx.quit() only if every close task returns true

Window should-close callback
  -> OnetCliApp::request_quit()
  -> returns false to prevent direct close

Linux custom close button
  -> OnetCliApp::request_quit()
```

`OnetCliApp` owns application-level quit state:

```rust
pub struct OnetCliApp {
    split_container: Entity<SplitTabContainer>,
    quit_prompt_open: bool,
    quit_in_progress: bool,
}
```

`quit_prompt_open` prevents duplicate confirmation dialogs. `quit_in_progress` prevents concurrent tab-close tasks after the user confirms quit.

`SplitTabContainer` gains a public close orchestrator:

```rust
pub fn close_all_tabs(
    &mut self,
    window: &mut Window,
    cx: &mut Context<Self>,
) -> Task<bool>
```

The method collects all current panes from the split tree in a stable left-to-right, top-to-bottom traversal and calls each pane's existing `TabContainer::close_all_tabs()` in order. It returns `false` immediately when any pane returns `false`.

`TabContainer::close_all_tabs()` keeps its current role. It activates each regular closable tab, calls that tab content's `try_close()`, removes the tab only after approval, and returns `false` when a tab refuses to close.

## Entry Points

### Keyboard Quit

`quit_app(cx)` changes from direct `cx.quit()` to a request routed through the active main window:

1. Get the active window.
2. Get the active `OnetCliApp` view or global quit controller.
3. Defer into the window context.
4. Call `request_quit(window, cx)`.

If no active window exists, the function may fall back to `cx.quit()` because no tab container can be consulted.

### Main Window Close

`OnetCliApp::new` registers `window.on_window_should_close()` for the main window. The callback calls `request_quit()` and returns `false`. This ensures system close requests do not bypass tab close checks.

### Linux Custom Close

The primary `TabContainer` already receives custom Linux window controls. Its close button currently calls `window.remove_window()` directly. This button should call the same quit request path used by `QuitApp`, so Linux client-side controls behave like system quit.

## Dialog Behavior

The dialog uses the existing `window.open_dialog()` and `Dialog::confirm()` pattern. It should reuse `DialogButtonProps` to set the confirm and cancel labels.

The confirm callback:

1. Clears `quit_prompt_open`.
2. Calls `confirm_quit(window, cx)`.
3. Returns `true` so the application-level dialog closes before any per-tab confirmation appears.

The cancel callback clears `quit_prompt_open` and returns `true`.

If the dialog is closed through Escape or any close affordance, the close handler also clears `quit_prompt_open`.

## Failure And Cancellation

If any tab rejects close:

- Do not call `cx.quit()`.
- Reset `quit_in_progress` so the user can request quit again.
- Leave already closed tabs closed. This matches the existing sequential `close_all_tabs()` behavior.
- Leave the rejecting tab or its pane active when possible, because `TabContainer::close_all_tabs()` activates each tab before asking it to close.

If the window or app context is no longer available while the async close task resolves, the task should stop without panicking.

## Localization

Add main locale keys under an application quit namespace:

```yaml
Quit:
  confirm_title:
    en: Quit application?
    zh-CN: 退出应用？
    zh-HK: 結束應用程式？
  confirm_message:
    en: All tabs will be closed. Some tabs may ask for additional confirmation before quitting.
    zh-CN: 将关闭所有标签页，部分标签页可能会在退出前要求再次确认。
    zh-HK: 將關閉所有標籤頁，部分標籤頁可能會在結束前要求再次確認。
  confirm_action:
    en: Quit
    zh-CN: 退出
    zh-HK: 結束
```

Use existing `Common.cancel` for the cancel button.

## Testing

Add targeted regression coverage for the quit orchestration.

`SplitTabContainer` tests should verify:

- Closing all tabs includes secondary panes.
- A rejecting tab makes the full close task return `false`.
- Closing stops after the first rejection.
- An empty split layout or panes with no regular closable tabs return `true`.

`OnetCliApp` tests should verify either through GPUI window tests or a small extracted state helper:

- A quit request opens at most one confirmation dialog.
- A quit request while `quit_in_progress` is true does not start another close task.
- A failed tab-close sequence resets `quit_in_progress`.
- The `QuitApp` handler no longer calls `cx.quit()` directly.

Suggested verification commands:

```bash
rtk cargo test -p one-core split_tab_container
rtk cargo test -p main quit
rtk cargo check -p main
```

## Implementation Notes

- Keep the change local to `main/src/onetcli_app.rs`, `crates/core/src/split_tab_container.rs`, and `main/locales/main.yml` unless tests require small helpers.
- Avoid changing business tab implementations. Their existing `try_close()` methods are the source of truth.
- Do not use `force_close_tab_by_id()` in the app quit path.
- Do not route app quit only through `GlobalTabContainer`, because that global currently points at the primary pane and would miss secondary split panes.
