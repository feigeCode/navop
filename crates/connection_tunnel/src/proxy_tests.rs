use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::timeout;

use crate::{ProxyTunnelConfig, ProxyTunnelType, TunnelGuard, start_proxy_tunnel};

#[test]
fn proxy_config_requires_host_and_port() {
    let missing_host = ProxyTunnelConfig {
        proxy_type: ProxyTunnelType::Socks5,
        host: "  ".to_string(),
        port: 1080,
        username: None,
        password: None,
    };
    assert_eq!(Some("host"), missing_host.validate().unwrap_err().field());

    let missing_port = ProxyTunnelConfig {
        host: "127.0.0.1".to_string(),
        port: 0,
        ..missing_host
    };
    assert_eq!(Some("port"), missing_port.validate().unwrap_err().field());
}

#[test]
fn proxy_config_requires_username_when_password_is_set() {
    let config = ProxyTunnelConfig {
        proxy_type: ProxyTunnelType::Http,
        host: "proxy.example.com".to_string(),
        port: 8080,
        username: None,
        password: Some("secret".to_string()),
    };

    assert_eq!(Some("username"), config.validate().unwrap_err().field());
}

#[test]
fn proxy_config_maps_trimmed_credentials_to_ssh_proxy() {
    let config = ProxyTunnelConfig {
        proxy_type: ProxyTunnelType::Socks5,
        host: " proxy.example.com ".to_string(),
        port: 1080,
        username: Some(" alice ".to_string()),
        password: Some("secret".to_string()),
    };

    let proxy = config.to_ssh_proxy().expect("proxy config should be valid");

    assert!(proxy.proxy_type == ssh::ProxyType::Socks5);
    assert_eq!("proxy.example.com", proxy.host);
    assert_eq!(1080, proxy.port);
    assert_eq!(Some("alice".to_string()), proxy.username);
    assert_eq!(Some("secret".to_string()), proxy.password);
}

#[test]
fn tunnel_guard_reports_proxy_local_address() {
    let tunnel = start_proxy_tunnel(
        ProxyTunnelConfig {
            proxy_type: ProxyTunnelType::Socks5,
            host: "127.0.0.1".to_string(),
            port: 9,
            username: None,
            password: None,
        },
        "db.internal",
        5432,
    )
    .expect("proxy tunnel listener should start without connecting eagerly");
    let local_addr = tunnel.local_addr();

    let guard = TunnelGuard::Proxy(tunnel);

    assert_eq!(local_addr, guard.local_addr());
}

#[tokio::test(flavor = "multi_thread")]
async fn http_proxy_tunnel_forwards_bytes_to_requested_target() {
    let target = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let target_addr = target.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut stream, _) = target.accept().await.unwrap();
        let mut request = [0_u8; 4];
        stream.read_exact(&mut request).await.unwrap();
        assert_eq!(b"ping", &request);
        stream.write_all(b"pong").await.unwrap();
    });

    let proxy = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_addr = proxy.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut inbound, _) = proxy.accept().await.unwrap();
        let request = read_http_headers(&mut inbound).await;
        assert!(request.starts_with("CONNECT db.internal:5432 HTTP/1.1\r\n"));
        let mut outbound = TcpStream::connect(target_addr).await.unwrap();
        inbound
            .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
            .await
            .unwrap();
        tokio::io::copy_bidirectional(&mut inbound, &mut outbound)
            .await
            .unwrap();
    });

    let tunnel = start_proxy_tunnel(
        ProxyTunnelConfig {
            proxy_type: ProxyTunnelType::Http,
            host: proxy_addr.ip().to_string(),
            port: proxy_addr.port(),
            username: None,
            password: None,
        },
        "db.internal",
        5432,
    )
    .expect("proxy tunnel should start");

    assert!(tunnel.local_addr().ip().is_loopback());
    let mut client = TcpStream::connect(tunnel.local_addr()).await.unwrap();
    client.write_all(b"ping").await.unwrap();
    let mut response = [0_u8; 4];
    timeout(Duration::from_secs(2), client.read_exact(&mut response))
        .await
        .expect("proxy tunnel response should not time out")
        .unwrap();
    assert_eq!(b"pong", &response);
}

async fn read_http_headers(stream: &mut TcpStream) -> String {
    let mut bytes = Vec::new();
    while !bytes.ends_with(b"\r\n\r\n") {
        let mut byte = [0_u8; 1];
        stream.read_exact(&mut byte).await.unwrap();
        bytes.push(byte[0]);
    }
    String::from_utf8(bytes).unwrap()
}
