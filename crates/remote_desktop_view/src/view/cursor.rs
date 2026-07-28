use std::sync::Arc;

use gpui::RenderImage;
use remote_desktop::RemoteDesktopCursor;

use super::frame_lifecycle::RenderedFrameLifecycle;
use crate::pixels::rgba_to_render_image;
use crate::pointer::RemoteCursorGeometry;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum RemoteCursorMode {
    #[default]
    Default,
    Hidden,
    Bitmap,
}

#[derive(Clone, PartialEq)]
struct RemoteCursorImage {
    image: Arc<RenderImage>,
    width: u16,
    height: u16,
    hotspot_x: u16,
    hotspot_y: u16,
}

impl RemoteCursorImage {
    fn new(cursor: RemoteDesktopCursor) -> anyhow::Result<Self> {
        let RemoteDesktopCursor {
            width,
            height,
            hotspot_x,
            hotspot_y,
            rgba,
        } = cursor;
        let image = rgba_to_render_image(width, height, rgba)?;
        Ok(Self {
            image: Arc::new(image),
            width,
            height,
            hotspot_x,
            hotspot_y,
        })
    }
}

#[derive(Clone)]
pub(super) struct RemoteCursorPaint {
    pub(super) image: Arc<RenderImage>,
    pub(super) geometry: RemoteCursorGeometry,
}

#[derive(Default)]
pub(super) struct RemoteCursorState {
    latest: Option<RemoteCursorImage>,
    rendered: RenderedFrameLifecycle<RemoteCursorImage>,
    pending_drops: Vec<RemoteCursorImage>,
    position: Option<(u16, u16)>,
    mode: RemoteCursorMode,
    pointer_hovered: bool,
}

impl RemoteCursorState {
    pub(super) fn install(&mut self, cursor: RemoteDesktopCursor) -> anyhow::Result<()> {
        self.latest = Some(RemoteCursorImage::new(cursor)?);
        self.mode = RemoteCursorMode::Bitmap;
        Ok(())
    }

    pub(super) fn set_position(&mut self, x: u16, y: u16) {
        self.position = Some((x, y));
    }

    pub(super) fn show_default(&mut self) {
        self.mode = RemoteCursorMode::Default;
        self.clear_images();
    }

    pub(super) fn hide(&mut self) {
        self.mode = RemoteCursorMode::Hidden;
        self.clear_images();
    }

    pub(super) fn reset_session(&mut self) {
        self.mode = RemoteCursorMode::Default;
        self.position = None;
        self.clear_images();
    }

    pub(super) fn set_pointer_hovered(&mut self, hovered: bool) {
        self.pointer_hovered = hovered;
    }

    pub(super) fn promote_latest(&mut self) -> Option<Arc<RenderImage>> {
        let latest = self.latest.clone()?;
        self.rendered.promote(latest).map(|cursor| cursor.image)
    }

    pub(super) fn paint_state(&self, remote_size: Option<(u16, u16)>) -> Option<RemoteCursorPaint> {
        // GPUI cannot keep the native cursor hidden while it is moving, so let
        // the native cursor own the hovered content and avoid drawing two cursors.
        if self.pointer_hovered || self.mode != RemoteCursorMode::Bitmap {
            return None;
        }
        let (remote_width, remote_height) = remote_size?;
        let (x, y) = self.position?;
        let cursor = self.rendered.current()?;
        Some(RemoteCursorPaint {
            image: cursor.image.clone(),
            geometry: RemoteCursorGeometry {
                remote_width,
                remote_height,
                x,
                y,
                width: cursor.width,
                height: cursor.height,
                hotspot_x: cursor.hotspot_x,
                hotspot_y: cursor.hotspot_y,
            },
        })
    }

    pub(super) fn take_pending_images(&mut self) -> Vec<Arc<RenderImage>> {
        std::mem::take(&mut self.pending_drops)
            .into_iter()
            .map(|cursor| cursor.image)
            .collect()
    }

    pub(super) fn release_all_images(&mut self) -> Vec<Arc<RenderImage>> {
        let mut cursors = std::mem::take(&mut self.pending_drops);
        for cursor in self
            .rendered
            .take_all_distinct(self.latest.take())
            .into_iter()
        {
            if !cursors.contains(&cursor) {
                cursors.push(cursor);
            }
        }
        self.position = None;
        self.mode = RemoteCursorMode::Default;
        self.pointer_hovered = false;
        cursors.into_iter().map(|cursor| cursor.image).collect()
    }

    fn clear_images(&mut self) {
        self.pending_drops
            .extend(self.rendered.take_all_distinct(self.latest.take()));
    }
}

#[cfg(test)]
#[path = "cursor_tests.rs"]
mod tests;
