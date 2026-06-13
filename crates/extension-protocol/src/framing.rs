//! 当前扩展协议使用的长度前缀 JSON 消息帧。
//!
//! 每条消息 = 4 字节 LE 长度前缀 + JSON 序列化的消息体。
//! 最大消息大小为 16 MiB。

use serde::Serialize;
use serde::de::DeserializeOwned;
use std::io::{self, Read, Write};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const MAX_MSG_SIZE: u32 = 16 * 1024 * 1024;

/// 发送一条长度前缀的 JSON 消息（同步）。
pub fn send_msg<W: Write, T: Serialize>(mut writer: W, msg: &T) -> io::Result<()> {
    let bytes = serde_json::to_vec(msg).map_err(io::Error::other)?;
    let len = bytes.len() as u32;
    write_framed(&mut writer, &bytes, len)
}

/// 接收一条长度前缀的 JSON 消息（同步）。
pub fn recv_msg<R: Read, T: DeserializeOwned>(mut reader: R) -> io::Result<T> {
    let buf = read_framed(&mut reader)?;
    serde_json::from_slice(&buf).map_err(io::Error::other)
}

/// 发送一条长度前缀的 JSON 消息（异步）。
pub async fn send_msg_async<W: AsyncWriteExt + Unpin, T: Serialize>(
    mut writer: W,
    msg: &T,
) -> io::Result<()> {
    let bytes = serde_json::to_vec(msg).map_err(io::Error::other)?;
    let len = bytes.len() as u32;
    write_framed_async(&mut writer, &bytes, len).await
}

/// 接收一条长度前缀的 JSON 消息（异步）。
pub async fn recv_msg_async<R: AsyncReadExt + Unpin, T: DeserializeOwned>(
    mut reader: R,
) -> io::Result<T> {
    let buf = read_framed_async(&mut reader).await?;
    serde_json::from_slice(&buf).map_err(io::Error::other)
}

fn write_framed(writer: &mut impl Write, bytes: &[u8], len: u32) -> io::Result<()> {
    if len > MAX_MSG_SIZE {
        return Err(io::Error::other("message exceeds max size"));
    }
    writer.write_all(&len.to_le_bytes())?;
    writer.write_all(bytes)?;
    writer.flush()
}

async fn write_framed_async(
    writer: &mut (impl AsyncWriteExt + Unpin),
    bytes: &[u8],
    len: u32,
) -> io::Result<()> {
    if len > MAX_MSG_SIZE {
        return Err(io::Error::other("message exceeds max size"));
    }
    writer.write_all(&len.to_le_bytes()).await?;
    writer.write_all(bytes).await?;
    writer.flush().await
}

fn read_framed(reader: &mut impl Read) -> io::Result<Vec<u8>> {
    let mut len_bytes = [0u8; 4];
    reader.read_exact(&mut len_bytes)?;
    let len = u32::from_le_bytes(len_bytes) as usize;
    if len > MAX_MSG_SIZE as usize {
        return Err(io::Error::other("message exceeds max size"));
    }
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf)?;
    Ok(buf)
}

async fn read_framed_async(reader: &mut (impl AsyncReadExt + Unpin)) -> io::Result<Vec<u8>> {
    let mut len_bytes = [0u8; 4];
    reader.read_exact(&mut len_bytes).await?;
    let len = u32::from_le_bytes(len_bytes) as usize;
    if len > MAX_MSG_SIZE as usize {
        return Err(io::Error::other("message exceeds max size"));
    }
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf).await?;
    Ok(buf)
}
