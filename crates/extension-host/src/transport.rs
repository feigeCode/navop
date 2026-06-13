//! 帧传输抽象。
//!
//! 把「收一条帧 / 发一条帧」从具体载体(local socket / stdio / 内存管道)解耦,
//! 方便 `JsonRpcClient` 在不同环境用同一份代码:
//!
//! - 生产:`local socket`(进程间隔离 + 平台原生)
//! - 开发/测试:`stdio`(便于 strace / 抓包)
//! - 单测:in-memory pipe(无需 spawn 子进程)
//!
//! 帧格式来自 [`extension_protocol::framing`]:4-byte LE 长度前缀 + JSON 字节。
//! 后续切换 MessagePack 时,只需替换 send/recv 的序列化层。

use std::io;

use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::io::{AsyncRead, AsyncWrite};

/// 异步读半。允许同时 hold 写半在另一个 task 里。
///
/// 用 trait alias 风格,实际让具体类型直接 impl `tokio::io::AsyncRead` 即可。
pub trait ReadFramed: AsyncRead + Unpin + Send {}
impl<T> ReadFramed for T where T: AsyncRead + Unpin + Send {}

/// 异步写半。
pub trait WriteFramed: AsyncWrite + Unpin + Send {}
impl<T> WriteFramed for T where T: AsyncWrite + Unpin + Send {}

/// 帧传输——拆成 reader / writer 两半,方便上层 tokio::spawn 路由 reader,
/// 写入端被 Mutex 串行化即可。
pub struct FramedTransport<R, W>
where
    R: ReadFramed,
    W: WriteFramed,
{
    pub reader: R,
    pub writer: W,
}

impl<R, W> FramedTransport<R, W>
where
    R: ReadFramed,
    W: WriteFramed,
{
    pub fn new(reader: R, writer: W) -> Self {
        Self { reader, writer }
    }

    /// 拆成 (reader, writer)。
    pub fn split(self) -> (R, W) {
        (self.reader, self.writer)
    }
}

/// 发送一条 framed JSON 消息。
///
/// 直接复用 `extension_protocol::framing::send_msg_async`。
pub async fn send_async<W, T>(writer: &mut W, msg: &T) -> io::Result<()>
where
    W: WriteFramed,
    T: Serialize,
{
    extension_protocol::framing::send_msg_async(writer, msg).await
}

/// 接收一条 framed JSON 消息。
pub async fn recv_async<R, T>(reader: &mut R) -> io::Result<T>
where
    R: ReadFramed,
    T: DeserializeOwned,
{
    extension_protocol::framing::recv_msg_async(reader).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};
    use tokio::io::duplex;

    #[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
    struct Msg {
        id: u64,
        kind: String,
    }

    #[tokio::test]
    async fn send_recv_round_trips_through_duplex() {
        let (a, b) = duplex(4096);
        let (mut reader, mut writer) = tokio::io::split(b);
        let (mut other_reader, mut other_writer) = tokio::io::split(a);

        let send = tokio::spawn(async move {
            let m = Msg {
                id: 7,
                kind: "ping".into(),
            };
            send_async(&mut writer, &m).await.unwrap();
            // 再发一条
            let m2 = Msg {
                id: 8,
                kind: "pong".into(),
            };
            send_async(&mut writer, &m2).await.unwrap();
            drop(writer);
        });

        let m1: Msg = recv_async(&mut other_reader).await.unwrap();
        let m2: Msg = recv_async(&mut other_reader).await.unwrap();
        send.await.unwrap();

        // 抑制 unused warning
        let _ = (&mut reader, &mut other_writer);

        assert_eq!(
            m1,
            Msg {
                id: 7,
                kind: "ping".into()
            }
        );
        assert_eq!(
            m2,
            Msg {
                id: 8,
                kind: "pong".into()
            }
        );
    }

    #[tokio::test]
    async fn recv_returns_eof_when_writer_drops() {
        let (a, b) = duplex(64);
        let (mut reader, _w) = tokio::io::split(b);
        drop(a); // 模拟对端关闭

        let r: io::Result<Msg> = recv_async(&mut reader).await;
        assert!(r.is_err());
    }

    #[test]
    fn framed_transport_split_returns_parts() {
        let (a, _b) = std::sync::mpsc::channel::<u8>();
        // 编译期就行,不实际用 duplex,主要看 split 不消费 panic
        let (r, w) = tokio::io::duplex(16);
        let t = FramedTransport::new(r, w);
        let (_r, _w) = t.split();
        // pacify unused
        drop(a);
    }
}
