# RDP Reconnect And Resize Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add RDP auto reconnect, visible reconnect status, manual reconnect, and remote desktop resize when the main app content area changes size.

**Architecture:** Keep reconnect orchestration in `remote_desktop::backends::rdp`, because that layer owns the helper process. The GPUI view sends control inputs (`Reconnect`, `Resize`) and renders a lightweight status overlay; helper protocol remains responsible for RDP input events.

**Tech Stack:** Rust, std process management, existing helper JSON protocol, GPUI view rendering.

---

### Task 1: Core Input And Helper Conversion

**Files:**
- Modify: `crates/remote_desktop/src/input.rs`
- Modify: `crates/remote_desktop/src/helper_protocol.rs`

- [x] Add a failing test showing `RemoteDesktopInput::Reconnect` is consumed by the RDP backend and is not serialized to helper JSON.
- [x] Add the `Reconnect` input variant.
- [x] Keep `Close` serializable so helper shutdown still works.

### Task 2: RDP Backend Reconnect Loop

**Files:**
- Modify: `crates/remote_desktop/src/backends/rdp.rs`

- [x] Add failing tests for reconnect delay calculation and helper disconnect signal detection.
- [x] Add an internal backend signal channel from stdout reader to process loop.
- [x] Restart helper on `ConnectionFailure`, `Terminated`, stdout EOF, or manual `Reconnect`.
- [x] Do not reconnect after user `Close`.
- [x] Preserve latest `Resize` and `ClipboardText` across reconnect.

### Task 3: View Status And Resize Sync

**Files:**
- Modify: `crates/remote_desktop_view/src/view.rs`

- [x] Add failing tests for converting content bounds to clamped RDP resize dimensions.
- [x] Send `RemoteDesktopInput::Resize` when content bounds change.
- [x] Debounce automatic resize, wait for an established remote size before sending, and normalize dimensions to RDP DisplayControl limits.
- [x] Show a small status overlay whenever status is not `Connected`.
- [x] Clicking the status overlay sends `RemoteDesktopInput::Reconnect`.
- [x] Clear stale modifier state on reconnect-triggering terminal statuses.

### Task 4: Verification

**Commands:**
- `cargo test -p remote_desktop -- --nocapture`
- `cargo test -p remote_desktop_view -- --nocapture`
- `cargo test --manifest-path tools/rdp-helper/Cargo.toml -- --nocapture`
- `CLANG_MODULE_CACHE_PATH=/tmp/clang-cache cargo check -p remote_desktop -p remote_desktop_view -p main`
- `cargo build --manifest-path tools/rdp-helper/Cargo.toml`
- `cargo fmt --manifest-path tools/rdp-helper/Cargo.toml`
- `cargo fmt -p remote_desktop -p remote_desktop_view`
- `git diff --check`
