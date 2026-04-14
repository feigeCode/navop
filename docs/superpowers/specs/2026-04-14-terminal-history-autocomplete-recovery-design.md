# Terminal History Autocomplete Recovery Design

## Summary

This change restores terminal history autocomplete to a safe and predictable baseline.
The current implementation keeps a local shadow copy of the shell input line inside
`HistoryPromptState` and sometimes treats it as authoritative even after terminal-side
editing has diverged. That causes missing prefixes, wrong replacements, and dropdowns
that appear against stale input.

## Scope

This design intentionally chooses a conservative recovery strategy:

- Keep history autocomplete only for trusted linear typing flows.
- Remove unsafe full-line replacement from inline history suggestion acceptance.
- Dismiss tracking when the local shadow input can no longer be trusted.
- Add regression tests for the failure modes observed in the current implementation.

Out of scope for this change:

- Synchronizing the real shell line buffer and cursor from shell integration
- Rebuilding history search UX from scratch
- Adding richer matching semantics beyond the existing history ranking

## Problem Statement

The terminal history prompt currently differs from the shared `input` component in one
critical way: the shared `input` completion system derives both query and replacement
range from the real editor state, while terminal history completion derives them from a
best-effort local shadow buffer.

That model breaks after non-linear terminal editing. Once shell-side state and local
state diverge, the system may:

- stop showing suggestions because matching runs against stale query text
- accept a suggestion by replacing the whole line with `Ctrl-U`, dropping existing prefix
- keep showing dropdown content for an input state that is no longer trustworthy

## Current Implementation

Today the terminal history autocomplete is stitched together from three separate flows:

1. `TerminalView::handle_key_event`
   - handles terminal hotkeys, history navigation, and inline suggestion accept / dismiss
   - used to also append printable keys directly into `HistoryPromptState`
2. `EntityInputHandler::replace_text_in_range -> TerminalView::commit_text`
   - this is the normal text-input path, equivalent to how the shared `input` component
     receives committed text and IME output
   - it writes text to the PTY and updates the local history prompt shadow input
3. terminal prompt lifecycle events
   - `ssh_backend` parses OSC 133 markers into `PromptStart` / `InputStart`
   - `terminal` forwards them as `TerminalModelEvent`
   - `TerminalView` resets the local history prompt shadow state when a new prompt starts

The key design mismatch versus the shared `input` component was that terminal inline
autocomplete partly trusted raw keydown events instead of only trusting the committed
text-input path.

## Confirmed Root Cause From Logs

The debug logs showed that one physical keypress was processed twice by the local shadow
state:

- `handle_key_event` logged `key="l"`
- `apply_inline_input_to_history_prompt` ran once from keydown handling
- `replace_text_in_range -> commit_text` ran for the same committed text and appended again

That produced a shadow query like `ll` after typing only `l`, so subsequent matching and
acceptance ran against the wrong prefix. This is why the dropdown sometimes disappeared
and why accepted commands could look like they had missing leading characters: the local
shadow input was already corrupted before accept logic ran.

## Chosen Approach

### 1. Trust only prefix-based acceptance in inline mode

For normal history suggestions, only prefix matches are accepted. If the selected
history entry does not start with the current query, acceptance returns `None` instead
of falling back to full-line replacement.

This prevents the most damaging symptom: commands being rewritten with missing leading
characters.

### 2. Treat non-linear edits as untrusted

When key handling enters a path where the shell input can change in ways the local
shadow model does not track reliably, the history prompt should be dismissed rather
than merely hidden. That includes non-linear cursor movement/editing and control-key
flows that mutate the shell line outside the tracked append/backspace path.

The practical rule is:

- linear printable typing and tracked backspace stay active
- anything that may desynchronize the line buffer moves to dismissed/untrusted state

### 3. Keep dropdown behavior simple

This recovery change does not attempt to preserve suggestions across uncertain state
transitions. If trust is lost, the prompt is dismissed and can restart on the next
printable input. This favors correctness over persistence.

### 4. Preserve search mode only where replacement is explicit

History search mode may still replace the line intentionally because it is an explicit
search-and-select interaction rather than inline suffix completion. The regression focus
for this change is inline autocomplete safety.

### 5. Reset shadow state from shell prompt lifecycle events

For SSH terminals, shell integration already emits OSC 133 prompt lifecycle markers.
When the backend receives `PromptStart` / `InputStart`, the event must flow through the
terminal model into `TerminalView`, and the history prompt must be dismissed so a new
prompt never reuses stale query text from the previous command line.

### 6. Use a single source of truth for printable text input

Printable text and space should be tracked only from the text-input system
(`replace_text_in_range -> commit_text`), not from raw keydown events.

That keeps terminal history autocomplete aligned with the shared `input` component:

- committed text, shifted characters, and IME output all go through one path
- keydown remains responsible for non-text control/navigation behavior
- the shadow input is appended once per committed character instead of once per event path

## Files

- Modify `crates/terminal_view/src/history_prompt.rs`
  - remove unsafe inline `ReplaceLine` fallback
  - add tests for rejecting non-prefix inline acceptance
- Modify `crates/terminal_view/src/view.rs`
  - dismiss prompt instead of merely hiding it on untrusted key paths
  - defer printable text tracking to `commit_text`
  - add regression tests for post-dismiss restart behavior

## Validation

- `cargo test -p terminal_view history_prompt -- --nocapture`
- `cargo check -p terminal_view`

## Risks

- This change is intentionally conservative, so autocomplete may appear less often than
  the redesign intended.
- Some keyboard paths may still need later refinement if additional shell-edit actions
  are found during manual testing.

## Follow-up

The durable long-term fix is to expose the real shell line buffer and cursor through
shell integration, then derive query text and replacement range from that authoritative
state. That is explicitly deferred from this recovery change.
