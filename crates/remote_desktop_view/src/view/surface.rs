use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use gpui::RenderImage;
use remote_desktop::{RemoteDesktopFrameRect, RgbaFramebuffer};

use crate::pixels::bgra_to_render_image;

use super::frames::copy_bgra_rect_from_framebuffer;

const REMOTE_DESKTOP_TILE_SIZE: u16 = 256;

#[derive(Clone)]
pub(super) struct RemoteDesktopTile {
    pub(super) x: u16,
    pub(super) y: u16,
    pub(super) width: u16,
    pub(super) height: u16,
    pub(super) image: Arc<RenderImage>,
}

pub(super) struct RemoteDesktopSurface {
    id: usize,
    width: u16,
    height: u16,
    tiles: Vec<RemoteDesktopTile>,
}

impl PartialEq for RemoteDesktopSurface {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for RemoteDesktopSurface {}

impl RemoteDesktopSurface {
    pub(super) fn from_framebuffer(framebuffer: &RgbaFramebuffer) -> anyhow::Result<Self> {
        let width = framebuffer.width();
        let height = framebuffer.height();
        anyhow::ensure!(width > 0 && height > 0, "remote desktop frame is empty");

        let mut tiles = Vec::with_capacity(tile_count(width, height));
        for y in (0..height).step_by(usize::from(REMOTE_DESKTOP_TILE_SIZE)) {
            for x in (0..width).step_by(usize::from(REMOTE_DESKTOP_TILE_SIZE)) {
                let tile_width = REMOTE_DESKTOP_TILE_SIZE.min(width - x);
                let tile_height = REMOTE_DESKTOP_TILE_SIZE.min(height - y);
                tiles.push(build_tile(framebuffer, x, y, tile_width, tile_height)?);
            }
        }

        Ok(Self {
            id: next_surface_id(),
            width,
            height,
            tiles,
        })
    }

    pub(super) fn with_dirty_rects(
        &self,
        framebuffer: &RgbaFramebuffer,
        rects: &[RemoteDesktopFrameRect],
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(
            self.width == framebuffer.width() && self.height == framebuffer.height(),
            "remote desktop surface dimensions changed"
        );

        let columns = tile_columns(self.width);
        let mut dirty_tiles = vec![false; self.tiles.len()];
        for rect in rects {
            anyhow::ensure!(
                rect.width > 0 && rect.height > 0,
                "dirty rectangle is empty"
            );
            let right = usize::from(rect.x)
                .checked_add(usize::from(rect.width))
                .ok_or_else(|| anyhow::anyhow!("dirty rectangle width overflows framebuffer"))?;
            let bottom = usize::from(rect.y)
                .checked_add(usize::from(rect.height))
                .ok_or_else(|| anyhow::anyhow!("dirty rectangle height overflows framebuffer"))?;
            anyhow::ensure!(
                right <= usize::from(self.width),
                "dirty rectangle width exceeds framebuffer"
            );
            anyhow::ensure!(
                bottom <= usize::from(self.height),
                "dirty rectangle height exceeds framebuffer"
            );
            let first_column = usize::from(rect.x) / usize::from(REMOTE_DESKTOP_TILE_SIZE);
            let last_column = (right - 1) / usize::from(REMOTE_DESKTOP_TILE_SIZE);
            let first_row = usize::from(rect.y) / usize::from(REMOTE_DESKTOP_TILE_SIZE);
            let last_row = (bottom - 1) / usize::from(REMOTE_DESKTOP_TILE_SIZE);
            for row in first_row..=last_row {
                for column in first_column..=last_column {
                    dirty_tiles[row * columns + column] = true;
                }
            }
        }

        let mut tiles = self.tiles.clone();
        for (index, dirty) in dirty_tiles.into_iter().enumerate() {
            if !dirty {
                continue;
            }
            let tile = &self.tiles[index];
            tiles[index] = build_tile(framebuffer, tile.x, tile.y, tile.width, tile.height)?;
        }

        Ok(Self {
            id: next_surface_id(),
            width: self.width,
            height: self.height,
            tiles,
        })
    }

    pub(super) fn width(&self) -> u16 {
        self.width
    }

    pub(super) fn height(&self) -> u16 {
        self.height
    }

    pub(super) fn tiles(&self) -> &[RemoteDesktopTile] {
        &self.tiles
    }

    pub(super) fn unshared_images(&self) -> Vec<Arc<RenderImage>> {
        self.tiles
            .iter()
            .filter(|tile| Arc::strong_count(&tile.image) == 1)
            .map(|tile| tile.image.clone())
            .collect()
    }

    pub(super) fn images(&self) -> impl Iterator<Item = &Arc<RenderImage>> {
        self.tiles.iter().map(|tile| &tile.image)
    }
}

fn next_surface_id() -> usize {
    static NEXT_SURFACE_ID: AtomicUsize = AtomicUsize::new(0);
    NEXT_SURFACE_ID.fetch_add(1, Ordering::SeqCst)
}

fn tile_columns(width: u16) -> usize {
    usize::from(width.div_ceil(REMOTE_DESKTOP_TILE_SIZE))
}

fn tile_count(width: u16, height: u16) -> usize {
    tile_columns(width) * usize::from(height.div_ceil(REMOTE_DESKTOP_TILE_SIZE))
}

fn build_tile(
    framebuffer: &RgbaFramebuffer,
    x: u16,
    y: u16,
    width: u16,
    height: u16,
) -> anyhow::Result<RemoteDesktopTile> {
    let bgra = copy_bgra_rect_from_framebuffer(framebuffer, x, y, width, height)?;
    let image = Arc::new(bgra_to_render_image(width, height, bgra)?);
    Ok(RemoteDesktopTile {
        x,
        y,
        width,
        height,
        image,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn framebuffer(width: u16, height: u16) -> RgbaFramebuffer {
        let mut bytes = Vec::with_capacity(usize::from(width) * usize::from(height) * 4);
        for index in 0..usize::from(width) * usize::from(height) {
            bytes.extend_from_slice(&[index as u8, 0, 0, 255]);
        }
        RgbaFramebuffer::from_bgra(width, height, bytes).unwrap()
    }

    #[test]
    fn full_surface_is_split_into_bounded_tiles() {
        let surface =
            RemoteDesktopSurface::from_framebuffer(&framebuffer(REMOTE_DESKTOP_TILE_SIZE + 1, 2))
                .unwrap();

        assert_eq!(2, surface.tiles().len());
        assert_eq!(
            (
                REMOTE_DESKTOP_TILE_SIZE,
                2,
                usize::from(REMOTE_DESKTOP_TILE_SIZE) * 2 * 4
            ),
            (
                surface.tiles()[0].width,
                surface.tiles()[0].height,
                surface.tiles()[0].image.as_bytes(0).unwrap().len()
            )
        );
        assert_eq!(
            (1, 2),
            (surface.tiles()[1].width, surface.tiles()[1].height)
        );
    }

    #[test]
    fn dirty_update_reuses_untouched_tile_images() {
        let mut framebuffer = framebuffer(REMOTE_DESKTOP_TILE_SIZE + 1, 1);
        let surface = RemoteDesktopSurface::from_framebuffer(&framebuffer).unwrap();
        let original_left = surface.tiles()[0].image.clone();
        let original_right = surface.tiles()[1].image.clone();
        let rect = RemoteDesktopFrameRect {
            x: REMOTE_DESKTOP_TILE_SIZE,
            y: 0,
            width: 1,
            height: 1,
            byte_len: 4,
        };
        framebuffer
            .patch_rgba_rect(rect.x, rect.y, rect.width, rect.height, &[9, 8, 7, 255])
            .unwrap();

        let updated = surface.with_dirty_rects(&framebuffer, &[rect]).unwrap();

        assert_eq!(original_left, updated.tiles()[0].image);
        assert_ne!(original_right, updated.tiles()[1].image);
        assert_eq!(
            &[9, 8, 7, 255],
            updated.tiles()[1].image.as_bytes(0).unwrap()
        );
    }

    #[test]
    fn dirty_rect_crossing_tile_boundary_refreshes_both_tiles() {
        let mut framebuffer = framebuffer(REMOTE_DESKTOP_TILE_SIZE + 1, 1);
        let surface = RemoteDesktopSurface::from_framebuffer(&framebuffer).unwrap();
        let rect = RemoteDesktopFrameRect {
            x: REMOTE_DESKTOP_TILE_SIZE - 1,
            y: 0,
            width: 2,
            height: 1,
            byte_len: 8,
        };
        framebuffer
            .patch_rgba_rect(
                rect.x,
                rect.y,
                rect.width,
                rect.height,
                &[1, 2, 3, 255, 4, 5, 6, 255],
            )
            .unwrap();

        let updated = surface.with_dirty_rects(&framebuffer, &[rect]).unwrap();

        assert_ne!(surface.tiles()[0].image, updated.tiles()[0].image);
        assert_ne!(surface.tiles()[1].image, updated.tiles()[1].image);
    }

    #[test]
    fn invalid_dirty_rect_is_rejected_without_panicking() {
        let framebuffer = framebuffer(2, 1);
        let surface = RemoteDesktopSurface::from_framebuffer(&framebuffer).unwrap();
        let rect = RemoteDesktopFrameRect {
            x: 2,
            y: 0,
            width: 1,
            height: 1,
            byte_len: 4,
        };

        assert!(surface.with_dirty_rects(&framebuffer, &[rect]).is_err());
    }

    #[test]
    fn empty_dirty_rect_is_rejected_without_panicking() {
        let framebuffer = framebuffer(2, 1);
        let surface = RemoteDesktopSurface::from_framebuffer(&framebuffer).unwrap();
        let rect = RemoteDesktopFrameRect {
            x: 0,
            y: 0,
            width: 0,
            height: 1,
            byte_len: 0,
        };

        assert!(surface.with_dirty_rects(&framebuffer, &[rect]).is_err());
    }
}
