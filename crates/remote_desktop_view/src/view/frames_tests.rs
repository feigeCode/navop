use remote_desktop::{RemoteDesktopFrameRect, RgbaFramebuffer};

use super::{apply_bgra_rects_to_framebuffer, copy_bgra_rect_from_framebuffer};

#[test]
fn patches_dirty_rectangles_in_place() {
    let mut framebuffer =
        RgbaFramebuffer::from_bgra(2, 1, vec![0x03, 0x02, 0x01, 0xff, 0x06, 0x05, 0x04, 0xff])
            .unwrap();
    let rects = [RemoteDesktopFrameRect {
        x: 1,
        y: 0,
        width: 1,
        height: 1,
        byte_len: 4,
    }];

    apply_bgra_rects_to_framebuffer(&mut framebuffer, 2, 1, &rects, &[0x30, 0x20, 0x10, 0xff])
        .unwrap();

    assert_eq!(
        framebuffer.as_rgba(),
        &[0x03, 0x02, 0x01, 0xff, 0x30, 0x20, 0x10, 0xff]
    );
}

#[test]
fn rejects_an_invalid_delta_atomically() {
    let mut framebuffer =
        RgbaFramebuffer::from_bgra(2, 1, vec![0x03, 0x02, 0x01, 0xff, 0, 0, 0, 0]).unwrap();
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

    let result = apply_bgra_rects_to_framebuffer(
        &mut framebuffer,
        2,
        1,
        &rects,
        &[0x30, 0x20, 0x10, 0xff, 0x60, 0x50, 0x40, 0xff],
    );

    assert!(result.is_err());
    assert_eq!(
        framebuffer.as_rgba(),
        &[0x03, 0x02, 0x01, 0xff, 0, 0, 0, 0],
        "a rejected delta must not partially patch its base"
    );
}

#[test]
fn rejects_delta_payload_with_trailing_bytes() {
    let mut framebuffer = RgbaFramebuffer::from_bgra(1, 1, vec![0, 0, 0, 0]).unwrap();
    let rects = [RemoteDesktopFrameRect {
        x: 0,
        y: 0,
        width: 1,
        height: 1,
        byte_len: 4,
    }];

    assert!(
        apply_bgra_rects_to_framebuffer(&mut framebuffer, 1, 1, &rects, &[1, 2, 3, 4, 5]).is_err()
    );
    assert_eq!(framebuffer.as_rgba(), &[0, 0, 0, 0]);
}

#[test]
fn rejects_a_rectangle_with_an_incorrect_declared_length_atomically() {
    let mut framebuffer = RgbaFramebuffer::from_bgra(1, 1, vec![0, 0, 0, 0]).unwrap();
    let rects = [RemoteDesktopFrameRect {
        x: 0,
        y: 0,
        width: 1,
        height: 1,
        byte_len: 3,
    }];

    assert!(apply_bgra_rects_to_framebuffer(&mut framebuffer, 1, 1, &rects, &[1, 2, 3]).is_err());
    assert_eq!(framebuffer.as_rgba(), &[0, 0, 0, 0]);
}

#[test]
fn copies_only_the_requested_bgra_rectangle() {
    let framebuffer = RgbaFramebuffer::from_bgra(
        3,
        2,
        vec![
            1, 0, 0, 255, 2, 0, 0, 255, 3, 0, 0, 255, 4, 0, 0, 255, 5, 0, 0, 255, 6, 0, 0, 255,
        ],
    )
    .unwrap();

    assert_eq!(
        vec![2, 0, 0, 255, 3, 0, 0, 255, 5, 0, 0, 255, 6, 0, 0, 255],
        copy_bgra_rect_from_framebuffer(&framebuffer, 1, 0, 2, 2).unwrap()
    );
}
