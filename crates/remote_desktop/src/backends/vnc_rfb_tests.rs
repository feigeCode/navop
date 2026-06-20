use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::RecvTimeoutError;
use std::time::{Duration, Instant};

use crate::backends::vnc::VncBackend;
use crate::{
    RemoteDesktopBackend, RemoteDesktopCapabilities, RemoteDesktopConnectionOptions,
    RemoteDesktopInput, RemoteDesktopOutput, RemoteDesktopProtocol, RemoteDesktopSize,
    ResizeSupport,
};

#[test]
fn vnc_backend_connects_to_mock_server_and_emits_raw_frame() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock VNC server");
    let destination = listener.local_addr().expect("mock server addr").to_string();
    let server = std::thread::spawn(move || run_mock_vnc_server(listener));
    let runtime = start_vnc_backend(destination);

    let connected = recv_until(&runtime.output_rx, |output| {
        matches!(output, RemoteDesktopOutput::Connected { .. })
    });
    assert_eq!(
        RemoteDesktopOutput::Connected {
            width: 2,
            height: 1,
            capabilities: RemoteDesktopCapabilities {
                resize: ResizeSupport::LocalScaleOnly,
                clipboard_text: true,
                cursor_shape: false,
                audio: false,
                file_transfer: false,
            }
        },
        connected
    );

    let frame = recv_until(&runtime.output_rx, |output| {
        matches!(output, RemoteDesktopOutput::Frame { .. })
    });
    assert_eq!(
        RemoteDesktopOutput::Frame {
            width: 2,
            height: 1,
            rgba: vec![255, 0, 0, 255, 0, 255, 0, 255],
        },
        frame
    );

    runtime
        .input_tx
        .send(RemoteDesktopInput::MouseMove { x: 1, y: 0 })
        .expect("send mouse move");
    server.join().expect("mock server thread joins");
    let _ = runtime.input_tx.send(RemoteDesktopInput::Close);
}

fn start_vnc_backend(destination: String) -> crate::RemoteDesktopRuntime {
    let options = RemoteDesktopConnectionOptions {
        protocol: RemoteDesktopProtocol::Vnc,
        destination,
        username: None,
        password: None,
        domain: None,
        read_only: false,
    };
    Box::new(VncBackend::new(options))
        .start(RemoteDesktopSize {
            width: 2,
            height: 1,
        })
        .expect("start VNC backend")
}

fn recv_until(
    output_rx: &std::sync::mpsc::Receiver<RemoteDesktopOutput>,
    predicate: impl Fn(&RemoteDesktopOutput) -> bool,
) -> RemoteDesktopOutput {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let now = Instant::now();
        assert!(now < deadline, "timed out waiting for VNC output");
        match output_rx.recv_timeout(deadline - now) {
            Ok(output) if predicate(&output) => return output,
            Ok(_) => {}
            Err(RecvTimeoutError::Timeout) => panic!("timed out waiting for VNC output"),
            Err(RecvTimeoutError::Disconnected) => panic!("VNC output channel disconnected"),
        }
    }
}

fn run_mock_vnc_server(listener: TcpListener) {
    let (mut stream, _) = listener.accept().expect("accept VNC client");
    send_protocol_version(&mut stream);
    read_protocol_version(&mut stream);
    send_none_security(&mut stream);
    read_security_choice(&mut stream);
    send_security_success(&mut stream);
    read_client_init(&mut stream);
    send_server_init(&mut stream);
    read_set_pixel_format(&mut stream);
    read_set_encodings(&mut stream);
    read_framebuffer_update_request(&mut stream);
    send_raw_frame(&mut stream);
    read_pointer_event(&mut stream);
}

fn send_protocol_version(stream: &mut TcpStream) {
    stream
        .write_all(b"RFB 003.008\n")
        .expect("send RFB version");
}

fn read_protocol_version(stream: &mut TcpStream) {
    let mut version = [0; 12];
    stream
        .read_exact(&mut version)
        .expect("read client version");
    assert_eq!(&version, b"RFB 003.008\n");
}

fn send_none_security(stream: &mut TcpStream) {
    stream.write_all(&[1, 1]).expect("send None security type");
}

fn read_security_choice(stream: &mut TcpStream) {
    let mut choice = [0; 1];
    stream
        .read_exact(&mut choice)
        .expect("read security choice");
    assert_eq!([1], choice);
}

fn send_security_success(stream: &mut TcpStream) {
    stream
        .write_all(&0u32.to_be_bytes())
        .expect("send security result");
}

fn read_client_init(stream: &mut TcpStream) {
    let mut shared = [0; 1];
    stream.read_exact(&mut shared).expect("read ClientInit");
    assert_eq!([1], shared);
}

fn send_server_init(stream: &mut TcpStream) {
    let mut payload = Vec::new();
    payload.extend_from_slice(&2u16.to_be_bytes());
    payload.extend_from_slice(&1u16.to_be_bytes());
    payload.extend_from_slice(&server_pixel_format());
    payload.extend_from_slice(&0u32.to_be_bytes());
    stream.write_all(&payload).expect("send ServerInit");
}

fn server_pixel_format() -> [u8; 16] {
    [32, 24, 0, 1, 0, 255, 0, 255, 0, 255, 16, 8, 0, 0, 0, 0]
}

fn read_set_pixel_format(stream: &mut TcpStream) {
    let mut message = [0; 20];
    stream
        .read_exact(&mut message)
        .expect("read SetPixelFormat");
    assert_eq!(0, message[0]);
}

fn read_set_encodings(stream: &mut TcpStream) {
    let mut header = [0; 4];
    stream
        .read_exact(&mut header)
        .expect("read SetEncodings header");
    assert_eq!(2, header[0]);
    let count = u16::from_be_bytes([header[2], header[3]]) as usize;
    let mut encodings = vec![0; count * 4];
    stream
        .read_exact(&mut encodings)
        .expect("read SetEncodings payload");
    assert!(encodings.chunks_exact(4).any(|chunk| chunk == [0, 0, 0, 0]));
    assert!(
        encodings
            .chunks_exact(4)
            .any(|chunk| chunk == (-239i32).to_be_bytes())
    );
}

fn read_framebuffer_update_request(stream: &mut TcpStream) {
    let mut request = [0; 10];
    stream
        .read_exact(&mut request)
        .expect("read FramebufferUpdateRequest");
    assert_eq!(3, request[0]);
    assert_eq!(0, request[1]);
    assert_eq!(&[0, 2], &request[6..8]);
    assert_eq!(&[0, 1], &request[8..10]);
}

fn send_raw_frame(stream: &mut TcpStream) {
    let mut payload = vec![0, 0];
    payload.extend_from_slice(&1u16.to_be_bytes());
    payload.extend_from_slice(&0u16.to_be_bytes());
    payload.extend_from_slice(&0u16.to_be_bytes());
    payload.extend_from_slice(&2u16.to_be_bytes());
    payload.extend_from_slice(&1u16.to_be_bytes());
    payload.extend_from_slice(&0i32.to_be_bytes());
    payload.extend_from_slice(&[255, 0, 0, 255, 0, 255, 0, 255]);
    stream.write_all(&payload).expect("send raw frame");
}

fn read_pointer_event(stream: &mut TcpStream) {
    loop {
        let mut message_type = [0; 1];
        stream
            .read_exact(&mut message_type)
            .expect("read client message type");
        match message_type[0] {
            3 => skip_framebuffer_update_request(stream),
            5 => {
                let mut pointer = [0; 5];
                stream.read_exact(&mut pointer).expect("read PointerEvent");
                assert_eq!([0, 0, 1, 0, 0], pointer);
                return;
            }
            other => panic!("unexpected VNC client message type {other}"),
        }
    }
}

fn skip_framebuffer_update_request(stream: &mut TcpStream) {
    let mut rest = [0; 9];
    stream
        .read_exact(&mut rest)
        .expect("skip FramebufferUpdateRequest");
}
