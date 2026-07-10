# RDP Frame Memory And Performance Design

**Date:** 2026-07-10
**Status:** Approved for written-spec review

## Summary

GitHub Issues #90 and #105 report that RDP sessions consume more than 1 GiB and can continue growing past 3 GiB. The two reports describe the same frame-delivery failure mode: full remote desktop frames cross two unbounded queues, every displayed frame creates a new GPUI image identity, and previously uploaded GPU textures are never explicitly removed.

The fix uses a latest-frame pipeline in both `onetcli` and `onetcli-extensions`. Control events remain reliable and ordered, but obsolete full framebuffer snapshots are replaceable. The GPUI view retains two rendered generations for renderer safety, explicitly releases older textures, owns its polling task for the lifetime of the tab, and caps extreme HiDPI requests to a 4K-equivalent pixel area.

The existing JSON-header-plus-binary-BGRA wire format remains unchanged. The corrected RDP provider is released as version `0.1.4`, and the main application requires that version so users do not silently continue with the unbounded helper implementation.

## Existing Behavior And Root Causes

### GPUI texture retention

`RemoteDesktopView` converts every `Frame` or `FrameBgra` into a fresh `RenderImage`. GPUI assigns every `RenderImage::new` a unique `ImageId`; the DirectX and Metal atlases retain textures by that identity until `drop_image` removes them. Replacing the Rust `Arc<RenderImage>` does not remove its atlas entry.

The current RDP view never calls `window.drop_image`, so rendered frame textures accumulate for the lifetime of the window. Zed's video view, built on the same GPUI revision, keeps current and previous rendered frames and explicitly drops older images.

### Host-side unbounded frame accumulation

`remote_desktop::backends::rdp` sends every complete RGBA/BGRA frame through an unbounded `std::sync::mpsc` channel. `RemoteDesktopView::drain_output` first moves every pending output into a `Vec`, then converts every queued frame even though only the newest one can be visible.

If producer throughput exceeds UI conversion and GPU upload throughput, host memory grows by complete-frame increments with no upper bound.

### Helper-side unbounded frame accumulation

The RDP helper maps IronRDP image events into complete BGRA buffers and sends them through another unbounded `std::sync::mpsc` channel. A separate writer thread serializes those events to stdout. If stdout or the host reader is slower than IronRDP output, the helper queue retains every obsolete framebuffer.

### Amplifying factors

- The view wakes every 33 ms even after its entity may have been released because the polling task is detached and does not stop on update failure.
- Display scaling is applied to the requested RDP dimensions. A 1920x1080 logical view at 2x requests 3840x2160, increasing pixels, raw frame bytes, conversion work, and texture memory by four times.
- The per-dimension limit of 8192 permits a theoretical 8192x8192 BGRA frame of 256 MiB; it does not constrain total pixel area.

## Goals

- Stop DirectX and Metal texture growth during continuous RDP updates.
- Bound pending complete framebuffers in both the helper and main process.
- Preserve reliable delivery of connection, status, clipboard, cursor, failure, and termination events.
- Display the most recent complete framebuffer rather than replaying obsolete frames.
- Stop all RDP view polling and close the helper when the tab/view is released.
- Preserve normal HiDPI clarity while preventing extreme framebuffer sizes.
- Keep the current helper wire format and existing input, clipboard, resize, and reconnect behavior.
- Add deterministic tests that prove the frame-retention bounds and lifecycle rules.

## Non-Goals

- Adding H.264, AV1, RemoteFX, or another video transport.
- Modifying GPUI internals or introducing mutable in-place GPUI textures.
- Adding a user-facing quality or frame-rate setting.
- Changing RDP keyboard, mouse, clipboard, reconnect, or authentication semantics.
- Optimizing VNC in this change. VNC may reuse the mailbox design in a later task.
- Refactoring unrelated remote desktop form, storage, provider-install, or marketplace code.

## Architecture

```text
IronRDP full framebuffer
        |
        v
helper coalescing mailbox
  - reliable control queue
  - at most one pending image
        |
        v
JSON header + binary BGRA stdout
        |
        v
host coalescing mailbox
  - reliable control queue
  - at most one pending image
        |
        v
RemoteDesktopView latest frame
        |
        v
GPUI current + previous rendered generations
  - older generation explicitly drop_image'd
```

Every IronRDP `Image` event is a complete framebuffer after IronRDP has applied partial RDP updates. Replacing an undisplayed image with a newer image is therefore safe: the newer image contains the complete current desktop state.

## Host Output Mailbox

Add a focused mailbox module in the `remote_desktop` crate. `RemoteDesktopRuntime` exposes its receiver instead of a raw `std::sync::mpsc::Receiver`.

The mailbox state contains:

```rust
struct OutputMailboxState {
    control: VecDeque<RemoteDesktopOutput>,
    latest_frame: Option<RemoteDesktopOutput>,
    closed: bool,
}
```

Only `Frame` and `FrameBgra` are accepted by `latest_frame`. Sending a new frame replaces and drops the previous pending frame immediately. All other variants enter the control queue.

Receiver draining returns control events plus at most one frame. Controls are applied before the current frame, except that `ConnectionFailure` or `Terminated` clears the pending frame so a stale desktop is not installed after terminal state.

Cursor position updates may be coalesced independently because only the latest absolute position matters. Cursor default/hidden state changes are reliable control events.

The host output reader continues reading stdout promptly, preventing pipe backpressure from becoming the primary frame limiter, while host heap retention remains bounded to the frame currently being read plus one pending frame.

## Helper Output Mailbox

Add a blocking coalescing mailbox to the RDP helper. It uses `Mutex`, `Condvar`, a control `VecDeque`, one latest-frame slot, a sender count, and a closed flag.

The IronRDP output mapper sends through this mailbox. The stdout writer blocks on `recv` without polling. While it writes one large frame, newer frame sends replace the one pending frame instead of appending complete buffers.

Required semantics:

- `Connected`, `Status`, clipboard, cursor state, failure, and termination are not dropped.
- A newer frame replaces only an undisplayed frame.
- Failure or termination removes the pending frame before enqueueing the terminal event.
- Dropping the last sender wakes the writer and lets it exit after reliable controls are drained.
- A broken stdout causes writer exit; helper shutdown then propagates through the existing process lifecycle.

The resulting helper bound is one frame being written plus one mapped pending frame. IronRDP's existing bounded output channel remains an additional upstream guard.

## GPUI Frame Lifecycle

`RemoteDesktopView` separates the newest decoded frame from frames already handed to GPUI:

```rust
latest_frame: Option<Arc<RenderImage>>,
current_rendered_frame: Option<Arc<RenderImage>>,
previous_rendered_frame: Option<Arc<RenderImage>>,
```

During render:

1. If `latest_frame` differs from `current_rendered_frame`, move current to previous.
2. Before replacing previous, call `window.drop_image` for the older previous image.
3. Promote latest to current and render current.
4. Never drop an image whose `ImageId` is also retained in another slot.

Two rendered generations are retained because the prior scene may still reference its texture in the GPU pipeline. This follows the same-revision Zed video implementation rather than dropping the immediately previous frame prematurely.

On view release, send `Close` if the runtime exists, cancel the owned polling task, and remove every distinct image remaining in the three slots through the captured window handle. Cleanup is idempotent and does not depend solely on `TabContent::try_close`.

## Polling Lifecycle

The 33 ms output polling task becomes an owned `Task<()>` field instead of a detached task. Dropping the view cancels the task. The loop also exits if updating the entity fails.

Polling remains at approximately 30 FPS. This limits image conversion and GPU upload frequency, while the latest-frame mailbox ensures the view receives the freshest state without accumulating 60 FPS or higher producer output.

An event-driven UI receiver is not introduced because it would broaden the runtime/threading change without improving the established memory bound.

## Frame Size Policy

Calculate the desired physical dimensions from content bounds and display scale, then apply a total-pixel limit:

```rust
const MAX_REMOTE_FRAME_PIXELS: u64 = 3840 * 2160;
```

If the desired area exceeds the limit, scale width and height down by the same factor. Then apply the existing RDP constraints: minimum dimensions, maximum dimensions, and even width.

This preserves 1920x1080 at 2x and any smaller area. Larger Retina, 5K, 8K, or extreme ultrawide requests are bounded to a 4K-equivalent area without changing aspect ratio.

## Module Boundaries

The change also reduces the touched oversized files by extracting responsibilities directly related to this fix.

Main repository:

- `crates/remote_desktop/src/output_mailbox.rs`: host coalescing sender/receiver and tests.
- `crates/remote_desktop/src/backends/rdp/session.rs`: helper session/reconnect orchestration.
- `crates/remote_desktop/src/backends/rdp/output_reader.rs`: stdout protocol reading and mailbox forwarding.
- `crates/remote_desktop_view/src/view/frame_pipeline.rs`: output application and image conversion.
- `crates/remote_desktop_view/src/view/frame_lifecycle.rs`: rendered-generation transitions and release set.
- `crates/remote_desktop_view/src/view/resize.rs`: HiDPI sizing and pixel-area cap.
- `crates/remote_desktop_view/src/view/render.rs`: GPUI render implementation.

Extension repository:

- `extensions/remote-desktop/rdp-helper/src/output_mailbox.rs`: blocking helper mailbox.
- `extensions/remote-desktop/rdp-helper/src/rdp/config.rs`: IronRDP connection configuration.
- `extensions/remote-desktop/rdp-helper/src/rdp/output.rs`: output mapper and mailbox bridge.

The split must keep each new or resulting touched source file at or below 300 lines. Keyboard, clipboard, input mapping, and provider packaging remain in their existing focused modules unless a compile dependency requires a minimal import update.

## Versioning And Compatibility

- Keep `FrameBgraBytes` JSON header and following binary BGRA payload unchanged.
- Bump `onetcli-rdp-helper` and the RDP provider manifest from `0.1.3` to `0.1.4`.
- Update marketplace/package metadata generated from the provider manifest as required by the existing release scripts.
- Raise `MIN_RDP_PROVIDER_VERSION` in the main repository from `0.1.3` to `0.1.4`.
- Existing providers below `0.1.4` use the current localized provider-too-old error and are not opened.
- VNC minimum-version behavior is unchanged.

## Error Handling

- Invalid binary frame lengths remain fatal to the helper output reader and trigger the existing failure/reconnect path.
- Mailbox poisoning is converted to connection failure or helper termination rather than panicking across the process boundary.
- `drop_image` errors are logged and do not stop the RDP session.
- Sender closure is a normal shutdown signal; it is not reported as a connection error after an explicit `Close`.
- Reliable control events are drained before normal mailbox shutdown.
- Terminal events clear stale pending frames before the UI applies terminal status.

## Testing Strategy

This is a high-regression-risk shared runtime and rendering behavior change, so implementation uses TDD.

Main repository tests must prove:

- Sending many frames before a receive retains only the newest frame.
- Control events survive frame replacement and preserve their ordering.
- Terminal events discard a stale pending frame.
- Receiver shutdown is deterministic when senders close.
- Rendered-frame lifecycle keeps at most two GPU generations and returns the correct older image for release.
- Release cleanup deduplicates identical `ImageId` values.
- A 1920x1080 view at 2x remains 3840x2160.
- Areas above 4K are scaled down without changing aspect ratio and still satisfy RDP dimension constraints.
- The polling task is owned by the view and release sends `Close` once.

Extension repository tests must prove:

- A blocked writer plus many producer frames retains only the currently written frame and newest pending frame.
- Reliable controls are not lost while frames are replaced.
- Failure and termination remove stale pending images.
- Last-sender drop wakes and closes the receiver.
- Existing binary frame bytes and headers remain wire-compatible.

Verification commands include targeted crate tests, full relevant crate tests, `cargo check`, `cargo clippy -- -D warnings`, formatting, package verification, and `git diff --check` in both repositories. A synthetic high-rate producer test provides deterministic queue-bound evidence; manual RDP observation records main and helper process memory separately under static and high-refresh scenes.

## Acceptance Criteria

- Continuous RDP updates no longer create an ever-growing number of GPUI atlas textures.
- Host pending-frame retention is at most one complete frame.
- Helper pending-frame retention is at most one complete frame in addition to the frame currently being written.
- Obsolete frames are dropped without losing connection, clipboard, cursor-state, failure, or termination events.
- Closing an RDP tab stops its polling task, closes the helper, and releases retained textures.
- Normal 2x 1080p remains sharp; requests above the 4K-equivalent pixel area are proportionally bounded.
- RDP provider `0.1.4` is required and package metadata is consistent.
- Relevant tests, checks, clippy, formatting, and package verification pass in both repositories.
- Manual verification shows memory stabilizing instead of growing with elapsed session time; if Windows runtime verification is unavailable locally, that limitation is reported explicitly and the deterministic bounds remain covered by tests.

## Delivery Boundaries

Only task-related files are staged and committed. Existing branch-ahead commits and unrelated untracked or modified files in either repository remain untouched. Main-repository and extension-repository changes use separate commits because they have independent release and rollback boundaries.
