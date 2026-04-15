# Local History Autocomplete Fuzzy Matching Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Improve local terminal history autocomplete so short fuzzy queries like `gcm` or `gst` can surface useful history commands without introducing shell-native completion complexity.

**Architecture:** Keep the change entirely inside `crates/terminal/src/history.rs`. Extend the existing history ranking pipeline with token-initialism and inline subsequence matching, preserve current prefix-first behavior, and cover the new behavior with focused tests before implementation.

**Tech Stack:** Rust, terminal history matcher, cargo test

---

### Task 1: Add failing tests for fuzzy history suggestions

**Files:**
- Modify: `crates/terminal/src/history.rs`

- [ ] **Step 1: Add a failing test for token initialism matching**

```rust
#[test]
fn inline_suggest_matches_token_initialism() {
    let session = session_from_strings(&["git commit -m", "git checkout main"]);

    let matches = collect_history_suggestions(&session, &[], "gcm", 5);

    assert!(matches.contains(&"git commit -m".to_string()));
}
```

- [ ] **Step 2: Add a failing test for inline subsequence fallback**

```rust
#[test]
fn inline_suggest_matches_subsequence_after_stronger_strategies() {
    let session = session_from_strings(&["git status", "git stash", "cargo test"]);

    let matches = collect_history_suggestions(&session, &[], "gst", 5);

    assert!(matches.contains(&"git status".to_string()));
    assert!(matches.contains(&"git stash".to_string()));
}
```

- [ ] **Step 3: Run focused tests and verify failure**

Run: `cargo test -p terminal inline_suggest_matches_ -- --nocapture`
Expected: FAIL because the current matcher stops at prefix / token-prefix / substring.

### Task 2: Implement fuzzy matching without changing UI semantics

**Files:**
- Modify: `crates/terminal/src/history.rs`

- [ ] **Step 1: Add helper to compute token initialism**

```rust
fn token_initialism(command: &str) -> String
```

- [ ] **Step 2: Add helper to match token initialism prefixes**

```rust
fn has_token_initialism_prefix(command: &str, query: &str) -> bool
```

- [ ] **Step 3: Extend inline suggestion rank order**

```rust
prefix -> token_prefix -> token_initialism -> substring -> subsequence
```

- [ ] **Step 4: Keep search ranking aligned**

```rust
prefix -> token_prefix -> token_initialism -> substring -> subsequence
```

### Task 3: Verify the matcher end to end

**Files:**
- Modify: `crates/terminal/src/history.rs`

- [ ] **Step 1: Run focused matcher tests**

Run: `cargo test -p terminal inline_suggest_matches_ -- --nocapture`
Expected: PASS

- [ ] **Step 2: Run broader history regression tests**

Run: `cargo test -p terminal history -- --nocapture`
Expected: PASS
