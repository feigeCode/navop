# Page-Local Large Text Editor Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move the large-text cell preview/editor from the database-level shared sidebar into each table/result page while keeping dialog mode available.

**Architecture:** Add a page-local host component that owns a `DataGrid` plus a right-side `CellPreviewPanel`. `DataGrid` only emits local toggle events in preview mode. Database-level sidebar remains AI-only. SQL result tabs and table data tabs both render the new host.

**Tech Stack:** Rust, gpui, gpui_component, existing `MultiTextEditor` / `CellPreviewPanel`

---

### Task 1: Add routing tests for large-text open behavior

**Files:**
- Modify: `crates/db_view/src/table_data/data_grid.rs`

- [ ] Add focused tests for preview-vs-dialog routing helpers.
- [ ] Run the targeted `db_view` tests and confirm the new tests fail for the missing helper/behavior.

### Task 2: Add a page-local preview host

**Files:**
- Create: `crates/db_view/src/table_data/cell_preview_host.rs`
- Modify: `crates/db_view/src/table_data/mod.rs`
- Modify: `crates/db_view/src/sidebar/cell_preview_panel.rs`

- [ ] Create a reusable host entity that owns a `DataGrid`, a `CellPreviewPanel`, open state, and local subscriptions.
- [ ] Reuse the existing panel logic but expose the module so page-level code can use it directly.
- [ ] Keep blur-based writeback behavior unchanged.

### Task 3: Switch `DataGrid` from global sidebar events to local page events

**Files:**
- Modify: `crates/db_view/src/table_data/data_grid.rs`

- [ ] Replace the global notifier trigger with a local event emitted from `DataGrid`.
- [ ] Keep dialog mode unchanged.
- [ ] Preserve toolbar highlight state through the existing `set_large_text_editor_sidebar_open` flag.

### Task 4: Mount the page-local host in table data and SQL result tabs

**Files:**
- Modify: `crates/db_view/src/table_data_tab.rs`
- Modify: `crates/db_view/src/sql_result_tab.rs`

- [ ] Render `TableDataTabContent` through the new host instead of the raw `DataGrid`.
- [ ] Flush pending preview edits before `TableDataTabContent::try_close`.
- [ ] Store/render page-local hosts for SQL result tabs so each result tab owns its own preview state.

### Task 5: Remove shared-sidebar cell editor wiring

**Files:**
- Modify: `crates/db_view/src/sidebar/mod.rs`
- Delete: `crates/db_view/src/sidebar/cell_editor_notifier.rs`
- Modify: `crates/db_view/src/lib.rs`
- Modify: `main/src/main.rs`

- [ ] Remove the cell-preview panel and notifier from the database-level sidebar.
- [ ] Keep AI sidebar behavior intact.
- [ ] Remove obsolete initialization/export paths.

### Task 6: Verify and format

**Files:**
- Modify: touched files above

- [ ] Run `rustfmt --edition 2024` on changed Rust files.
- [ ] Run targeted `cargo test -p db_view ...` for the new/changed tests.
- [ ] Run `cargo check -p db_view`.
