//! Remoting 指令与帧编解码。
//!
//! 帧格式(对照官方 `RemotingCommand.encode()` / `NettyDecoder`):
//!
//! ```text
//! | 4B 总长度 | 4B 头部描述 | 头部 JSON | body 原始字节 |
//! ```
//!
//! - 总长度 = 4(头部描述字段自身) + 头部长度 + body 长度,不含总长度字段本身;
//! - 头部描述 = `(serialize_type << 24) | header_len`,本实现只支持 JSON(0);
//! - 头部为 fastjson 序列化的 `RemotingCommand` 字段,字段名与官方一致
//!   (code/language/version/opaque/flag/remark/extFields);
//! - body 为原始字节,语义由请求码决定(通常为 JSON DTO 或 commitlog 消息)。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::codes::response_code;
use crate::types::RocketmqError;

/// 单帧最大长度,与官方 `NettyDecoder.FRAME_MAX_LENGTH` 一致(16 MiB)
pub const MAX_FRAME_LENGTH: usize = 16 * 1024 * 1024;

/// 语言标识:发送固定 `OTHER`,与 4.x 服务端 `LanguageCode` 枚举兼容
pub const LANGUAGE_OTHER: &str = "OTHER";

/// 序列化类型:仅实现 JSON(官方默认),ROCKETMQ 二进制头暂不支持
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SerializeType {
    /// JSON 头(官方默认,编码 0)
    Json,
}

impl SerializeType {
    /// 编码值(位于头部描述的最高字节)
    pub fn code(self) -> u8 {
        match self {
            Self::Json => 0,
        }
    }

    /// 从编码值解析
    pub fn from_code(code: u8) -> Option<Self> {
        match code {
            0 => Some(Self::Json),
            _ => None,
        }
    }
}

/// Remoting 指令(请求与响应共用同一结构,靠 `flag` 的 RPC_TYPE 位区分)。
///
/// 字段名与官方 fastjson 序列化一致(camelCase,如 `extFields`)。
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemotingCommand {
    /// 请求/响应码
    pub code: i32,
    /// 客户端语言(发送端固定 OTHER)
    #[serde(default)]
    pub language: String,
    /// 协议版本号(发送 0 即可)
    #[serde(default)]
    pub version: i32,
    /// 请求标识,响应原样带回用于配对
    #[serde(default)]
    pub opaque: i32,
    /// 标志位:bit0=RPC_TYPE(0 请求/1 响应),bit1=RPC_ONEWAY
    #[serde(default)]
    pub flag: i32,
    /// 备注信息(错误描述)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remark: Option<String>,
    /// 扩展字段(即 Java 端 customHeader 展开后的字段)
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub ext_fields: HashMap<String, String>,
    /// body 原始字节(帧编解码专用,不参与头部 JSON)
    #[serde(skip)]
    pub body: Option<Vec<u8>>,
}

impl RemotingCommand {
    /// 构造请求指令
    pub fn create_request(code: i32, ext_fields: HashMap<String, String>) -> Self {
        Self {
            code,
            language: LANGUAGE_OTHER.to_string(),
            version: 0,
            opaque: 0,
            flag: 0,
            remark: None,
            ext_fields,
            body: None,
        }
    }

    /// 设置 body(链式)
    pub fn with_body(mut self, body: Vec<u8>) -> Self {
        self.body = Some(body);
        self
    }

    /// 是否为响应指令(flag bit0 = 1)
    pub fn is_response(&self) -> bool {
        (self.flag & 0x01) == 0x01
    }

    /// 按 JSON 序列化头部
    fn header_json(&self) -> Result<Vec<u8>, RocketmqError> {
        serde_json::to_vec(self)
            .map_err(|error| RocketmqError::Protocol(format!("头部 JSON 序列化失败: {error}")))
    }

    /// 编码为完整帧字节流
    pub fn encode_frame(&self) -> Result<Vec<u8>, RocketmqError> {
        let header = self.header_json()?;
        let body = self.body.as_deref().unwrap_or_default();
        // 总长度 = 4(头部描述) + 头部 + body,不含总长度字段本身(官方 encode 语义)
        let total = 4 + header.len() + body.len();
        if 4 + total > MAX_FRAME_LENGTH {
            return Err(RocketmqError::Protocol(format!(
                "帧长度超过上限 {MAX_FRAME_LENGTH}"
            )));
        }
        let mut frame = Vec::with_capacity(4 + total);
        frame.extend_from_slice(&(total as u32).to_be_bytes());
        // serialize_type(JSON=0) << 24 | header_len
        let header_desc = ((SerializeType::Json.code() as u32) << 24) | (header.len() as u32);
        frame.extend_from_slice(&header_desc.to_be_bytes());
        frame.extend_from_slice(&header);
        frame.extend_from_slice(body);
        Ok(frame)
    }
}

/// 从字节切片解析一帧(不含 4B 总长度前缀的"帧体"输入)。
///
/// 入参 `frame` 应为读取掉总长度字段后的完整剩余字节
/// (即 4B 头部描述 + 头部 + body),长度须与总长度一致。
pub fn decode_frame(frame: &[u8]) -> Result<RemotingCommand, RocketmqError> {
    if frame.len() < 4 {
        return Err(RocketmqError::Protocol(
            "帧长度不足: 缺少头部描述字段".into(),
        ));
    }
    let header_desc = u32::from_be_bytes([frame[0], frame[1], frame[2], frame[3]]);
    let serialize_code = ((header_desc >> 24) & 0xFF) as u8;
    let serialize_type = SerializeType::from_code(serialize_code).ok_or_else(|| {
        RocketmqError::Protocol(format!(
            "不支持的头部序列化类型: {serialize_code}(仅支持 JSON=0)"
        ))
    })?;
    debug_assert_eq!(serialize_type, SerializeType::Json);
    let header_len = (header_desc & 0x00FF_FFFF) as usize;
    let rest_len = frame.len() - 4;
    if header_len > rest_len {
        return Err(RocketmqError::Protocol(format!(
            "非法头部长度 {header_len}: 超出剩余帧体 {rest_len}"
        )));
    }
    let header_end = 4 + header_len;
    let mut command: RemotingCommand = serde_json::from_slice(&frame[4..header_end])
        .map_err(|error| RocketmqError::Protocol(format!("头部 JSON 解析失败: {error}")))?;
    if frame.len() > header_end {
        command.body = Some(frame[header_end..].to_vec());
    }
    Ok(command)
}

/// 从已读取的 4B 总长度解析应读取的帧体长度(总长度本身)。
///
/// 返回 Err 表示长度非法(0/超上限)。
pub fn frame_body_length(total_len: u32) -> Result<usize, RocketmqError> {
    if total_len < 4 {
        return Err(RocketmqError::Protocol(format!(
            "非法帧总长度: {total_len}"
        )));
    }
    let total = total_len as usize;
    if 4 + total > MAX_FRAME_LENGTH {
        return Err(RocketmqError::Protocol(format!(
            "帧长度超过上限 {MAX_FRAME_LENGTH}"
        )));
    }
    Ok(total)
}

/// 校验响应码:0 成功,NO_PERMISSION 归一化为认证错误,其余为协议错误。
pub fn ensure_success(command: &RemotingCommand) -> Result<(), RocketmqError> {
    if command.code == response_code::SUCCESS {
        return Ok(());
    }
    let remark = command.remark.clone().unwrap_or_default();
    if command.code == response_code::NO_PERMISSION {
        return Err(RocketmqError::Auth(format!(
            "RocketMQ 拒绝访问(code={}): {remark}",
            command.code
        )));
    }
    Err(RocketmqError::Protocol(format!(
        "RocketMQ 返回错误 code={}: {remark}",
        command.code
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::codes::{RequestCode, ResponseCode};

    fn sample_request() -> RemotingCommand {
        let mut ext = HashMap::new();
        ext.insert("topic".to_string(), "order-topic".to_string());
        ext.insert("queueId".to_string(), "0".to_string());
        RemotingCommand::create_request(RequestCode::GET_ROUTEINFO_BY_TOPIC, ext)
            .with_body(b"body-bytes".to_vec())
    }

    /// 请求 → 编码 → 解码往返一致,且帧头字节与官方格式吻合
    #[test]
    fn request_roundtrip_and_frame_layout() {
        let mut request = sample_request();
        request.opaque = 7;
        let frame = request.encode_frame().expect("编码失败");

        // 总长度 = 4 + header + body
        let total = u32::from_be_bytes([frame[0], frame[1], frame[2], frame[3]]) as usize;
        assert_eq!(total + 4, frame.len());

        // 头部描述:serialize_type=0 << 24 | header_len
        let desc = u32::from_be_bytes([frame[4], frame[5], frame[6], frame[7]]);
        assert_eq!(desc >> 24, 0, "JSON 序列化类型编码应为 0");
        let header_len = (desc & 0x00FF_FFFF) as usize;
        let header = &frame[8..8 + header_len];
        let body = &frame[8 + header_len..];
        assert_eq!(body, b"body-bytes");

        // 头部 JSON 字段名与官方 fastjson 一致
        let header_json: serde_json::Value = serde_json::from_slice(header).unwrap();
        assert_eq!(header_json["code"], RequestCode::GET_ROUTEINFO_BY_TOPIC);
        assert_eq!(header_json["language"], "OTHER");
        assert_eq!(header_json["opaque"], 7);
        assert_eq!(header_json["flag"], 0);
        assert_eq!(header_json["extFields"]["topic"], "order-topic");
        assert!(header_json.get("remark").is_none(), "空 remark 应省略");

        // 解码往返
        let decoded = decode_frame(&frame[4..]).expect("解码失败");
        assert_eq!(decoded.code, request.code);
        assert_eq!(decoded.opaque, request.opaque);
        assert_eq!(decoded.ext_fields, request.ext_fields);
        assert_eq!(decoded.body.as_deref(), Some(body));
        assert!(!decoded.is_response());
    }

    /// 响应头解析样本(flag=1 表示响应,remark/extFields 可选)
    #[test]
    fn response_header_parse_sample() {
        let header = br#"{"code":17,"language":"JAVA","version":412,"opaque":42,"flag":1,"remark":"topic[order-topic] not exist"}"#;
        let mut frame = Vec::new();
        let body_len = header.len();
        frame.extend_from_slice(&((4 + body_len) as u32).to_be_bytes());
        // serialize_type=JSON(0) 位于最高字节,此处显式补零以示帧格式
        frame.extend_from_slice(&(body_len as u32).to_be_bytes());
        frame.extend_from_slice(header);

        let total = frame_body_length(u32::from_be_bytes([frame[0], frame[1], frame[2], frame[3]]))
            .unwrap();
        let decoded = decode_frame(&frame[4..4 + total]).expect("解码失败");
        assert_eq!(decoded.code, ResponseCode::TOPIC_NOT_EXIST);
        assert_eq!(decoded.opaque, 42);
        assert_eq!(decoded.version, 412);
        assert!(decoded.is_response());
        assert!(decoded.remark.as_deref().unwrap().contains("not exist"));
        assert!(decoded.ext_fields.is_empty());
        assert!(decoded.body.is_none());

        // 非成功码应映射为协议错误
        let err = ensure_success(&decoded).unwrap_err();
        assert!(err.to_string().contains("code=17"));
    }

    /// 截断帧与非法长度拒绝
    #[test]
    fn rejects_truncated_and_invalid_frames() {
        // 少于 4 字节
        assert!(decode_frame(&[0, 0]).is_err());

        // 头部声明长度超出剩余帧体
        let mut frame = Vec::new();
        frame.extend_from_slice(&(1u32 << 24 | 100u32).to_be_bytes());
        frame.extend_from_slice(b"{}");
        assert!(decode_frame(&frame).is_err());

        // 非法 serialize_type(ROCKETMQ 二进制头)
        let mut frame = Vec::new();
        frame.extend_from_slice(&(2u32 << 24 | 2u32).to_be_bytes());
        frame.extend_from_slice(b"{}");
        let err = decode_frame(&frame).unwrap_err();
        assert!(err.to_string().contains("序列化类型"));

        // 总长度非法
        assert!(frame_body_length(0).is_err());
        assert!(frame_body_length(3).is_err());
        assert!(frame_body_length(u32::MAX).is_err());
        assert_eq!(frame_body_length(8).unwrap(), 8);
    }

    /// ACL 无 body 请求编码(签名场景兼容空 body)
    #[test]
    fn encode_without_body() {
        let mut ext = HashMap::new();
        ext.insert("AccessKey".to_string(), "rocketmq".to_string());
        let request = RemotingCommand::create_request(RequestCode::GET_BROKER_CLUSTER_INFO, ext);
        let frame = request.encode_frame().expect("编码失败");
        let total = u32::from_be_bytes([frame[0], frame[1], frame[2], frame[3]]) as usize;
        let decoded = decode_frame(&frame[4..4 + total]).unwrap();
        assert_eq!(decoded.body, None);
        assert_eq!(
            decoded.ext_fields.get("AccessKey").map(String::as_str),
            Some("rocketmq")
        );
    }
}
