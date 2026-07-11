# Remote Mutation Sidebar Refresh Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans and superpowers:test-driven-development.

**Goal:** Refresh the current SFTP remote directory after every successful remote mutation without changing the current path.

**Architecture:** `remote_file_editor` exposes an erased success callback carried by built-in and external editor requests. SFTP callers provide a callback capturing their weak view entity. SFTP-native mutations use a single refresh helper, with batch operations refreshing once on completion.

## Tasks

- [x] Add a callback contract and a failing invocation-count test.
- [x] Carry the callback through external editor launch/controller and invoke after successful upload.
- [x] Carry the callback through the built-in editor and invoke after successful save.
- [x] Update SFTP and terminal-sidebar call sites with weak-entity callbacks.
- [x] Consolidate editor-originated successful remote mutation refresh behind one helper per caller.
- [x] Verify cancellation/failure paths do not invoke the success callback and current path is preserved.
- [x] Run focused tests, `cargo check -p main`, formatting and diff checks.
- [ ] Review, commit, rebuild the worktree app and manually verify sidebar refresh.
