# External File Monitoring Auto Upload Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans and superpowers:test-driven-development.

**Goal:** Detect external-editor disk writes through file events plus polling and optionally upload them to the remote file.

**Architecture:** A default-on Host setting controls creation of the existing upload controller. The controller stores the last successfully synchronized local content fingerprint; notify events provide fast detection and a periodic poll provides recovery from missed events. Unchanged polls stop before remote I/O.

## Tasks

- [x] Add default-on `auto_upload_external_changes` settings tests and model field.
- [x] Add the settings checkbox and three-locale text.
- [x] Add content-fingerprint RED/GREEN coverage.
- [x] Store the initial and last synchronized local fingerprint.
- [x] Add a 2-second polling fallback with the existing 750ms stability delay.
- [x] Skip remote stat/write when local content is unchanged.
- [x] Do not create watcher/poller/controller when auto upload is disabled.
- [ ] Run complete focused tests, main check, formatting and diff validation.
- [ ] Review lifecycle, duplicate suppression, reload suppression and setting compatibility.
- [ ] Build/install and manually verify Zed and Notepad-- with the switch on/off.
