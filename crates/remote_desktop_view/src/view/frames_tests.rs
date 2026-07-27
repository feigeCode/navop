use remote_desktop::{RemoteDesktopFrameRect, RgbaFramebuffer};

use super::patched_bgra_framebuffer;

#[test]
fn patches_dirty_rectangles_without_mutating_the_base() {
    let base =
        RgbaFramebuffer::from_bgra(2, 1, vec![0x03, 0x02, 0x01, 0xff, 0x06, 0x05, 0x04, 0xff])
            .unwrap();
    let rects = [RemoteDesktopFrameRect {
        x: 1,
        y: 0,
        width: 1,
        height: 1,
        byte_len: 4,
    }];

    let patched = patched_bgra_framebuffer(&base, 2, 1, &rects, &[0x30, 0x20, 0x10, 0xff]).unwrap();

    assert_eq!(
        base.as_rgba(),
        &[0x03, 0x02, 0x01, 0xff, 0x06, 0x05, 0x04, 0xff]
    );
    assert_eq!(
        patched.as_rgba(),
        &[0x03, 0x02, 0x01, 0xff, 0x30, 0x20, 0x10, 0xff]
    );
}

#[test]
fn rejects_an_invalid_delta_atomically() {
    let base = RgbaFramebuffer::from_bgra(2, 1, vec![0x03, 0x02, 0x01, 0xff, 0, 0, 0, 0]).unwrap();
    let rects = [
        RemoteDesktopFrameRect {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
            byte_len: 4,
        },
        RemoteDesktopFrameRect {
            x: 2,
            y: 0,
            width: 1,
            height: 1,
            byte_len: 4,
        },
    ];

    let result = patched_bgra_framebuffer(
        &base,
        2,
        1,
        &rects,
        &[0x30, 0x20, 0x10, 0xff, 0x60, 0x50, 0x40, 0xff],
    );

    assert!(result.is_err());
    assert_eq!(
        base.as_rgba(),
        &[0x03, 0x02, 0x01, 0xff, 0, 0, 0, 0],
        "a rejected delta must not partially patch its base"
    );
}

#[test]
fn rejects_delta_payload_with_trailing_bytes() {
    let base = RgbaFramebuffer::from_bgra(1, 1, vec![0, 0, 0, 0]).unwrap();
    let rects = [RemoteDesktopFrameRect {
        x: 0,
        y: 0,
        width: 1,
        height: 1,
        byte_len: 4,
    }];

    assert!(patched_bgra_framebuffer(&base, 1, 1, &rects, &[1, 2, 3, 4, 5]).is_err());
    assert_eq!(base.as_rgba(), &[0, 0, 0, 0]);
}
