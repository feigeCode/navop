# Settings Sidebar Scroll Alignment Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make a settings sidebar group click align the corresponding settings block to the top of the right-hand scrollable list.

**Architecture:** Preserve the existing deferred group-index request stored in `SettingsState`. Convert that index into a zero-offset GPUI `ListOffset` through a small pure helper, then pass the result to `ListState::scroll_to`; test the helper directly so the navigation contract cannot regress to reveal-only scrolling.

**Tech Stack:** Rust, GPUI variable-height `ListState`, `gpui-component` unit tests.

---

### Task 1: Protect the top-alignment contract with a regression test

**Files:**
- Modify: `crates/ui/src/setting/page.rs`
- Test: `crates/ui/src/setting/page.rs`

- [ ] **Step 1: Add a failing unit test for the navigation offset**

Add a private test module and assert that the planned helper preserves the requested group index and produces a zero item offset:

```rust
#[cfg(test)]
mod tests {
    use super::settings_group_list_offset;
    use gpui::px;

    #[test]
    fn settings_group_list_offset_aligns_group_to_top() {
        let offset = settings_group_list_offset(7);

        assert_eq!(7, offset.item_ix);
        assert_eq!(px(0.), offset.offset_in_item);
    }
}
```

- [ ] **Step 2: Run the test and verify that it fails for the missing helper**

Run:

```bash
rtk cargo test -p gpui-component settings_group_list_offset_aligns_group_to_top
```

Expected: compilation fails because `settings_group_list_offset` does not exist yet.

### Task 2: Implement exact top alignment

**Files:**
- Modify: `crates/ui/src/setting/page.rs`

- [ ] **Step 1: Import `ListOffset` and add the pure offset helper**

Extend the GPUI imports with `ListOffset`, then add:

```rust
fn settings_group_list_offset(group_ix: usize) -> ListOffset {
    ListOffset {
        item_ix: group_ix,
        offset_in_item: px(0.),
    }
}
```

- [ ] **Step 2: Replace reveal-only scrolling with exact offset scrolling**

Change the deferred scroll consumer from:

```rust
list_state.scroll_to_reveal_item(ix);
```

to:

```rust
list_state.scroll_to(settings_group_list_offset(ix));
```

Do not modify sidebar selection, search filtering, page headers, list layout, or any editor behavior.

- [ ] **Step 3: Run the focused regression test**

Run:

```bash
rtk cargo test -p gpui-component settings_group_list_offset_aligns_group_to_top
```

Expected: one matching test passes with zero failures.

### Task 3: Verify the affected component and application

**Files:**
- Verify: `crates/ui/src/setting/page.rs`

- [ ] **Step 1: Format the changed Rust file**

Run:

```bash
rtk cargo fmt --all -- --check
```

If formatting is required, run `rtk cargo fmt --all`, then re-run the check.

- [ ] **Step 2: Run the full UI component test suite**

Run:

```bash
rtk cargo test -p gpui-component
```

Expected: all `gpui-component` tests pass.

- [ ] **Step 3: Compile the main application**

Run:

```bash
rtk cargo check -p main
```

Expected: zero compilation errors; pre-existing warnings may remain.

- [ ] **Step 4: Check the patch and repository status**

Run:

```bash
rtk git diff --check
rtk git status --short
```

Expected: no whitespace errors; only this task's intended implementation/test files and already committed design/plan history are involved.

- [ ] **Step 5: Commit the implementation**

Run:

```bash
rtk git add crates/ui/src/setting/page.rs
rtk git commit -m "fix(settings): align sidebar navigation to group top"
```

Expected: the commit contains only the settings page implementation and regression test.
