# Terminal History Prompt Stability Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make terminal history suggestions stable by showing inline hints only in trusted input states and removing unsafe full-line replacement.

**Architecture:** Keep the current overlay UI, but change `HistoryPromptState` from a mutable shell-line shadow model into a safer query-and-selection model. `TerminalView` will explicitly suspend history tracking on non-linear editing and mouse interactions, and accept suggestions only by appending a verified suffix.

**Tech Stack:** Rust, gpui, terminal_view, terminal

---

### Task 1: Stabilize Prompt State Machine

**Files:**
- Modify: `crates/terminal_view/src/history_prompt.rs`
- Test: `crates/terminal_view/src/view.rs`

- [ ] **Step 1: Write failing tests**

Add tests covering:
- suspended state hides matches and ignores further typed characters until reset
- navigation changes selected suggestion without mutating the tracked query
- accepting a clicked or navigated suggestion only returns the suffix beyond the original query

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p terminal_view history_prompt -- --nocapture`
Expected: FAIL because the current state object mutates `input` during navigation and automatically resumes after invalidation.

- [ ] **Step 3: Write minimal implementation**

Change `HistoryPromptState` to:
- keep `input` as the trusted query buffer
- track whether syncing is suspended
- keep suggestion selection independent from `input`
- only mutate `input` when a suggestion is actually accepted

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p terminal_view history_prompt -- --nocapture`
Expected: PASS

### Task 2: Remove Unsafe Terminal Line Replacement

**Files:**
- Modify: `crates/terminal_view/src/view.rs`
- Test: `crates/terminal_view/src/view.rs`

- [ ] **Step 1: Write failing tests**

Add tests covering:
- browsing suggestions does not overwrite the tracked query
- dismiss keeps the typed prefix intact

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p terminal_view history_prompt -- --nocapture`
Expected: FAIL because browsing currently replaces `input` and `TerminalView` relies on full-line replacement behavior.

- [ ] **Step 3: Write minimal implementation**

In `TerminalView`:
- suspend tracking on non-linear keys and mouse interactions
- make Up/Down only move the selected suggestion in the overlay
- make click, Right, and Tab accept suggestions by suffix append only
- stop using `Ctrl-U` full-line replacement for history selection
- only record local commands on Enter when the tracked input is trusted

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p terminal_view history_prompt -- --nocapture`
Expected: PASS

### Task 3: Verify Integration

**Files:**
- Modify: `crates/terminal_view/src/view.rs`
- Modify: `crates/terminal_view/src/history_prompt.rs`

- [ ] **Step 1: Run focused terminal_view tests**

Run: `cargo test -p terminal_view history_prompt -- --nocapture`
Expected: PASS with all new and existing history prompt tests green.

- [ ] **Step 2: Run compile verification**

Run: `cargo check -p terminal_view`
Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add crates/terminal_view/src/history_prompt.rs crates/terminal_view/src/view.rs docs/superpowers/plans/2026-04-11-terminal-history-prompt-stability.md
git commit -m "fix(terminal_view): stabilize history prompt interactions"
```
