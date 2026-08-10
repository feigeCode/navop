use remote_desktop::{RemoteDesktopFrameRect, RgbaFramebuffer};

use super::{RemoteDesktopView, frame_sync};

impl RemoteDesktopView {
    pub(super) fn record_rejected_delta(&mut self, width: u16, height: u16, reason: &str) {
        let disposition = self.frame_sync.reject_delta();
        self.log_rejected_delta(width, height, reason, disposition);
    }

    pub(super) fn log_rejected_delta(
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

pub(super) fn validate_bgra_rects(
    framebuffer: &RgbaFramebuffer,
    width: u16,
    height: u16,
    rects: &[RemoteDesktopFrameRect],
    bgra: &[u8],
) -> anyhow::Result<()> {
    anyhow::ensure!(
        framebuffer.width() == width && framebuffer.height() == height,
        "base framebuffer dimensions changed"
    );

    let mut offset = 0usize;
    for rect in rects {
        anyhow::ensure!(
            rect.width > 0 && rect.height > 0,
            "dirty rectangle is empty"
        );
        let expected_len = usize::from(rect.width)
            .checked_mul(usize::from(rect.height))
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or_else(|| anyhow::anyhow!("dirty rectangle size overflow"))?;
        anyhow::ensure!(
            rect.byte_len == expected_len,
            "dirty rectangle declared length is invalid"
        );
        let end = offset
            .checked_add(rect.byte_len)
            .ok_or_else(|| anyhow::anyhow!("dirty rectangle payload length overflow"))?;
        anyhow::ensure!(end <= bgra.len(), "dirty rectangle payload is truncated");
        anyhow::ensure!(
            rect.x <= framebuffer.width() && rect.y <= framebuffer.height(),
            "dirty rectangle origin is outside framebuffer"
        );
        anyhow::ensure!(
            usize::from(rect.x) + usize::from(rect.width) <= usize::from(framebuffer.width()),
            "dirty rectangle width exceeds framebuffer"
        );
        anyhow::ensure!(
            usize::from(rect.y) + usize::from(rect.height) <= usize::from(framebuffer.height()),
            "dirty rectangle height exceeds framebuffer"
        );
        offset = end;
    }
    anyhow::ensure!(
        offset == bgra.len(),
        "dirty rectangle payload has trailing bytes"
    );
    Ok(())
}

pub(super) fn apply_bgra_rects_to_framebuffer(
    framebuffer: &mut RgbaFramebuffer,
    width: u16,
    height: u16,
    rects: &[RemoteDesktopFrameRect],
    bgra: &[u8],
) -> anyhow::Result<()> {
    validate_bgra_rects(framebuffer, width, height, rects, bgra)?;

    let mut offset = 0usize;
    for rect in rects {
        let end = offset + rect.byte_len;
        framebuffer.patch_rgba_rect(rect.x, rect.y, rect.width, rect.height, &bgra[offset..end])?;
        offset = end;
    }
    Ok(())
}

#[cfg(test)]
#[path = "frames_tests.rs"]
mod tests;
