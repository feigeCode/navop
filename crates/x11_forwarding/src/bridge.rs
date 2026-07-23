//! sshd 回连通道与本机 X server 之间的桥接。
//!
//! 读取策略：先 `read_exact` 读 12 字节固定头，按头里的长度再
//! `read_exact` 读认证区——精确读取从不过量，因此认证区之后到达的
//! X11 数据原封不动留在通道里，桥接建立后直接进入双向拷贝。

use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::setup::{ClientHello, HEADER_LEN};
use crate::{MagicCookie, ServerEndpoint, X11Error};

#[derive(Debug, Error)]
pub enum BridgeError {
    #[error(transparent)]
    Protocol(#[from] X11Error),
    #[error("通道 I/O：{0}")]
    Io(#[from] std::io::Error),
}

/// 按出示的 fake cookie 换取对应的本机真实 cookie。
/// 返回 `None` 表示该 cookie 不被接受。
pub trait CookieExchange: Send + Sync {
    fn exchange(&self, presented: &MagicCookie) -> Option<MagicCookie>;
}

impl<F> CookieExchange for F
where
    F: Fn(&MagicCookie) -> Option<MagicCookie> + Send + Sync,
{
    fn exchange(&self, presented: &MagicCookie) -> Option<MagicCookie> {
        self(presented)
    }
}

pub trait AsyncReadWrite: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> AsyncReadWrite for T {}

/// 处理一条 sshd 回连的 x11 通道，直到连接关闭。
pub async fn relay<S>(channel: S, endpoint: &ServerEndpoint, exchange: &dyn CookieExchange)
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    if let Err(error) = relay_inner(channel, endpoint, exchange).await {
        tracing::debug!(target: "ssh.x11", %error, "x11 通道结束");
    }
}

async fn relay_inner<S>(
    mut channel: S,
    endpoint: &ServerEndpoint,
    exchange: &dyn CookieExchange,
) -> Result<(), BridgeError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    // 1. 精确读出 setup 报文（头 + 认证区）。
    let mut header = [0u8; HEADER_LEN];
    channel.read_exact(&mut header).await?;
    let mut body = vec![0u8; ClientHello::auth_section_len(&header)?];
    channel.read_exact(&mut body).await?;

    let mut packet = Vec::with_capacity(HEADER_LEN + body.len());
    packet.extend_from_slice(&header);
    packet.extend_from_slice(&body);
    let hello = ClientHello::decode(&packet)?;

    // 2. 校验 fake cookie；失败则回一个 setup 失败应答再关闭。
    let presented = hello.presented_cookie()?;
    let Some(local) = exchange.exchange(&presented) else {
        reject(&mut channel, &hello).await;
        return Err(X11Error::CookieRejected.into());
    };

    // 3. 换成本机真实 cookie 重新编码，连上本机 X server 后双向转发。
    let rewritten = hello.encode_with(local.bytes());
    let mut server = open(endpoint).await?;
    server.write_all(&rewritten).await?;
    tokio::io::copy_bidirectional(&mut channel, &mut server).await?;
    Ok(())
}

async fn reject<S>(channel: &mut S, hello: &ClientHello)
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    let reply = hello.failure_reply("X11 forwarding: authentication failed");
    let _ = channel.write_all(&reply).await;
    let _ = channel.shutdown().await;
}

async fn open(endpoint: &ServerEndpoint) -> Result<Box<dyn AsyncReadWrite>, std::io::Error> {
    match endpoint {
        ServerEndpoint::Inet { host, port } => {
            let stream = tokio::net::TcpStream::connect((host.as_str(), *port)).await?;
            Ok(Box::new(stream))
        }
        ServerEndpoint::Unix(path) => unix_stream(path).await,
    }
}

#[cfg(unix)]
async fn unix_stream(path: &std::path::Path) -> Result<Box<dyn AsyncReadWrite>, std::io::Error> {
    Ok(Box::new(tokio::net::UnixStream::connect(path).await?))
}

#[cfg(not(unix))]
async fn unix_stream(_path: &std::path::Path) -> Result<Box<dyn AsyncReadWrite>, std::io::Error> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "unix socket endpoint on non-unix host",
    ))
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncReadExt, AsyncWriteExt, duplex};
    use tokio::net::TcpListener;

    use crate::setup::ByteOrder;

    use super::*;

    fn cookie(byte: u8) -> MagicCookie {
        MagicCookie::from_slice(&[byte; 16]).unwrap()
    }

    fn packet_with(cookie: &MagicCookie) -> Vec<u8> {
        ClientHello::forge(ByteOrder::Msb, "MIT-MAGIC-COOKIE-1", cookie.bytes())
            .encode_with(cookie.bytes())
    }

    #[tokio::test]
    async fn rewrites_cookie_then_relays_both_ways() {
        let fake = cookie(0xf1);
        let real = cookie(0x0e);

        // 伪装的本机 X server：校验收到的 setup 里已是真实 cookie，再回显数据。
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let expected_len = packet_with(&real).len();
        let server_task = tokio::spawn(async move {
            let (mut conn, _) = listener.accept().await.unwrap();
            let mut setup = vec![0u8; expected_len];
            conn.read_exact(&mut setup).await.unwrap();
            assert!(setup.windows(16).any(|w| w == [0x0e; 16]));
            assert!(!setup.windows(16).any(|w| w == [0xf1; 16]));

            let mut buf = [0u8; 4];
            conn.read_exact(&mut buf).await.unwrap();
            conn.write_all(&buf).await.unwrap();
        });

        let endpoint = ServerEndpoint::Inet {
            host: "127.0.0.1".into(),
            port,
        };
        let (mut client, channel) = duplex(8192);
        let expected_fake = fake.clone();
        let exchange =
            move |presented: &MagicCookie| (presented == &expected_fake).then(|| real.clone());

        let relay = relay_inner(channel, &endpoint, &exchange);
        let client_flow = async move {
            client.write_all(&packet_with(&fake)).await.unwrap();
            // setup 之后的字节应原样穿过桥接到达“X server”并被回显。
            client.write_all(b"ping").await.unwrap();
            let mut echoed = [0u8; 4];
            client.read_exact(&mut echoed).await.unwrap();
            assert_eq!(&echoed, b"ping");
        };
        let (relay_result, ()) = tokio::join!(relay, client_flow);

        server_task.await.unwrap();
        relay_result.unwrap();
    }

    #[tokio::test]
    async fn unknown_cookie_gets_failure_reply() {
        let endpoint = ServerEndpoint::Inet {
            host: "127.0.0.1".into(),
            port: 1, // 不应被连接
        };
        let (mut client, channel) = duplex(8192);

        let relay = relay_inner(channel, &endpoint, &|_: &MagicCookie| None);
        let client_flow = async move {
            client.write_all(&packet_with(&cookie(0x77))).await.unwrap();
            let mut reply = vec![0u8; 8];
            client.read_exact(&mut reply).await.unwrap();
            assert_eq!(reply[0], 0, "setup 失败应答的状态字节为 0");
        };
        let (relay_result, ()) = tokio::join!(relay, client_flow);

        assert!(matches!(
            relay_result,
            Err(BridgeError::Protocol(X11Error::CookieRejected))
        ));
    }
}
