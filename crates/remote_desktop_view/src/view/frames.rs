use std::sync::Arc;

use gpui::{RenderImage, SharedString};
use remote_desktop::{RemoteDesktopFrameRect, RgbaFramebuffer};

use crate::pixels::bgra_to_render_image;

use super::{RemoteDesktopView, frame_sync};

impl RemoteDesktopView {
    pub(super) fn install_rgba_frame(&mut self, width: u16, height: u16, rgba: Vec<u8>) -> bool {
        self.install_bgra_frame(width, height, crate::pixels::rgba_to_bgra(rgba))
    }

    pub(super) fn install_bgra_frame(&mut self, width: u16, height: u16, bgra: Vec<u8>) -> bool {
        let framebuffer = match RgbaFramebuffer::from_bgra(width, height, bgra) {
            Ok(framebuffer) => framebuffer,
            Err(error) => {
                self.status = SharedString::from(error.to_string());
                return false;
            }
        };
        let image = match bgra_to_render_image(width, height, framebuffer.clone_rgba()) {
            Ok(image) => image,
            Err(error) => {
                self.status = SharedString::from(error.to_string());
                return false;
            }
        };
        self.framebuffer = Some(framebuffer);
        self.install_frame(Ok(image))
    }

    pub(super) fn apply_bgra_rects(
        &mut self,
        width: u16,
        height: u16,
        rects: &[RemoteDesktopFrameRect],
        bgra: Vec<u8>,
    ) {
        if !self.frame_sync.can_apply_delta((width, height)) {
            self.record_rejected_delta(width, height, "delta has no matching base frame");
            return;
        }
        let Some(framebuffer) = self.framebuffer.as_ref() else {
            self.record_rejected_delta(width, height, "missing base framebuffer");
            return;
        };
        let patched = match patched_bgra_framebuffer(framebuffer, width, height, rects, &bgra) {
            Ok(patched) => patched,
            Err(error) => {
                let reason = error.to_string();
                self.record_rejected_delta(width, height, &reason);
                return;
            }
        };
        let image = match bgra_to_render_image(width, height, patched.clone_rgba()) {
            Ok(image) => image,
            Err(error) => {
                self.record_rejected_delta(width, height, "failed to build delta frame");
                self.status = SharedString::from(error.to_string());
                return;
            }
        };
        match self.frame_sync.accept_delta((width, height)) {
            frame_sync::DeltaDisposition::Applied => {
                self.framebuffer = Some(patched);
                self.latest_frame = Some(Arc::new(image));
                self.remote_size = Some((width, height));
            }
            disposition @ frame_sync::DeltaDisposition::Rejected { .. } => {
                self.log_rejected_delta(
                    width,
                    height,
                    "delta synchronization changed before commit",
                    disposition,
                );
            }
        }
    }

    fn install_frame(&mut self, image: anyhow::Result<RenderImage>) -> bool {
        match image {
            Ok(image) => {
                self.latest_frame = Some(Arc::new(image));
                true
            }
            Err(error) => {
                self.status = SharedString::from(error.to_string());
                false
            }
        }
    }

    fn record_rejected_delta(&mut self, width: u16, height: u16, reason: &str) {
        let disposition = self.frame_sync.reject_delta();
        self.log_rejected_delta(width, height, reason, disposition);
    }

    fn log_rejected_delta(
        &self,
        width: u16,
        height: u16,
        reason: &str,
        disposition: frame_sync::DeltaDisposition,
    ) {
        if let frame_sync::DeltaDisposition::Rejected { recovery_started } = disposition {
            let snapshot = self.frame_sync.snapshot();
            let resize_capability = self.capabilities.map(|capabilities| capabilities.resize);
            if recovery_started {
                tracing::warn!(
                    protocol = self.options.protocol.label(),
                    session_generation = snapshot.session_generation,
                    phase = ?snapshot.phase,
                    base_size = ?snapshot.base_size,
                    remote_size = ?self.remote_size,
                    viewport_size = ?self.last_resize_size,
                    resize_capability = ?resize_capability,
                    width,
                    height,
                    full_frames = snapshot.full_frames,
                    deltas = snapshot.deltas,
                    dropped_deltas = snapshot.dropped_deltas,
                    recoveries = snapshot.recoveries,
                    reason,
                    "remote desktop frame recovery required"
                );
            } else {
                tracing::debug!(
                    protocol = self.options.protocol.label(),
                    session_generation = snapshot.session_generation,
                    phase = ?snapshot.phase,
                    base_size = ?snapshot.base_size,
                    remote_size = ?self.remote_size,
                    viewport_size = ?self.last_resize_size,
                    resize_capability = ?resize_capability,
                    width,
                    height,
                    full_frames = snapshot.full_frames,
                    deltas = snapshot.deltas,
                    dropped_deltas = snapshot.dropped_deltas,
                    recoveries = snapshot.recoveries,
                    reason,
                    "dropping remote desktop delta while awaiting a base frame"
                );
            }
        }
    }
}

fn patched_bgra_framebuffer(
    framebuffer: &RgbaFramebuffer,
    width: u16,
    height: u16,
    rects: &[RemoteDesktopFrameRect],
    bgra: &[u8],
) -> anyhow::Result<RgbaFramebuffer> {
    anyhow::ensure!(
        framebuffer.width() == width && framebuffer.height() == height,
        "base framebuffer dimensions changed"
    );

    let mut patched = framebuffer.clone();
    let mut offset = 0usize;
    for rect in rects {
        anyhow::ensure!(
            rect.width > 0 && rect.height > 0,
            "dirty rectangle is empty"
        );
        let end = offset
            .checked_add(rect.byte_len)
            .ok_or_else(|| anyhow::anyhow!("dirty rectangle payload length overflow"))?;
        anyhow::ensure!(end <= bgra.len(), "dirty rectangle payload is truncated");
        patched
            .patch_rgba_rect(rect.x, rect.y, rect.width, rect.height, &bgra[offset..end])
            .map_err(|_| anyhow::anyhow!("invalid dirty rectangle"))?;
        offset = end;
    }
    anyhow::ensure!(
        offset == bgra.len(),
        "dirty rectangle payload has trailing bytes"
    );
    Ok(patched)
}

#[cfg(test)]
#[path = "frames_tests.rs"]
mod tests;
