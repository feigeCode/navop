use crate::http::{PreparedRequest, RequestMethod};
use crate::tcp::{decode_payload, prepare_tcp_request, target_from_url};

fn prepared_request(url: &str) -> PreparedRequest {
    PreparedRequest {
        method: RequestMethod::Post,
        url: url.into(),
        headers: vec![("x-ignored".into(), "value".into())],
        body: b"initial".to_vec(),
    }
}

#[test]
fn tcp_target_accepts_url_and_bare_host_port() {
    assert_eq!(
        target_from_url("tcp://127.0.0.1:9000/path?debug=1").unwrap(),
        "127.0.0.1:9000"
    );
    assert_eq!(target_from_url("localhost:7000").unwrap(), "localhost:7000");
    assert_eq!(target_from_url("tcp://[::1]:6000").unwrap(), "[::1]:6000");
}

#[test]
fn tcp_target_rejects_missing_port() {
    assert!(target_from_url("tcp://localhost").is_err());
    assert!(target_from_url("localhost").is_err());
}

#[test]
fn payload_supports_text_and_hex_input() {
    assert_eq!(decode_payload("hello").unwrap(), b"hello");
    assert_eq!(decode_payload("0x00 ff 10").unwrap(), vec![0, 255, 16]);
    assert!(decode_payload("0x0").is_err());
    assert!(decode_payload("0xnot-hex").is_err());
}

#[test]
fn prepare_tcp_request_normalizes_scheme_without_losing_initial_payload() {
    let request = prepare_tcp_request(prepared_request("localhost:7000"));

    assert_eq!(request.url, "tcp://localhost:7000");
    assert_eq!(request.body, b"initial");
}
