# Terminal History Autocomplete Recovery Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Restore terminal history autocomplete to a safe baseline by preventing stale-state replacements and dismissing tracking when input trust is lost.

**Architecture:** Keep the existing history prompt overlay and matching pipeline, but narrow inline acceptance to prefix-only suffix insertion. In `TerminalView`, move uncertain key flows to dismissed state so the local shadow buffer is never reused after non-linear editing.

**Tech Stack:** Rust, gpui, terminal_view

---

### Task 1: Lock Inline Acceptance To Safe Prefix Matches

**Files:**
- Modify: `crates/terminal_view/src/history_prompt.rs`
- Test: `crates/terminal_view/src/history_prompt.rs`

- [ ] **Step 1: Write the failing test**

Add a test proving inline mode rejects non-prefix matches instead of returning
`ReplaceLine("cargo test")` for query `test`.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p terminal_view accept_non_prefix_match_uses_replace_line -- --nocapture`
Expected: FAIL because current inline acceptance still returns `ReplaceLine`.

- [ ] **Step 3: Write minimal implementation**

Change `HistoryPromptState::accept_selected_suggestion` so inline mode:
- returns `AppendSuffix` only for true prefix matches
- returns `None` for token-prefix / substring matches
- keeps search mode replacement behavior unchanged

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p terminal_view accept_non_prefix_match_uses_replace_line -- --nocapture`
Expected: PASS after updating the assertion to the new safe behavior.

### Task 2: Dismiss Tracking On Untrusted Key Paths

**Files:**
- Modify: `crates/terminal_view/src/view.rs`
- Test: `crates/terminal_view/src/view.rs`

- [ ] **Step 1: Write the failing tests**

Add regression coverage for:
- a dismissed prompt restarting cleanly on the next printable input
- untrusted key paths using dismiss instead of hide so stale input is cleared before reuse

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p terminal_view history_prompt -- --nocapture`
Expected: FAIL because current view logic only hides the dropdown for many non-linear key paths.

- [ ] **Step 3: Write minimal implementation**

Update terminal key handling so paths that can desynchronize shell input from the local
shadow input dismiss the prompt instead of hiding it. Keep pure linear typing and
tracked backspace behavior active.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p terminal_view history_prompt -- --nocapture`
Expected: PASS

### Task 3: Verify Recovery Behavior

**Files:**
- Modify: `crates/terminal_view/src/history_prompt.rs`
- Modify: `crates/terminal_view/src/view.rs`

- [ ] **Step 1: Run focused tests**

Run: `cargo test -p terminal_view history_prompt -- --nocapture`
Expected: PASS

- [ ] **Step 2: Run compile verification**

Run: `cargo check -p terminal_view`
Expected: PASS
