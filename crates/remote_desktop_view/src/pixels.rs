use gpui::RenderImage;
use image::{ImageBuffer, Rgba};

/// GPUI (Zed) stores `RenderImage` pixels in **BGRA** byte order, even though
/// the buffer type is `image::RgbaImage` (its atlas textures are `Bgra8Unorm`).
/// The remote desktop backend produces normal RGBA, so swap the red and blue
/// channels before handing the buffer to GPUI, otherwise blue Windows chrome
/// renders as yellow/orange.
pub fn rgba_to_render_image(width: u16, height: u16, rgba: Vec<u8>) -> anyhow::Result<RenderImage> {
    let rgba = rgba_to_bgra(rgba);
    let image = ImageBuffer::<Rgba<u8>, _>::from_vec(width as u32, height as u32, rgba)
        .ok_or_else(|| anyhow::anyhow!("invalid RGBA frame buffer length"))?;

    Ok(RenderImage::new(smallvec::SmallVec::from_elem(
        image::Frame::new(image),
        1,
    )))
}

pub(super) fn rgba_to_bgra(mut rgba: Vec<u8>) -> Vec<u8> {
    for pixel in rgba.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    rgba
}

pub fn bgra_to_render_image(width: u16, height: u16, bgra: Vec<u8>) -> anyhow::Result<RenderImage> {
    let image = ImageBuffer::<Rgba<u8>, _>::from_vec(width as u32, height as u32, bgra)
        .ok_or_else(|| anyhow::anyhow!("invalid BGRA frame buffer length"))?;

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

    #[test]
    fn swaps_rgba_to_bgra_for_gpui_atlas() {
        // RGBA blue [0, 0, 255, 255] must become BGRA [255, 0, 0, 255].
        let image = rgba_to_render_image(1, 1, vec![0, 0, 255, 255]).unwrap();

        assert_eq!(image.as_bytes(0).unwrap(), &[255, 0, 0, 255]);
    }

    #[test]
    fn swaps_red_to_bgra() {
        // RGBA red [255, 0, 0, 255] must become BGRA [0, 0, 255, 255].
        let image = rgba_to_render_image(1, 1, vec![255, 0, 0, 255]).unwrap();

        assert_eq!(image.as_bytes(0).unwrap(), &[0, 0, 255, 255]);
    }

    #[test]
    fn converts_rgba_backing_for_delta_presentation() {
        assert_eq!(
            vec![0x33, 0x22, 0x11, 0xff],
            rgba_to_bgra(vec![0x11, 0x22, 0x33, 0xff])
        );
    }

    #[test]
    fn keeps_bgra_buffer_without_channel_swap() {
        let image = bgra_to_render_image(1, 1, vec![0x33, 0x22, 0x11, 0xff]).unwrap();

        assert_eq!(image.as_bytes(0).unwrap(), &[0x33, 0x22, 0x11, 0xff]);
    }
}
