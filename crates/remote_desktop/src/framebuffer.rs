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
}
