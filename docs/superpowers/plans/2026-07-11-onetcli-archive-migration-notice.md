# OnetCli Archive Migration Notice Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Show an archive-and-migration notice on every OnetCli GUI launch and update repository entry documents to direct users to Navop.

**Architecture:** Add a focused `archive_notice` module that owns URLs, localized Dialog composition, and launch scheduling. Wire it after the main `Root` is created, remove the normal GUI update-check schedule, and keep historical update code intact. Add prominent bilingual archive notices to the repository entry documents.

**Tech Stack:** Rust 2024, GPUI, gpui-component Dialog, rust-i18n, Markdown.

---

### Task 1: Define the archive notice contract

**Files:**
- Create: `main/src/archive_notice.rs`
- Modify: `main/locales/main.yml`
- Test: `main/src/archive_notice.rs`

- [x] **Step 1: Add failing contract tests**

Add tests that require `NAVOP_WEBSITE_URL` to equal `https://navop.dev`, `NAVOP_GITHUB_URL` to equal `https://github.com/feigeCode/navop`, and require the source module to expose a startup scheduling function.

- [x] **Step 2: Run the focused test and confirm failure**

Run: `rtk cargo test -p main archive_notice --lib`

Expected: FAIL because the archive notice module and constants do not exist.

- [x] **Step 3: Implement the localized Dialog**

Create `archive_notice.rs` with URL constants, a `schedule_archive_notice` entry point, and a private Dialog builder. Use `window.defer` so the `Root` dialog layer is mounted before calling `window.open_dialog`. Add English, Simplified Chinese, and Traditional Chinese keys under `ArchiveNotice` in `main/locales/main.yml`.

- [x] **Step 4: Run the focused test**

Run: `rtk cargo test -p main archive_notice --lib`

Expected: PASS.

### Task 2: Wire the notice into GUI startup

**Files:**
- Modify: `main/src/main.rs`
- Test: `main/src/main.rs`

- [x] **Step 1: Add a startup source contract test**

Add a test that reads `main.rs` and asserts the GUI startup path calls `archive_notice::schedule_archive_notice` and no longer calls `update::schedule_update_check`.

- [x] **Step 2: Run the startup contract and confirm failure**

Run: `rtk cargo test -p main startup_shows_archive_notice --lib`

Expected: FAIL because the startup path still schedules update checks and does not schedule the archive notice.

- [x] **Step 3: Update the startup path**

Declare `mod archive_notice`, replace `update::schedule_update_check(window, cx)` with `archive_notice::schedule_archive_notice(window, cx)`, and retain `update::handle_update_command()` for historical command compatibility.

- [x] **Step 4: Run the startup contract**

Run: `rtk cargo test -p main startup_shows_archive_notice --lib`

Expected: PASS.

### Task 3: Mark repository documentation as archived

**Files:**
- Modify: `README.md`
- Modify: `README_CN.md`
- Modify: `CONTRIBUTING.md`

- [x] **Step 1: Add prominent archive notices**

Place the archive notice before the existing centered project introduction in both README files. State that OnetCli is archived and no longer receives features, fixes, or releases; link `https://navop.dev` and `https://github.com/feigeCode/navop` as the active successor.

- [x] **Step 2: Clarify historical installation content**

At the start of the English `Install` and Chinese `安装` sections, state that downloads are historical releases and new users should use Navop.

- [x] **Step 3: Close OnetCli contributions**

Add an archive note at the beginning of `CONTRIBUTING.md` that redirects new issues and pull requests to the Navop repository.

- [x] **Step 4: Verify document links**

Run: `rtk rg -n "navop.dev|feigeCode/navop|archived|归档" README.md README_CN.md CONTRIBUTING.md`

Expected: both Navop URLs and archive language appear in all three entry documents.

### Task 4: Verify and review the complete change

**Files:**
- Verify: `main/src/archive_notice.rs`
- Verify: `main/src/main.rs`
- Verify: `main/locales/main.yml`
- Verify: `README.md`
- Verify: `README_CN.md`
- Verify: `CONTRIBUTING.md`

- [x] **Step 1: Format Rust code**

Run: `rtk cargo fmt --all -- --check`

Expected: PASS with no formatting differences.

- [ ] **Step 2: Run focused tests**

Run: `rtk cargo test -p main archive_notice --lib`

Expected: PASS.

Run: `rtk cargo test -p main startup_shows_archive_notice --lib`

Expected: PASS.

- [ ] **Step 3: Compile the application crate**

Run: `rtk cargo check -p main`

Expected: PASS.

- [x] **Step 4: Review the diff**

Run: `rtk git diff --check`

Expected: PASS with no whitespace errors.

Run: `rtk git diff -- main/src/archive_notice.rs main/src/main.rs main/locales/main.yml README.md README_CN.md CONTRIBUTING.md`

Expected: only the approved archive notice, startup behavior, localization, and documentation changes are present.
