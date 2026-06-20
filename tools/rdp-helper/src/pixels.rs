pub fn rdp_u32_pixels_to_rgba(buffer: &[u32]) -> Vec<u8> {
    let mut rgba = Vec::with_capacity(buffer.len() * 4);

    for pixel in buffer {
        let [_, r, g, b] = pixel.to_be_bytes();
        rgba.extend_from_slice(&[r, g, b, 0xff]);
    }

    rgba
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_ironrdp_pixels_to_rgba() {
        assert_eq!(
            rdp_u32_pixels_to_rgba(&[0x00112233, 0x00abcdef]),
            vec![0x11, 0x22, 0x33, 0xff, 0xab, 0xcd, 0xef, 0xff]
        );
    }
}
