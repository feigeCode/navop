use gpui::RenderImage;
use image::{ImageBuffer, Rgba};

pub fn rgba_to_render_image(width: u16, height: u16, rgba: Vec<u8>) -> anyhow::Result<RenderImage> {
    let image = ImageBuffer::<Rgba<u8>, _>::from_vec(width as u32, height as u32, rgba)
        .ok_or_else(|| anyhow::anyhow!("invalid RGBA frame buffer length"))?;

    Ok(RenderImage::new(smallvec::SmallVec::from_elem(
        image::Frame::new(image),
        1,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_rgba_buffer_length() {
        let result = rgba_to_render_image(2, 2, vec![0; 3]);

        assert!(result.is_err());
    }
}
