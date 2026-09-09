//! RocketMQ Remoting 私有协议层:帧编解码、指令结构与请求/响应码。
//!
//! 协议契约基准:apache/rocketmq `remoting` 模块 `RemotingCommand.java` /
//! `RequestCode.java` / `ResponseCode.java`(以官方源码为准,兼容 4.x/5.x)。

pub mod codes;
pub mod command;
pub mod dto;

pub use codes::{RequestCode, ResponseCode};
pub use command::{
    MAX_FRAME_LENGTH, RemotingCommand, SerializeType, decode_frame, ensure_success,
    frame_body_length,
};
