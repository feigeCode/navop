use x11_forwarding::{MagicCookie, ServerEndpoint};

use super::{encode_xauth, write_xauth};

#[test]
fn tcp_record_contains_loopback_display_and_cookie() {
    let cookie = MagicCookie::from_slice(&[7; 16]).unwrap();
    let endpoint = ServerEndpoint::Inet {
        host: "127.0.0.1".into(),
        port: 6012,
    };
    let bytes = encode_xauth(&endpoint, &cookie).unwrap();
    let mut expected = vec![0, 0, 0, 4, 127, 0, 0, 1, 0, 2, b'1', b'2', 0, 18];
    expected.extend_from_slice(b"MIT-MAGIC-COOKIE-1");
    expected.extend_from_slice(&[0, 16]);
    expected.extend_from_slice(&[7; 16]);
    assert_eq!(bytes, expected);
}

#[test]
fn unix_record_uses_local_family() {
    let cookie = MagicCookie::from_slice(&[9; 16]).unwrap();
    let endpoint = ServerEndpoint::Unix("/tmp/.X11-unix/X23".into());
    let bytes = encode_xauth(&endpoint, &cookie).unwrap();
    assert_eq!(&bytes[..2], &256u16.to_be_bytes());
    assert!(bytes.windows(2).any(|window| window == b"23"));
}

#[cfg(unix)]
#[test]
fn unix_record_uses_the_actual_local_hostname() {
    let cookie = MagicCookie::from_slice(&[10; 16]).unwrap();
    let endpoint = ServerEndpoint::Unix("/tmp/.X11-unix/X24".into());
    let bytes = encode_xauth(&endpoint, &cookie).unwrap();
    let hostname = super::local_hostname().unwrap();
    assert!(
        bytes
            .windows(hostname.len())
            .any(|window| window == hostname)
    );
}

#[test]
fn xauth_file_is_owner_read_write_only() {
    let root = std::env::temp_dir().join(format!("x11-xauth-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("xauthority");
    let cookie = MagicCookie::from_slice(&[3; 16]).unwrap();
    write_xauth(
        &path,
        &ServerEndpoint::Inet {
            host: "127.0.0.1".into(),
            port: 6000,
        },
        &cookie,
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    std::fs::remove_dir_all(root).unwrap();
}
