//! 本机 X server 的 X11 setup 健康检查。

use std::io;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::setup::ClientHello;
use crate::{MagicCookie, ServerEndpoint, X11Error, X11Result};

const REPLY_HEADER_LEN: usize = 8;
const MAX_REPLY_BODY_LEN: usize = 1024 * 1024;

trait ProbeStream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> ProbeStream for T {}

/// 用真实 cookie 完成一次 setup 往返；成功应答的可变长度正文也必须完整到达。
pub async fn probe_endpoint(
    endpoint: &ServerEndpoint,
    cookie: &MagicCookie,
    timeout: Duration,
) -> X11Result<()> {
    let packet = ClientHello::for_probe(cookie.bytes()).encode_with(cookie.bytes());
    probe_with_setup(endpoint, packet, timeout).await
}

/// Windows 本机无认证 X server 的健康检查；不要伪造 MIT cookie。
#[cfg(any(target_os = "windows", test))]
pub(crate) async fn probe_endpoint_without_auth(
    endpoint: &ServerEndpoint,
    timeout: Duration,
) -> X11Result<()> {
    let packet = ClientHello::for_probe(&[]).encode_with_auth("", &[]);
    probe_with_setup(endpoint, packet, timeout).await
}

async fn probe_with_setup(
    endpoint: &ServerEndpoint,
    packet: Vec<u8>,
    timeout: Duration,
) -> X11Result<()> {
    tokio::time::timeout(timeout, probe_inner(endpoint, packet))
        .await
        .map_err(|_| X11Error::ProbeTimeout)?
}

async fn probe_inner(endpoint: &ServerEndpoint, packet: Vec<u8>) -> X11Result<()> {
    let mut stream = connect(endpoint).await?;
    stream.write_all(&packet).await.map_err(io_error)?;
    let mut header = [0; REPLY_HEADER_LEN];
    stream.read_exact(&mut header).await.map_err(io_error)?;
    let body = read_reply_body(&mut stream, &header).await?;
    match header[0] {
        1 => Ok(()),
        0 | 2 => Err(X11Error::ProbeRejected(reply_reason(&header, &body))),
        status => Err(X11Error::ProbeRejected(format!("未知 setup 状态 {status}"))),
    }
}

async fn connect(endpoint: &ServerEndpoint) -> X11Result<Box<dyn ProbeStream>> {
    match endpoint {
        ServerEndpoint::Inet { host, port } => {
            let stream = TcpStream::connect((host.as_str(), *port))
                .await
                .map_err(io_error)?;
            Ok(Box::new(stream))
        }
        #[cfg(unix)]
        ServerEndpoint::Unix(path) => {
            let stream = tokio::net::UnixStream::connect(path)
                .await
                .map_err(io_error)?;
            Ok(Box::new(stream))
        }
        #[cfg(not(unix))]
        ServerEndpoint::Unix(path) => Err(X11Error::ProbeIo(format!(
            "当前平台不支持 Unix socket：{}",
            path.display()
        ))),
    }
}

async fn read_reply_body(
    stream: &mut (impl ProbeStream + ?Sized),
    header: &[u8; REPLY_HEADER_LEN],
) -> X11Result<Vec<u8>> {
    let body_len = u16::from_le_bytes([header[6], header[7]]) as usize * 4;
    if body_len > MAX_REPLY_BODY_LEN {
        return Err(X11Error::ProbeRejected(format!(
            "setup 应答超过 {MAX_REPLY_BODY_LEN} 字节"
        )));
    }
    let mut body = vec![0; body_len];
    stream.read_exact(&mut body).await.map_err(io_error)?;
    Ok(body)
}

fn reply_reason(header: &[u8; REPLY_HEADER_LEN], body: &[u8]) -> String {
    let reason_len = usize::from(header[1]).min(body.len());
    String::from_utf8_lossy(&body[..reason_len]).into_owned()
}

fn io_error(error: io::Error) -> X11Error {
    X11Error::ProbeIo(error.to_string())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    use super::{probe_endpoint, probe_endpoint_without_auth};
    use crate::setup::ClientHello;
    use crate::{MagicCookie, ServerEndpoint, X11Error};

    async fn tcp_server(reply: Vec<u8>) -> (ServerEndpoint, MagicCookie) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let endpoint = ServerEndpoint::Inet {
            host: "127.0.0.1".into(),
            port: listener.local_addr().unwrap().port(),
        };
        let cookie = MagicCookie::from_slice(&[0x42; 16]).unwrap();
        let expected_cookie = cookie.clone();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut header = [0; 12];
            stream.read_exact(&mut header).await.unwrap();
            let auth_len = ClientHello::auth_section_len(&header).unwrap();
            let mut packet = header.to_vec();
            packet.resize(12 + auth_len, 0);
            stream.read_exact(&mut packet[12..]).await.unwrap();
            let hello = ClientHello::decode(&packet).unwrap();
            assert_eq!(hello.presented_cookie().unwrap(), expected_cookie);
            stream.write_all(&reply).await.unwrap();
        });
        (endpoint, cookie)
    }

    #[tokio::test]
    async fn probe_completes_real_setup_handshake() {
        let (endpoint, cookie) = tcp_server(vec![1, 0, 11, 0, 0, 0, 0, 0]).await;
        probe_endpoint(&endpoint, &cookie, Duration::from_secs(1))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn probe_surfaces_protocol_rejection() {
        let (endpoint, cookie) = tcp_server(vec![0, 0, 11, 0, 0, 0, 0, 0]).await;
        let error = probe_endpoint(&endpoint, &cookie, Duration::from_secs(1))
            .await
            .unwrap_err();
        assert!(matches!(error, X11Error::ProbeRejected(_)));
    }

    #[tokio::test]
    async fn probe_rejects_truncated_success() {
        let (endpoint, cookie) = tcp_server(vec![1, 0, 11, 0]).await;
        let error = probe_endpoint(&endpoint, &cookie, Duration::from_secs(1))
            .await
            .unwrap_err();
        assert!(matches!(error, X11Error::ProbeIo(_)));
    }

    #[tokio::test]
    async fn no_auth_probe_omits_authentication_instead_of_forging_a_cookie() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let endpoint = ServerEndpoint::Inet {
            host: "127.0.0.1".into(),
            port: listener.local_addr().unwrap().port(),
        };
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut header = [0; 12];
            stream.read_exact(&mut header).await.unwrap();
            assert_eq!(ClientHello::auth_section_len(&header).unwrap(), 0);
            let hello = ClientHello::decode(&header).unwrap();
            assert!(hello.auth_name.is_empty());
            assert!(hello.auth_data.is_empty());
            stream.write_all(&[1, 0, 11, 0, 0, 0, 0, 0]).await.unwrap();
        });

        probe_endpoint_without_auth(&endpoint, Duration::from_secs(1))
            .await
            .unwrap();
        server.await.unwrap();
    }

    #[tokio::test]
    async fn probe_times_out_when_server_never_replies() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let endpoint = ServerEndpoint::Inet {
            host: "127.0.0.1".into(),
            port: listener.local_addr().unwrap().port(),
        };
        let cookie = MagicCookie::from_slice(&[0x42; 16]).unwrap();
        let server = tokio::spawn(async move {
            let (_stream, _) = listener.accept().await.unwrap();
            tokio::time::sleep(Duration::from_secs(1)).await;
        });

        let error = probe_endpoint(&endpoint, &cookie, Duration::from_millis(20))
            .await
            .unwrap_err();
        assert!(matches!(error, X11Error::ProbeTimeout));
        server.abort();
    }
}
