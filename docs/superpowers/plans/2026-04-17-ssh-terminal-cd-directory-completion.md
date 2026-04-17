# SSH Terminal CD Directory Completion Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make SSH terminal autocomplete show remote child directories when the user types `cd ` and keep filtering as they continue typing a path prefix.

**Architecture:** Add a dedicated `cd` directory completion helper in `terminal_view` for parsing the current shell input and building shell-safe directory suggestions, then wire `TerminalView` to fetch remote directory entries through a lazy SFTP client and feed those suggestions into the existing autocomplete dropdown instead of history suggestions when the current input is a supported `cd` path query.

**Tech Stack:** Rust, terminal_view, sftp, ssh

---

### Task 1: Lock down `cd` parsing and suggestion formatting with failing tests

**Files:**
- Create: `crates/terminal_view/src/cd_completion.rs`
- Modify: `crates/terminal_view/src/lib.rs`

- [ ] **Step 1: Add tests that parse `cd ` into a completion query rooted at the current SSH working directory**
- [ ] **Step 2: Add tests that parse `../` and absolute paths into the correct parent directory and typed prefix**
- [ ] **Step 3: Add tests that format directory suggestions with prefix filtering, trailing `/`, and shell escaping for spaces**

### Task 2: Implement pure `cd` completion helpers

**Files:**
- Create: `crates/terminal_view/src/cd_completion.rs`
- Modify: `crates/terminal_view/src/lib.rs`

- [ ] **Step 1: Add a pure parser that recognizes supported `cd` input forms and resolves the remote parent directory**
- [ ] **Step 2: Add pure suggestion builders that filter directory names by typed prefix and produce shell-safe `cd .../` candidates**
- [ ] **Step 3: Export the helper module for `TerminalView`**

### Task 3: Connect SSH directory completion into the existing dropdown

**Files:**
- Modify: `crates/terminal_view/src/view.rs`

- [ ] **Step 1: Add lazy SFTP client and per-parent directory cache state to `TerminalView`**
- [ ] **Step 2: Make autocomplete refresh choose remote `cd` directory suggestions for inline SSH `cd` input and keep history suggestions for all other cases**
- [ ] **Step 3: Fetch remote directory entries asynchronously, discard stale responses, and reuse the existing dropdown accept/navigation flow**

### Task 4: Verify targeted coverage and regressions

- [ ] **Step 1: Run targeted tests**

Run: `cargo test -p terminal_view cd_completion -- --nocapture`
Expected: PASS

- [ ] **Step 2: Run terminal view regression coverage**

Run: `cargo test -p terminal_view -- --nocapture`
Expected: PASS
