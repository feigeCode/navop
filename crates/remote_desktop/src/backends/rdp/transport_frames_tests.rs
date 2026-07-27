use crate::RemoteDesktopFrameRect;

use super::*;

#[test]
fn reads_binary_frame_event_from_helper_stream() {
    let mut input = std::io::Cursor::new(
        b"{\"type\":\"FrameBytes\",\"width\":2,\"height\":1,\"rgba_len\":8}\n\
          \x01\x02\x03\xff\x04\x05\x06\xff"
            .to_vec(),
    );

    let output = read_output(&mut input);

    assert_eq!(
        RemoteDesktopOutput::Frame {
            width: 2,
            height: 1,
            rgba: vec![1, 2, 3, 255, 4, 5, 6, 255],
        },
        output
    );
}

#[test]
fn reads_bgra_frame_event_from_helper_stream() {
    let mut input = std::io::Cursor::new(
        b"{\"type\":\"FrameBgraBytes\",\"width\":2,\"height\":1,\"bgra_len\":8}\n\
          \x03\x02\x01\xff\x06\x05\x04\xff"
            .to_vec(),
    );

    let output = read_output(&mut input);

    assert_eq!(
        RemoteDesktopOutput::FrameBgra {
            width: 2,
            height: 1,
            bgra: vec![3, 2, 1, 255, 6, 5, 4, 255],
        },
        output
    );
}

#[test]
fn reads_bgra_rectangles_from_helper_stream() {
    let mut input = std::io::Cursor::new(
        b"{\"type\":\"FrameBgraRects\",\"width\":2,\"height\":2,\"rects\":[{\"x\":1,\"y\":1,\"width\":1,\"height\":1,\"byte_len\":4}],\"bgra_len\":4}\n\
          \x03\x02\x01\xff"
            .to_vec(),
    );

    let output = read_output(&mut input);

    assert_eq!(
        RemoteDesktopOutput::FrameBgraRects {
            width: 2,
            height: 2,
            rects: vec![RemoteDesktopFrameRect {
                x: 1,
                y: 1,
                width: 1,
                height: 1,
                byte_len: 4,
            }],
            bgra: vec![3, 2, 1, 255],
        },
        output
    );
}

#[test]
fn reads_legacy_base64_frame_event_from_helper_stream() {
    let mut input = std::io::Cursor::new(
        b"{\"type\":\"Frame\",\"width\":2,\"height\":1,\"rgba_base64\":\"AQID/wQFBv8=\"}\n"
            .to_vec(),
    );

    let output = read_output(&mut input);

    assert_eq!(
        RemoteDesktopOutput::Frame {
            width: 2,
            height: 1,
            rgba: vec![1, 2, 3, 255, 4, 5, 6, 255],
        },
        output
    );
}

fn read_output(input: &mut impl BufRead) -> RemoteDesktopOutput {
    read_helper_output(input, RemoteDesktopProtocol::Rdp)
        .expect("helper output reads")
        .expect("helper output exists")
        .output
}
