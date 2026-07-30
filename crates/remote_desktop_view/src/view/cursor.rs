use std::sync::Arc;

use gpui::RenderImage;
use remote_desktop::RemoteDesktopCursor;

use super::frame_lifecycle::RenderedFrameLifecycle;
use crate::native_cursor;
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
    manage_native_cursor: bool,
}

impl RemoteCursorState {
    pub(super) fn new(manage_native_cursor: bool) -> Self {
        Self {
            manage_native_cursor,
            ..Self::default()
        }
    }

    pub(super) fn install(&mut self, cursor: RemoteDesktopCursor) -> anyhow::Result<()> {
        self.latest = Some(RemoteCursorImage::new(cursor)?);
        self.mode = RemoteCursorMode::Bitmap;
        Ok(())
    }

    pub(super) fn set_position(&mut self, x: u16, y: u16) -> bool {
        let previous = self.position;
        let was_paintable = self.has_paintable_bitmap();
        self.position = Some((x, y));
        if was_paintable != self.has_paintable_bitmap() {
            self.sync_native_cursor();
        }
        previous != self.position
    }

    pub(super) fn show_default(&mut self) {
        self.mode = RemoteCursorMode::Default;
        self.clear_images();
        self.sync_native_cursor();
    }

    pub(super) fn hide(&mut self) {
        self.mode = RemoteCursorMode::Hidden;
        self.clear_images();
        self.sync_native_cursor();
    }

    pub(super) fn reset_session(&mut self) {
        self.mode = RemoteCursorMode::Default;
        self.position = None;
        self.clear_images();
        self.sync_native_cursor();
    }

    pub(super) fn set_pointer_hovered(&mut self, hovered: bool) -> bool {
        if self.pointer_hovered == hovered {
            return false;
        }
        self.pointer_hovered = hovered;
        self.sync_native_cursor();
        true
    }

    pub(super) fn rehide_native_cursor_after_pointer_move(&self) {
        if self.manage_native_cursor
            && should_hide_native_cursor(
                self.mode,
                self.pointer_hovered,
                self.has_paintable_bitmap(),
            )
        {
            // GPUI/Win32 may restore its native cursor before dispatching the
            // mouse-move callback. Hide it once after the canvas position has
            // been updated instead of repeatedly syncing it from every setter.
            native_cursor::hide();
        }
    }

    pub(super) fn promote_latest(&mut self) -> Option<Arc<RenderImage>> {
        let latest = self.latest.clone()?;
        let retired = self.rendered.promote(latest).map(|cursor| cursor.image);
        self.sync_native_cursor();
        retired
    }

    pub(super) fn paint_state(&self, remote_size: Option<(u16, u16)>) -> Option<RemoteCursorPaint> {
        if self.mode != RemoteCursorMode::Bitmap {
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
        if self.manage_native_cursor {
            native_cursor::restore();
        }
        cursors.into_iter().map(|cursor| cursor.image).collect()
    }

    fn clear_images(&mut self) {
        self.pending_drops
            .extend(self.rendered.take_all_distinct(self.latest.take()));
    }

    fn sync_native_cursor(&self) {
        if !self.manage_native_cursor {
            return;
        }
        if should_hide_native_cursor(self.mode, self.pointer_hovered, self.has_paintable_bitmap()) {
            native_cursor::hide();
        } else {
            native_cursor::restore();
        }
    }

    fn has_paintable_bitmap(&self) -> bool {
        self.position.is_some() && self.rendered.current().is_some()
    }
}

fn should_hide_native_cursor(
    mode: RemoteCursorMode,
    pointer_hovered: bool,
    bitmap_available: bool,
) -> bool {
    pointer_hovered
        && (mode == RemoteCursorMode::Hidden
            || (mode == RemoteCursorMode::Bitmap && bitmap_available))
}

#[cfg(test)]
#[path = "cursor_tests.rs"]
mod tests;
