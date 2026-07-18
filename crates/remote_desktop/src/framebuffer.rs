#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RgbaFramebuffer {
    width: u16,
    height: u16,
    rgba: Vec<u8>,
}

impl RgbaFramebuffer {
    pub fn new(width: u16, height: u16) -> Self {
        let len = width as usize * height as usize * 4;
        Self {
            width,
            height,
            rgba: vec![0; len],
        }
    }

    pub fn from_bgra(width: u16, height: u16, bgra: Vec<u8>) -> anyhow::Result<Self> {
        let expected = usize::from(width) * usize::from(height) * 4;
        anyhow::ensure!(bgra.len() == expected, "invalid framebuffer length");
        Ok(Self {
            width,
            height,
            rgba: bgra,
        })
    }

    pub fn width(&self) -> u16 {
        self.width
    }

    pub fn height(&self) -> u16 {
        self.height
    }

    pub fn as_rgba(&self) -> &[u8] {
        &self.rgba
    }

    pub fn clone_rgba(&self) -> Vec<u8> {
        self.rgba.clone()
    }

    pub fn patch_rgba_rect(
        &mut self,
        x: u16,
        y: u16,
        width: u16,
        height: u16,
        rect_rgba: &[u8],
    ) -> anyhow::Result<()> {
        let expected = width as usize * height as usize * 4;
        anyhow::ensure!(
            rect_rgba.len() == expected,
            "invalid rectangle buffer length"
        );
        anyhow::ensure!(x <= self.width, "rectangle x is outside framebuffer");
        anyhow::ensure!(y <= self.height, "rectangle y is outside framebuffer");
        anyhow::ensure!(
            x.saturating_add(width) <= self.width,
            "rectangle width exceeds framebuffer"
        );
        anyhow::ensure!(
            y.saturating_add(height) <= self.height,
            "rectangle height exceeds framebuffer"
        );

        for row in 0..height as usize {
            let src_start = row * width as usize * 4;
            let src_end = src_start + width as usize * 4;
            let dst_start = ((y as usize + row) * self.width as usize + x as usize) * 4;
            let dst_end = dst_start + width as usize * 4;
            self.rgba[dst_start..dst_end].copy_from_slice(&rect_rgba[src_start..src_end]);
        }

        Ok(())
    }

    pub fn copy_rect(
        &mut self,
        src_x: u16,
        src_y: u16,
        dst_x: u16,
        dst_y: u16,
        width: u16,
        height: u16,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(src_x <= self.width, "source x is outside framebuffer");
        anyhow::ensure!(src_y <= self.height, "source y is outside framebuffer");
        anyhow::ensure!(dst_x <= self.width, "destination x is outside framebuffer");
        anyhow::ensure!(dst_y <= self.height, "destination y is outside framebuffer");
        anyhow::ensure!(
            src_x.saturating_add(width) <= self.width,
            "source width exceeds framebuffer"
        );
        anyhow::ensure!(
            src_y.saturating_add(height) <= self.height,
            "source height exceeds framebuffer"
        );
        anyhow::ensure!(
            dst_x.saturating_add(width) <= self.width,
            "destination width exceeds framebuffer"
        );
        anyhow::ensure!(
            dst_y.saturating_add(height) <= self.height,
            "destination height exceeds framebuffer"
        );

        let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);
        for row in 0..height as usize {
            let start = ((src_y as usize + row) * self.width as usize + src_x as usize) * 4;
            let end = start + width as usize * 4;
            pixels.extend_from_slice(&self.rgba[start..end]);
        }
        self.patch_rgba_rect(dst_x, dst_y, width, height, &pixels)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patch_rgba_rect_updates_only_target_region() {
        let mut fb = RgbaFramebuffer::new(3, 2);
        let red = [255, 0, 0, 255, 255, 0, 0, 255];

        fb.patch_rgba_rect(1, 1, 2, 1, &red).unwrap();

        assert_eq!(
            fb.as_rgba(),
            &[
                0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 255, 0, 0, 255, 255, 0, 0, 255,
            ]
        );
    }

    #[test]
    fn copy_rect_uses_source_snapshot_before_writing_destination() {
        let mut fb = RgbaFramebuffer::new(3, 1);
        let pixels = [1, 0, 0, 255, 2, 0, 0, 255, 3, 0, 0, 255];
        fb.patch_rgba_rect(0, 0, 3, 1, &pixels).unwrap();

        fb.copy_rect(0, 0, 1, 0, 2, 1).unwrap();

        assert_eq!(fb.as_rgba(), &[1, 0, 0, 255, 1, 0, 0, 255, 2, 0, 0, 255]);
    }
}
