# RDP Text Clipboard Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add bidirectional plain-text clipboard synchronization between local GPUI/onelcli and the active RDP session.

**Architecture:** Use IronRDP CLIPRDR rather than keyboard text simulation. The helper owns a text-only clipboard backend and bridges CLIPRDR messages to the existing JSON helper protocol; the GPUI view reads/writes local text clipboard while focused.

**Tech Stack:** Rust, GPUI clipboard APIs, IronRDP `cliprdr`, existing `remote_desktop` helper JSON protocol.

---

### Task 1: Protocol and Core Model

**Files:**
- Modify: `crates/remote_desktop/src/input.rs`
- Modify: `crates/remote_desktop/src/output.rs`
- Modify: `crates/remote_desktop/src/helper_protocol.rs`
- Modify: `tools/rdp-helper/src/protocol.rs`
- Test: existing unit tests in the same files

- [x] Add failing tests for `ClipboardText` request/event JSON and remote input/output conversion.
- [x] Add `RemoteDesktopInput::ClipboardText { text }` and `RemoteDesktopOutput::ClipboardText { text }`.
- [x] Add `HelperRequest::ClipboardText { text }` and `HelperEvent::ClipboardText { text }`.
- [x] Update RDP capability conversion so `clipboard_text` is true for connected RDP sessions.

### Task 2: Helper CLIPRDR Backend

**Files:**
- Create: `tools/rdp-helper/src/clipboard.rs`
- Modify: `tools/rdp-helper/src/rdp.rs`
- Modify: `tools/rdp-helper/src/main.rs`
- Modify: `tools/rdp-helper/Cargo.toml`

- [x] Add failing tests for advertising `CF_UNICODETEXT`, replying with `FormatDataResponse::new_unicode_string`, and decoding remote `FormatDataResponse::to_unicode_string`.
- [x] Implement a text-only `CliprdrBackend` and `CliprdrBackendFactory`.
- [x] Enable `cliprdr_factory` in `RdpClient`.
- [x] On `HelperRequest::ClipboardText`, advertise local text to the remote.
- [x] On remote text copy, emit `HelperEvent::ClipboardText`.

### Task 3: GPUI View Bridge

**Files:**
- Modify: `crates/remote_desktop_view/src/view.rs`

- [x] Read local text clipboard while the RDP view is focused and send changes as `RemoteDesktopInput::ClipboardText`.
- [x] Write remote `RemoteDesktopOutput::ClipboardText` into GPUI clipboard.
- [x] Keep existing key/mouse capture behavior unchanged.

### Task 4: Verification

**Commands:**
- `cargo test --manifest-path tools/rdp-helper/Cargo.toml -- --nocapture`
- `cargo test -p remote_desktop -- --nocapture`
- `cargo test -p remote_desktop_view -- --nocapture`
- `CLANG_MODULE_CACHE_PATH=/tmp/clang-cache cargo check -p remote_desktop -p remote_desktop_view -p main`
- `cargo build --manifest-path tools/rdp-helper/Cargo.toml`
- `cargo fmt --manifest-path tools/rdp-helper/Cargo.toml`
- `cargo fmt -p remote_desktop -p remote_desktop_view`
- `git diff --check`

