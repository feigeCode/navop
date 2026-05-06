use crate::plugin::Plugin;
use gpui::{
    Bounds, Corners, Element as _, InteractiveElement as _, ParentElement, RenderImage, Size,
    Styled, canvas, div, px,
};
use image::{Frame, RgbaImage};
use smallvec::smallvec;
use std::sync::Arc;

const BASE_GRID: f32 = 40.0;

#[derive(Clone, Copy, PartialEq)]
struct BitmapKey {
    offset_x_mod: i32, // (offset_x % grid * 1000) as i32
    offset_y_mod: i32,
    grid_i: i32, // (grid * 1000) as i32
    width: u32,
    height: u32,
    bg_color: u32,
    dot_color: u32,
}

fn generate_fullscreen_bitmap(
    width: u32,
    height: u32,
    grid: f32,
    start_x: f32,
    start_y: f32,
    bg_color: u32,
    dot_color: u32,
) -> Arc<RenderImage> {
    let w = width as usize;
    let h = height as usize;

    let bg = [
        ((bg_color >> 16) & 0xFF) as u8,
        ((bg_color >> 8) & 0xFF) as u8,
        (bg_color & 0xFF) as u8,
        255u8,
    ];
    let dot = [
        ((dot_color >> 16) & 0xFF) as u8,
        ((dot_color >> 8) & 0xFF) as u8,
        (dot_color & 0xFF) as u8,
        255u8,
    ];

    let mut data = vec![0u8; w * h * 4];
    for i in 0..w * h {
        let p = i * 4;
        data[p] = bg[0];
        data[p + 1] = bg[1];
        data[p + 2] = bg[2];
        data[p + 3] = 255;
    }

    let mut x = start_x;
    while x < width as f32 {
        let mut y = start_y;
        while y < height as f32 {
            for dy in 0..2i32 {
                for dx in 0..2i32 {
                    let px = (x - 1.0 + dx as f32).floor() as isize;
                    let py = (y - 1.0 + dy as f32).floor() as isize;
                    if px >= 0 && py >= 0 && (px as usize) < w && (py as usize) < h {
                        let i = ((py as usize) * w + (px as usize)) * 4;
                        data[i] = dot[0];
                        data[i + 1] = dot[1];
                        data[i + 2] = dot[2];
                        data[i + 3] = 255;
                    }
                }
            }
            y += grid;
        }
        x += grid;
    }

    for chunk in data.chunks_exact_mut(4) {
        chunk.swap(0, 2);
    }

    let img = RgbaImage::from_raw(width, height, data).unwrap();
    Arc::new(RenderImage::new(smallvec![Frame::new(img)]))
}

pub struct BackgroundPlugin {
    bitmap_key: Option<BitmapKey>,
    bitmap: Option<Arc<RenderImage>>,
}

impl Default for BackgroundPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl BackgroundPlugin {
    pub fn new() -> Self {
        Self {
            bitmap_key: None,
            bitmap: None,
        }
    }

    fn sync_bitmap(&mut self, ctx: &crate::plugin::RenderContext) {
        let zoom = ctx.zoom();
        let grid = BASE_GRID * zoom;
        let offset = ctx.offset();
        let offset_x = f32::from(offset.x);
        let offset_y = f32::from(offset.y);
        let bounds = ctx.window.bounds();
        let width = f32::from(bounds.size.width) as u32;
        let height = f32::from(bounds.size.height) as u32;

        if grid <= 0.0 || width == 0 || height == 0 {
            return;
        }

        let ox_mod = offset_x % grid;
        let oy_mod = offset_y % grid;

        let key = BitmapKey {
            offset_x_mod: (ox_mod * 1000.0) as i32,
            offset_y_mod: (oy_mod * 1000.0) as i32,
            grid_i: (grid * 1000.0) as i32,
            width,
            height,
            bg_color: ctx.theme.background,
            dot_color: ctx.theme.background_grid_dot,
        };

        if self.bitmap_key == Some(key) {
            return;
        }

        self.bitmap_key = Some(key);
        self.bitmap = Some(generate_fullscreen_bitmap(
            width,
            height,
            grid,
            ox_mod,
            oy_mod,
            ctx.theme.background,
            ctx.theme.background_grid_dot,
        ));
    }
}

impl Plugin for BackgroundPlugin {
    fn name(&self) -> &'static str {
        "background"
    }
    fn priority(&self) -> i32 {
        0
    }
    fn render_layer(&self) -> crate::plugin::RenderLayer {
        crate::plugin::RenderLayer::Background
    }

    fn render(&mut self, ctx: &mut crate::plugin::RenderContext) -> Option<gpui::AnyElement> {
        self.sync_bitmap(ctx);

        let Some(bitmap) = self.bitmap.as_ref().map(Arc::clone) else {
            return Some(
                div()
                    .id("background")
                    .absolute()
                    .size_full()
                    .bg(gpui::rgb(ctx.theme.background))
                    .into_any(),
            );
        };

        let bounds = ctx.window.bounds();
        let width = f32::from(bounds.size.width);
        let height = f32::from(bounds.size.height);

        let el = canvas(
            move |_, _, _| bitmap,
            move |bounds, bitmap, window, _cx| {
                let _ = window.paint_image(
                    Bounds {
                        origin: bounds.origin,
                        size: Size::new(px(width), px(height)),
                    },
                    Corners::default(),
                    Arc::clone(&bitmap),
                    0,
                    false,
                );
            },
        )
        .absolute()
        .size_full();

        Some(
            div()
                .id("background")
                .absolute()
                .size_full()
                .child(el)
                .into_any(),
        )
    }
}
