# RDP Frame Memory And Performance Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Bound RDP framebuffer retention in both processes, release obsolete GPUI textures, stop tab-lifetime polling leaks, and cap extreme HiDPI framebuffer sizes.

**Architecture:** Replace both unbounded frame paths with mailboxes that preserve control events while retaining only the newest undisplayed complete frame. The GPUI view owns its polling task and retains two rendered image generations before explicitly dropping older atlas textures. The helper wire format remains unchanged and the corrected provider is released as `0.1.4`.

**Tech Stack:** Rust, GPUI, Tokio, std synchronization primitives, IronRDP, JSON-line plus binary BGRA helper protocol.

---

### Task 1: Add The Host Coalescing Output Mailbox

**Files:**
- Create: `crates/remote_desktop/src/output_mailbox.rs`
- Modify: `crates/remote_desktop/src/lib.rs`
- Modify: `crates/remote_desktop/src/runtime.rs`
- Test: `crates/remote_desktop/src/output_mailbox.rs`

- [ ] **Step 1: Write failing mailbox tests**

Add tests covering latest-frame replacement, control ordering, terminal-frame clearing, and sender shutdown. Use small frame payloads so tests prove ownership semantics without allocating production-sized buffers.

```rust
#[test]
fn keeps_only_latest_pending_frame() {
    let (tx, rx) = output_mailbox();
    tx.send(frame(1)).unwrap();
    tx.send(frame(2)).unwrap();
    tx.send(frame(3)).unwrap();

    let batch = rx.drain();

    assert_eq!(Vec::<RemoteDesktopOutput>::new(), batch.control);
    assert_eq!(Some(frame(3)), batch.latest_frame);
}

#[test]
fn terminal_event_discards_pending_frame() {
    let (tx, rx) = output_mailbox();
    tx.send(frame(7)).unwrap();
    tx.send(RemoteDesktopOutput::Terminated("closed".into()))
        .unwrap();

    let batch = rx.drain();

    assert_eq!(None, batch.latest_frame);
    assert_eq!(
        vec![RemoteDesktopOutput::Terminated("closed".into())],
        batch.control
    );
}
```

- [ ] **Step 2: Run tests and confirm the API is missing**

Run:

```bash
rtk cargo test -p remote_desktop output_mailbox -- --nocapture
```

Expected: FAIL because `output_mailbox`, `OutputMailboxSender`, `OutputMailboxReceiver`, and `OutputBatch` do not exist.

- [ ] **Step 3: Implement the mailbox**

Implement this API with `Arc<Mutex<State>>`; host receiving is non-blocking, so no condition variable is required.

```rust
pub struct OutputBatch {
    pub control: Vec<RemoteDesktopOutput>,
    pub latest_frame: Option<RemoteDesktopOutput>,
}

#[derive(Clone)]
pub struct OutputMailboxSender {
    shared: Arc<Mutex<State>>,
}

pub struct OutputMailboxReceiver {
    shared: Arc<Mutex<State>>,
}

pub fn output_mailbox() -> (OutputMailboxSender, OutputMailboxReceiver);

impl OutputMailboxSender {
    pub fn send(&self, output: RemoteDesktopOutput) -> Result<(), OutputMailboxClosed>;
}

impl OutputMailboxReceiver {
    pub fn drain(&self) -> OutputBatch;
}
```

`Frame` and `FrameBgra` replace `latest_frame`. `ConnectionFailure` and `Terminated` clear it before entering `control`. Other outputs append to `control`. Track sender count with a manual `Clone`/`Drop` implementation so `send` rejects output after the last sender closes.

- [ ] **Step 4: Export and wire the runtime type**

```rust
pub struct RemoteDesktopRuntime {
    pub input_tx: tokio::sync::mpsc::UnboundedSender<RemoteDesktopInput>,
    pub output_rx: OutputMailboxReceiver,
}
```

Export the mailbox receiver from `remote_desktop::lib`; keep the sender crate-private unless backend tests need it.

- [ ] **Step 5: Run mailbox tests**

Run:

```bash
rtk cargo test -p remote_desktop output_mailbox -- --nocapture
```

Expected: PASS with all mailbox tests succeeding.

- [ ] **Step 6: Commit the mailbox**

```bash
rtk git add crates/remote_desktop/src/output_mailbox.rs crates/remote_desktop/src/lib.rs crates/remote_desktop/src/runtime.rs
rtk git commit -m "perf(rdp): bound host frame mailbox"
```

### Task 2: Integrate The Host Mailbox And Split The RDP Backend

**Files:**
- Modify: `crates/remote_desktop/src/backends/rdp.rs`
- Create: `crates/remote_desktop/src/backends/rdp/session.rs`
- Create: `crates/remote_desktop/src/backends/rdp/output_reader.rs`
- Test: `crates/remote_desktop/src/backends/rdp/output_reader.rs`
- Test: `crates/remote_desktop/src/backends/rdp.rs`

- [ ] **Step 1: Add a regression test for output-reader frame replacement**

Feed two binary `FrameBgraBytes` events through the reader and assert the receiver returns only the second frame while retaining a preceding `Connected` event.

```rust
#[test]
fn output_reader_coalesces_binary_frames() {
    let input = two_bgra_frames_stream();
    let (output_tx, output_rx) = output_mailbox();
    let (signal_tx, signal_rx) = std::sync::mpsc::channel();

    read_outputs(std::io::Cursor::new(input), output_tx, signal_tx);

    let batch = output_rx.drain();
    assert_eq!(Some(expected_second_frame()), batch.latest_frame);
    assert!(matches!(signal_rx.try_recv(), Ok(BackendSignal::OutputEnded)));
}
```

- [ ] **Step 2: Run the focused test and confirm old channel behavior fails the contract**

Run:

```bash
rtk cargo test -p remote_desktop output_reader_coalesces_binary_frames -- --nocapture
```

Expected: FAIL because the backend still accepts `std::sync::mpsc::Sender` and exposes every frame.

- [ ] **Step 3: Extract stdout parsing**

Move `HelperOutput`, binary frame readers, helper-event conversion, disconnect detection, and output-reader tests to `backends/rdp/output_reader.rs`. Expose only:

```rust
pub(super) fn spawn_output_reader(
    stdout: ChildStdout,
    output_tx: OutputMailboxSender,
    signal_tx: std::sync::mpsc::Sender<BackendSignal>,
);
```

Preserve binary length validation and the legacy Base64 frame compatibility test.

- [ ] **Step 4: Extract session orchestration**

Move helper lifecycle, reconnect state, input draining/coalescing, request writing, and process spawning to `backends/rdp/session.rs`. Keep `RdpBackend`, `HelperProcessConfig`, public constructor, and module declarations in `rdp.rs`.

```rust
pub(super) fn run_backend(
    helper: HelperProcessConfig,
    options: RemoteDesktopConnectionOptions,
    initial_size: RemoteDesktopSize,
    input_rx: UnboundedReceiver<RemoteDesktopInput>,
    output_tx: OutputMailboxSender,
);
```

Keep each resulting source file at or below 300 lines by placing tests beside their owning helper module.

- [ ] **Step 5: Replace the backend output channel**

Create the runtime with `output_mailbox()` and pass cloned senders to the session and stdout reader. Convert all `send_status` and `send_failure` helpers to `OutputMailboxSender`.

- [ ] **Step 6: Update failed-runtime construction**

Use the same mailbox in `remote_desktop_view` error startup paths so all runtime creation has one output API.

- [ ] **Step 7: Run backend tests and size checks**

Run:

```bash
rtk cargo test -p remote_desktop -- --nocapture
rtk wc -l crates/remote_desktop/src/backends/rdp.rs crates/remote_desktop/src/backends/rdp/*.rs
```

Expected: tests PASS and every touched backend source file is at most 300 lines.

- [ ] **Step 8: Commit backend integration**

```bash
rtk git add crates/remote_desktop/src/backends/rdp.rs crates/remote_desktop/src/backends/rdp crates/remote_desktop_view/src/view.rs
rtk git commit -m "refactor(rdp): coalesce backend frame output"
```

### Task 3: Bound GPUI Frame And View Lifecycles

**Files:**
- Modify: `crates/remote_desktop_view/src/view.rs`
- Create: `crates/remote_desktop_view/src/view/frame_pipeline.rs`
- Create: `crates/remote_desktop_view/src/view/frame_lifecycle.rs`
- Create: `crates/remote_desktop_view/src/view/resize.rs`
- Create: `crates/remote_desktop_view/src/view/render.rs`
- Modify: `main/src/home/home_tabs.rs`
- Test: new modules above

- [ ] **Step 1: Write failing rendered-generation tests**

Use a pure `RenderedFrameLifecycle<T>` contract so tests do not depend on a real GPU.

```rust
#[test]
fn third_distinct_frame_retires_the_first_generation() {
    let mut lifecycle = RenderedFrameLifecycle::default();

    assert_eq!(None, lifecycle.promote(frame(1)));
    assert_eq!(None, lifecycle.promote(frame(2)));
    assert_eq!(Some(frame(1)), lifecycle.promote(frame(3)));
    assert_eq!(Some(&frame(3)), lifecycle.current());
}

#[test]
fn release_deduplicates_retained_image_ids() {
    let mut lifecycle = RenderedFrameLifecycle::default();
    lifecycle.promote(frame(4));
    lifecycle.promote(frame(4));

    assert_eq!(vec![frame(4)], lifecycle.take_all_distinct());
}
```

- [ ] **Step 2: Write failing resize-area tests**

```rust
#[test]
fn caps_extreme_hidpi_area_without_changing_aspect_ratio() {
    let bounds = Bounds::new(point(px(0.0), px(0.0)), size(px(5120.0), px(2880.0)));

    assert_eq!(Some((3840, 2160)), resize_dimensions(bounds, 2.0));
}

#[test]
fn preserves_1080p_at_two_x() {
    let bounds = Bounds::new(point(px(0.0), px(0.0)), size(px(1920.0), px(1080.0)));

    assert_eq!(Some((3840, 2160)), resize_dimensions(bounds, 2.0));
}
```

- [ ] **Step 3: Run tests and verify the new contracts are absent**

Run:

```bash
rtk cargo test -p remote_desktop_view frame_lifecycle resize -- --nocapture
```

Expected: FAIL because the modules and lifecycle contract do not exist.

- [ ] **Step 4: Implement frame lifecycle and frame pipeline**

`frame_pipeline.rs` applies `OutputBatch.control`, converts only `latest_frame`, and updates the newest `Arc<RenderImage>`. `frame_lifecycle.rs` owns current/previous rendered generations and returns images that are safe to drop.

```rust
pub(crate) struct RenderedFrameLifecycle {
    current: Option<Arc<RenderImage>>,
    previous: Option<Arc<RenderImage>>,
}

impl RenderedFrameLifecycle {
    pub(crate) fn promote(
        &mut self,
        latest: Arc<RenderImage>,
    ) -> Option<Arc<RenderImage>>;
    pub(crate) fn current(&self) -> Option<Arc<RenderImage>>;
    pub(crate) fn take_all_distinct(&mut self) -> Vec<Arc<RenderImage>>;
}
```

- [ ] **Step 5: Implement render-time texture retirement**

Move `impl Render for RemoteDesktopView` to `view/render.rs`. Before painting, promote the latest image; call `window.drop_image(retired)` for the returned older generation; then render the current image.

- [ ] **Step 6: Own the polling task and release cleanup**

Store the polling `Task<()>` in the view. Capture a `WindowHandle` passed from `home_tabs.rs`. Register `cx.on_release` to send `Close`, cancel by dropping the owned task, and call `window.drop_image` for all distinct retained images.

```rust
let window_handle = window.window_handle();
let view = cx.new(|cx| RemoteDesktopView::new(config, window_handle, cx));
```

The loop exits when `this.update` returns an error even before field-drop cancellation runs.

- [ ] **Step 7: Implement the 4K-equivalent area cap**

Move resize helpers to `view/resize.rs`. Compute desired physical dimensions, apply a uniform scale when area exceeds `3840 * 2160`, clamp to DisplayControl bounds, and normalize width to even.

- [ ] **Step 8: Run view tests and file-size checks**

Run:

```bash
rtk cargo test -p remote_desktop_view -- --nocapture
rtk wc -l crates/remote_desktop_view/src/view.rs crates/remote_desktop_view/src/view/*.rs
```

Expected: tests PASS and every touched view source file is at most 300 lines.

- [ ] **Step 9: Commit the view lifecycle fix**

```bash
rtk git add crates/remote_desktop_view/src/view.rs crates/remote_desktop_view/src/view main/src/home/home_tabs.rs
rtk git commit -m "fix(rdp): release obsolete GPUI frame textures"
```

### Task 4: Add The Helper Coalescing Mailbox

**Files:**
- Create in `../onetcli-extensions`: `extensions/remote-desktop/rdp-helper/src/output_mailbox.rs`
- Modify in `../onetcli-extensions`: `extensions/remote-desktop/rdp-helper/src/main.rs`
- Test: helper mailbox module

- [ ] **Step 1: Write failing blocking-mailbox tests**

Tests cover newest-frame retention, control delivery, terminal clearing, and wakeup when the last sender drops.

```rust
#[test]
fn last_sender_drop_wakes_receiver() {
    let (tx, rx) = output_mailbox();
    let waiter = std::thread::spawn(move || rx.recv());

    drop(tx);

    assert_eq!(None, waiter.join().unwrap());
}
```

- [ ] **Step 2: Run helper tests and confirm failure**

Run in `../onetcli-extensions`:

```bash
rtk cargo test --manifest-path extensions/remote-desktop/rdp-helper/Cargo.toml output_mailbox -- --nocapture
```

Expected: FAIL because the helper mailbox module is absent.

- [ ] **Step 3: Implement the blocking mailbox**

Use `Arc<(Mutex<State>, Condvar)>` with manual sender counting. The state stores reliable controls, one latest frame, and `closed`. `recv` waits until a control, frame, or closure is available.

```rust
pub fn output_mailbox() -> (OutputSender, OutputReceiver);

impl OutputSender {
    pub fn send(&self, event: HelperEvent) -> Result<(), MailboxClosed>;
}

impl OutputReceiver {
    pub fn recv(&self) -> Option<HelperEvent>;
}
```

- [ ] **Step 4: Update the output writer**

Replace `for event in output_rx` with:

```rust
while let Some(event) = output_rx.recv() {
    write_event(&event)?;
}
```

- [ ] **Step 5: Run helper mailbox tests**

Run:

```bash
rtk cargo test --manifest-path extensions/remote-desktop/rdp-helper/Cargo.toml output_mailbox -- --nocapture
```

Expected: PASS.

- [ ] **Step 6: Commit the helper mailbox**

```bash
rtk git -C ../onetcli-extensions add extensions/remote-desktop/rdp-helper/src/output_mailbox.rs extensions/remote-desktop/rdp-helper/src/main.rs
rtk git -C ../onetcli-extensions commit -m "perf(rdp): bound helper frame mailbox"
```

### Task 5: Integrate Helper Output, Split The Module, And Release 0.1.4

**Files:**
- Modify in `../onetcli-extensions`: `extensions/remote-desktop/rdp-helper/src/rdp.rs`
- Create in `../onetcli-extensions`: `extensions/remote-desktop/rdp-helper/src/rdp/config.rs`
- Create in `../onetcli-extensions`: `extensions/remote-desktop/rdp-helper/src/rdp/output.rs`
- Modify in `../onetcli-extensions`: `extensions/remote-desktop/rdp-helper/src/clipboard.rs`
- Modify in `../onetcli-extensions`: `extensions/remote-desktop/rdp-helper/Cargo.toml`
- Modify in `../onetcli-extensions`: `extensions/remote-desktop/rdp-helper/Cargo.lock`
- Modify in `../onetcli-extensions`: `extensions/remote-desktop/rdp/remote_desktop_provider.json`
- Modify in `../onetcli-extensions`: generated/package metadata required by existing scripts
- Modify in `onetcli`: `crates/remote_desktop/src/backend.rs`

- [ ] **Step 1: Write a failing integration test for burst output**

Map multiple `RdpOutputEvent::Image` values into the helper sender without consuming and assert receiver produces only the final image after the reliable first-frame `Connected` event.

- [ ] **Step 2: Run the integration test and confirm unbounded behavior**

Run:

```bash
rtk cargo test --manifest-path extensions/remote-desktop/rdp-helper/Cargo.toml output_mapper_coalesces_burst -- --nocapture
```

Expected: FAIL until `RdpRuntime` and clipboard output use `OutputSender`.

- [ ] **Step 3: Replace all helper output senders**

Change `RdpRuntime.output_rx`, `spawn_client_thread`, output mapper, clipboard factory/backend, and failure sends to use the helper mailbox types. Preserve the existing event shapes.

- [ ] **Step 4: Split helper configuration and output mapping**

Move `build_config`, `client_build`, and `platform_type` to `rdp/config.rs`. Move `RdpOutputMapper` and its tests to `rdp/output.rs`. Keep input mapping and runtime startup in `rdp.rs`. Each touched file must be at most 300 lines.

- [ ] **Step 5: Bump the provider to 0.1.4**

Update crate version, lockfile package version, provider manifest version, and any generated marketplace/package metadata. In the main repository set:

```rust
const MIN_RDP_PROVIDER_VERSION: &str = "0.1.4";
```

Update version tests from `0.1.3` to `0.1.4` and ensure `0.1.3` is rejected.

- [ ] **Step 6: Verify helper protocol compatibility and packages**

Run in `../onetcli-extensions`:

```bash
rtk cargo test --manifest-path extensions/remote-desktop/rdp-helper/Cargo.toml -- --nocapture
rtk cargo build --release --manifest-path extensions/remote-desktop/rdp-helper/Cargo.toml --target aarch64-apple-darwin
rtk bash scripts/package-remote-desktop-provider.sh rdp aarch64-apple-darwin /tmp/onetcli-rdp-package 0.1.4
rtk bash scripts/verify-remote-desktop-provider-package.sh /tmp/onetcli-rdp-package/rdp-remote-desktop-provider-aarch64-apple-darwin.tar.gz
rtk wc -l extensions/remote-desktop/rdp-helper/src/rdp.rs extensions/remote-desktop/rdp-helper/src/rdp/*.rs extensions/remote-desktop/rdp-helper/src/output_mailbox.rs
```

Expected: helper tests and package verification PASS; touched source files are at most 300 lines.

- [ ] **Step 7: Commit provider integration and versioning**

```bash
rtk git -C ../onetcli-extensions add extensions/remote-desktop/rdp-helper extensions/remote-desktop/rdp scripts manifest.json
rtk git -C ../onetcli-extensions commit -m "fix(rdp): coalesce provider frames"
rtk git add crates/remote_desktop/src/backend.rs
rtk git commit -m "chore(rdp): require provider 0.1.4"
```

### Task 6: Cross-Repository Verification And Completion Audit

**Files:**
- Main and extension repositories

- [ ] **Step 1: Format both repositories**

Run:

```bash
rtk cargo fmt -p remote_desktop -p remote_desktop_view -p main
rtk cargo fmt --manifest-path ../onetcli-extensions/extensions/remote-desktop/rdp-helper/Cargo.toml
```

Expected: exit 0.

- [ ] **Step 2: Run main repository tests**

Run:

```bash
rtk cargo test -p remote_desktop -- --nocapture
rtk cargo test -p remote_desktop_view -- --nocapture
```

Expected: all tests PASS.

- [ ] **Step 3: Run helper tests**

Run:

```bash
rtk cargo test --manifest-path ../onetcli-extensions/extensions/remote-desktop/rdp-helper/Cargo.toml -- --nocapture
```

Expected: all tests PASS.

- [ ] **Step 4: Run compilation and lint gates**

Run:

```bash
rtk proxy env CLANG_MODULE_CACHE_PATH=/tmp/clang-cache cargo check -p remote_desktop -p remote_desktop_view -p main
rtk proxy env CLANG_MODULE_CACHE_PATH=/tmp/clang-cache cargo clippy -p remote_desktop -p remote_desktop_view -p main -- -D warnings
rtk cargo clippy --manifest-path ../onetcli-extensions/extensions/remote-desktop/rdp-helper/Cargo.toml -- -D warnings
```

Expected: all commands exit 0 with no warnings promoted to errors.

- [ ] **Step 5: Run deterministic retention audit**

Confirm tests prove these bounds rather than only checking successful rendering:

```text
host pending complete frames <= 1
helper pending complete frames <= 1, plus the frame being written
GPUI rendered generations retained <= 2
terminal events clear stale pending frames
view release owns/cancels its polling task
```

- [ ] **Step 6: Inspect repository scope**

Run:

```bash
rtk git status --short --branch
rtk git diff --check
rtk git -C ../onetcli-extensions status --short --branch
rtk git -C ../onetcli-extensions diff --check
```

Expected: only the known unrelated `connection.ncx` remains untracked in the extension repository; task changes are committed.

- [ ] **Step 7: Perform code review and address findings**

Use `superpowers:requesting-code-review`. Verify correctness against the design, especially terminal-event ordering, sender closure, duplicate image IDs, release cleanup, and provider version enforcement. Apply accepted findings and rerun all affected verification commands.

- [ ] **Step 8: Final completion verification**

Use `superpowers:verification-before-completion`. Re-read the design acceptance criteria and map each criterion to current code, tests, command output, or an explicitly reported unavailable Windows manual check. Do not claim #90/#105 fixed solely because unit tests pass.
