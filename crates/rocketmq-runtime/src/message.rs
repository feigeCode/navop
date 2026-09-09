//! commitlog 消息二进制编解码与消息 ID 解析。
//!
//! `VIEW_MESSAGE_BY_ID` 响应 body 为 commitlog 存储格式的单条消息字节流,
//! 解析布局对照官方 `common/message/MessageDecoder.java`(`clientDecode` 路径):
//!
//! ```text
//! 1  TOTALSIZE              int32
//! 2  MAGICCODE              int32
//! 3  BODYCRC                int32
//! 4  QUEUEID                int32
//! 5  FLAG                   int32
//! 6  QUEUEOFFSET           int64
//! 7  PHYSICALOFFSET        int64
//! 8  SYSFLAG                int32
//! 9  BORNTIMESTAMP         int64
//! 10 BORNHOST               8B(IPv4+端口) 或 20B(IPv6+端口,依 SYSFLAG 位)
//! 11 STORETIMESTAMP        int64
//! 12 STOREHOST              8B / 20B
//! 13 RECONSUMETIMES         int32
//! 14 PREPARED_TXN_OFFSET   int64
//! 15 BODYLEN int32 + body 字节
//! 16 TOPICLEN int16 + topic 字节
//! 17 PROPERTIESLEN int16 + "k=v;k=v" 属性串
//! ```
//!
//! 消息 ID 对照 `MessageDecoder.createMessageId`:
//! 16 进制串 = 8B(4B IP + 4B 端口大端) + 8B commitlog 偏移。

use std::collections::HashMap;
use std::net::{Ipv4Addr, Ipv6Addr};

use crate::types::RocketmqError;

/// 系统标志位:存储主机为 IPv6
pub const STOREHOSTADDRESS_V6_FLAG: i32 = 1 << 4;
/// 系统标志位:生产主机为 IPv6
pub const BORNHOST_V6_FLAG: i32 = 1 << 5;

/// 属性串分隔符(官方 `MessageDecoder.NAME_VALUE_SEPARATOR` 等)
const PROPERTY_SEPARATOR: char = ';';
const PROPERTY_KV_SEPARATOR: char = '=';

/// 解析出的 commitlog 消息
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StoredMessage {
    /// 消息总字节数
    pub total_size: i32,
    /// 魔数(commitlog 数据魔数)
    pub magic_code: i32,
    /// body CRC
    pub body_crc: i32,
    /// 队列 ID
    pub queue_id: i32,
    /// 消息标志
    pub flag: i32,
    /// 队列偏移
    pub queue_offset: i64,
    /// commitlog 物理偏移
    pub physic_offset: i64,
    /// 系统标志
    pub sys_flag: i32,
    /// 生成时间(Unix 毫秒)
    pub born_timestamp: i64,
    /// 生成主机 `ip:port`
    pub born_host: String,
    /// 存储时间(Unix 毫秒)
    pub store_timestamp: i64,
    /// 存储 Broker `ip:port`
    pub store_host: String,
    /// 重试次数
    pub reconsume_times: i32,
    /// 事务半消息偏移
    pub prepared_transaction_offset: i64,
    /// 所属 Topic
    pub topic: String,
    /// 消息体
    pub body: Vec<u8>,
    /// 消息属性
    pub properties: HashMap<String, String>,
}

impl StoredMessage {
    /// 计算存储侧消息 ID(16 进制 32 位:storeHost 8B + physicOffset 8B)
    pub fn offset_msg_id(&self) -> String {
        build_msg_id(&self.store_host, self.physic_offset)
    }
}

/// 从字节流解析一条 commitlog 消息(等价官方 `MessageDecoder.clientDecode`)
pub fn decode_stored_message(bytes: &[u8]) -> Result<StoredMessage, RocketmqError> {
    let mut cursor = CursorReader::new(bytes);
    let total_size = cursor.read_i32()?;
    let magic_code = cursor.read_i32()?;
    let body_crc = cursor.read_i32()?;
    let queue_id = cursor.read_i32()?;
    let flag = cursor.read_i32()?;
    let queue_offset = cursor.read_i64()?;
    let physic_offset = cursor.read_i64()?;
    let sys_flag = cursor.read_i32()?;
    let born_timestamp = cursor.read_i64()?;
    let born_host = cursor.read_host((sys_flag & BORNHOST_V6_FLAG) != 0)?;
    let store_timestamp = cursor.read_i64()?;
    let store_host = cursor.read_host((sys_flag & STOREHOSTADDRESS_V6_FLAG) != 0)?;
    let reconsume_times = cursor.read_i32()?;
    let prepared_transaction_offset = cursor.read_i64()?;
    let body_len = cursor.read_i32()?;
    if body_len < 0 {
        return Err(RocketmqError::Protocol(format!(
            "非法 body 长度: {body_len}"
        )));
    }
    let body = cursor.read_bytes(body_len as usize)?.to_vec();
    let topic_len = cursor.read_i16()?;
    if topic_len <= 0 {
        return Err(RocketmqError::Protocol(format!(
            "非法 topic 长度: {topic_len}"
        )));
    }
    let topic = String::from_utf8(cursor.read_bytes(topic_len as usize)?.to_vec())
        .map_err(|error| RocketmqError::Protocol(format!("topic 非法 UTF-8: {error}")))?;
    let properties_len = cursor.read_i16()?;
    let properties = if properties_len > 0 {
        let text = String::from_utf8(cursor.read_bytes(properties_len as usize)?.to_vec())
            .map_err(|error| RocketmqError::Protocol(format!("属性串非法 UTF-8: {error}")))?;
        parse_properties(&text)
    } else {
        HashMap::new()
    };

    Ok(StoredMessage {
        total_size,
        magic_code,
        body_crc,
        queue_id,
        flag,
        queue_offset,
        physic_offset,
        sys_flag,
        born_timestamp,
        born_host,
        store_timestamp,
        store_host,
        reconsume_times,
        prepared_transaction_offset,
        topic,
        body,
        properties,
    })
}

/// 解析属性串 `"k1=v1;k2=v2"`(官方 `MessageDecoder.string2messageProperties`)
pub fn parse_properties(text: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for pair in text.split(PROPERTY_SEPARATOR) {
        if pair.is_empty() {
            continue;
        }
        match pair.split_once(PROPERTY_KV_SEPARATOR) {
            Some((key, value)) => map.insert(key.to_string(), value.to_string()),
            None => map.insert(pair.to_string(), String::new()),
        };
    }
    map
}

/// 序列化属性串(官方 `MessageDecoder.messageProperties2String`)
pub fn build_properties(map: &HashMap<String, String>) -> String {
    let mut pairs: Vec<String> = map
        .iter()
        .map(|(key, value)| format!("{key}{PROPERTY_KV_SEPARATOR}{value}"))
        .collect();
    pairs.sort();
    pairs.join(&PROPERTY_SEPARATOR.to_string())
}

/// 构造存储侧消息 ID:`hex(4B IP + 4B 端口 + 8B 偏移)`
pub fn build_msg_id(host_port: &str, physic_offset: i64) -> String {
    let mut buf = Vec::with_capacity(16);
    encode_host_bytes(host_port, &mut buf);
    buf.extend_from_slice(&(physic_offset as u64).to_be_bytes());
    buf.iter().map(|byte| format!("{byte:02X}")).collect()
}

/// 解析存储侧消息 ID → (host:port, 物理偏移)
pub fn parse_msg_id(msg_id: &str) -> Result<(String, i64), RocketmqError> {
    let trimmed = msg_id.trim();
    let bytes = hex_to_bytes(trimmed)?;
    if bytes.len() != 16 {
        return Err(RocketmqError::Protocol(format!(
            "非法消息 ID 长度: 期望 32 个十六进制字符, 实际 {}",
            trimmed.len()
        )));
    }
    let host = decode_host_bytes(&bytes[0..8])?;
    let offset = i64::from_be_bytes([
        bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15],
    ]);
    Ok((host, offset))
}

/// 十六进制串(不区分大小写)转字节
pub fn hex_to_bytes(text: &str) -> Result<Vec<u8>, RocketmqError> {
    if text.len() % 2 != 0 {
        return Err(RocketmqError::Protocol("十六进制串长度应为偶数".into()));
    }
    let mut bytes = Vec::with_capacity(text.len() / 2);
    let chars: Vec<char> = text.chars().collect();
    for index in (0..chars.len()).step_by(2) {
        let high = chars[index].to_digit(16).ok_or_else(|| {
            RocketmqError::Protocol(format!("非法十六进制字符: {}", chars[index]))
        })?;
        let low = chars[index + 1].to_digit(16).ok_or_else(|| {
            RocketmqError::Protocol(format!("非法十六进制字符: {}", chars[index + 1]))
        })?;
        bytes.push(((high << 4) | low) as u8);
    }
    Ok(bytes)
}

/// 编码 host:port → 8B(IPv4)字节;无法解析时按 IPv4 0.0.0.0 兜底
fn encode_host_bytes(host_port: &str, out: &mut Vec<u8>) {
    let (host, port) = split_host_port(host_port);
    if let Ok(ip) = host.parse::<Ipv4Addr>() {
        out.extend_from_slice(&ip.octets());
    } else if let Ok(ip) = host.parse::<Ipv6Addr>() {
        out.extend_from_slice(&ip.octets());
    } else {
        out.extend_from_slice(&[0, 0, 0, 0]);
    }
    out.extend_from_slice(&(port as u32).to_be_bytes());
}

/// 解码 8B(IPv4+端口)为 `ip:port`
fn decode_host_bytes(bytes: &[u8]) -> Result<String, RocketmqError> {
    if bytes.len() != 8 {
        return Err(RocketmqError::Protocol("主机字节长度应为 8".into()));
    }
    let ip = Ipv4Addr::new(bytes[0], bytes[1], bytes[2], bytes[3]);
    let port = u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    Ok(format!("{ip}:{port}"))
}

/// 拆分 `host:port`(端口非法时按 0 处理)
fn split_host_port(host_port: &str) -> (String, u16) {
    match host_port.rsplit_once(':') {
        Some((host, port)) => match port.parse::<u16>() {
            Ok(port) => (host.to_string(), port),
            Err(_) => (host_port.to_string(), 0),
        },
        None => (host_port.to_string(), 0),
    }
}

/// 顺序字节读取器
struct CursorReader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> CursorReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.pos)
    }

    fn read_bytes(&mut self, len: usize) -> Result<&'a [u8], RocketmqError> {
        if self.remaining() < len {
            return Err(RocketmqError::Protocol(format!(
                "commitlog 消息字节不足: 需要 {len}, 剩余 {}",
                self.remaining()
            )));
        }
        let slice = &self.bytes[self.pos..self.pos + len];
        self.pos += len;
        Ok(slice)
    }

    fn read_i16(&mut self) -> Result<i16, RocketmqError> {
        let bytes = self.read_bytes(2)?;
        Ok(i16::from_be_bytes([bytes[0], bytes[1]]))
    }

    fn read_i32(&mut self) -> Result<i32, RocketmqError> {
        let bytes = self.read_bytes(4)?;
        Ok(i32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn read_i64(&mut self) -> Result<i64, RocketmqError> {
        let bytes = self.read_bytes(8)?;
        Ok(i64::from_be_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]))
    }

    /// 读取主机地址:IPv4 为 8B,IPv6 为 20B
    fn read_host(&mut self, ipv6: bool) -> Result<String, RocketmqError> {
        if ipv6 {
            let bytes = self.read_bytes(20)?;
            let ip = Ipv6Addr::new(
                u16::from_be_bytes([bytes[0], bytes[1]]),
                u16::from_be_bytes([bytes[2], bytes[3]]),
                u16::from_be_bytes([bytes[4], bytes[5]]),
                u16::from_be_bytes([bytes[6], bytes[7]]),
                u16::from_be_bytes([bytes[8], bytes[9]]),
                u16::from_be_bytes([bytes[10], bytes[11]]),
                u16::from_be_bytes([bytes[12], bytes[13]]),
                u16::from_be_bytes([bytes[14], bytes[15]]),
            );
            let port = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
            Ok(format!("[{ip}]:{port}"))
        } else {
            let bytes = self.read_bytes(8)?;
            decode_host_bytes(bytes)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 参数化构造 commitlog 消息字节流(主机地址原始字节可注入)
    fn build_message_bytes(
        sys_flag: i32,
        born_host_bytes: Vec<u8>,
        store_host_bytes: Vec<u8>,
    ) -> Vec<u8> {
        let topic = b"order-topic";
        let body = br#"{"orderId":1}"#;
        let properties = "KEYS=order-1;TAGS=create;UNIQ_KEY=ABCDEF0123456789ABCDEF01";

        let mut buf = Vec::new();
        buf.extend_from_slice(&0i32.to_be_bytes()); // total_size 占位
        buf.extend_from_slice(&(0xAABBCCDDu32 as i32).to_be_bytes()); // magic
        buf.extend_from_slice(&123456789i32.to_be_bytes()); // body crc
        buf.extend_from_slice(&3i32.to_be_bytes()); // queue id
        buf.extend_from_slice(&0i32.to_be_bytes()); // flag
        buf.extend_from_slice(&7i64.to_be_bytes()); // queue offset
        buf.extend_from_slice(&2912i64.to_be_bytes()); // physic offset
        buf.extend_from_slice(&sys_flag.to_be_bytes());
        buf.extend_from_slice(&1735710000000i64.to_be_bytes()); // born ts
        buf.extend_from_slice(&born_host_bytes);
        buf.extend_from_slice(&1735710000010i64.to_be_bytes()); // store ts
        buf.extend_from_slice(&store_host_bytes);
        buf.extend_from_slice(&2i32.to_be_bytes()); // reconsume times
        buf.extend_from_slice(&0i64.to_be_bytes()); // prepared txn offset
        buf.extend_from_slice(&(body.len() as i32).to_be_bytes());
        buf.extend_from_slice(body);
        buf.extend_from_slice(&(topic.len() as i16).to_be_bytes());
        buf.extend_from_slice(topic);
        buf.extend_from_slice(&(properties.len() as i16).to_be_bytes());
        buf.extend_from_slice(properties.as_bytes());
        let total = buf.len() as i32;
        buf[0..4].copy_from_slice(&total.to_be_bytes());
        buf
    }

    /// IPv4 主机地址原始字节(4B IP + 4B 端口)
    fn ipv4_host(ip: [u8; 4], port: u16) -> Vec<u8> {
        let mut bytes = ip.to_vec();
        bytes.extend_from_slice(&(port as u32).to_be_bytes());
        bytes
    }

    /// IPv6 主机地址原始字节(16B IP + 4B 端口)
    fn ipv6_host(octets: [u8; 16], port: u16) -> Vec<u8> {
        let mut bytes = octets.to_vec();
        bytes.extend_from_slice(&(port as u32).to_be_bytes());
        bytes
    }

    /// 往返:构造 → 解析 → 字段断言
    #[test]
    fn decode_stored_message_sample() {
        let bytes = build_message_bytes(
            0,
            ipv4_host([10, 0, 0, 9], 52300),
            ipv4_host([10, 0, 0, 2], 10911),
        );
        let message = decode_stored_message(&bytes).expect("解析失败");
        assert_eq!(message.total_size as usize, bytes.len());
        assert_eq!(message.magic_code, 0xAABBCCDDu32 as i32);
        assert_eq!(message.queue_id, 3);
        assert_eq!(message.queue_offset, 7);
        assert_eq!(message.physic_offset, 2912);
        assert_eq!(message.born_host, "10.0.0.9:52300");
        assert_eq!(message.store_host, "10.0.0.2:10911");
        assert_eq!(message.reconsume_times, 2);
        assert_eq!(message.topic, "order-topic");
        assert_eq!(message.body, br#"{"orderId":1}"#);
        assert_eq!(message.properties["KEYS"], "order-1");
        assert_eq!(message.properties["TAGS"], "create");
        assert_eq!(message.born_timestamp, 1735710000000);
        assert_eq!(message.store_timestamp, 1735710000010);
    }

    /// 截断流拒绝
    #[test]
    fn rejects_truncated_message() {
        let bytes = build_message_bytes(
            0,
            ipv4_host([10, 0, 0, 9], 52300),
            ipv4_host([10, 0, 0, 2], 10911),
        );
        let err = decode_stored_message(&bytes[..bytes.len() - 10]).unwrap_err();
        assert!(err.to_string().contains("字节不足"), "实际: {err}");
        assert!(decode_stored_message(&bytes[..12]).is_err());
    }

    /// 消息 ID 构造/解析往返(对照官方 createMessageId 格式)
    #[test]
    fn msg_id_roundtrip() {
        let msg_id = build_msg_id("10.0.0.2:10911", 2912);
        // 10.0.0.2 = 0A000002, 10911 = 0x2A9F, offset=0x0000000000000B60
        assert_eq!(msg_id, "0A00000200002A9F0000000000000B60");
        let (host, offset) = parse_msg_id(&msg_id).unwrap();
        assert_eq!(host, "10.0.0.2:10911");
        assert_eq!(offset, 2912);

        // 小写输入也可解析
        let (host, offset) = parse_msg_id(&msg_id.to_lowercase()).unwrap();
        assert_eq!(host, "10.0.0.2:10911");
        assert_eq!(offset, 2912);

        // 非法输入
        assert!(parse_msg_id("0A00").is_err());
        assert!(parse_msg_id("XY").is_err());
        assert!(parse_msg_id(&"0".repeat(31)).is_err());
    }

    /// 属性串解析/序列化
    #[test]
    fn properties_string_roundtrip() {
        let map = parse_properties("KEYS=order-1;TAGS=create;UNIQ_KEY=ABC");
        assert_eq!(map.len(), 3);
        assert_eq!(map["KEYS"], "order-1");
        let text = build_properties(&map);
        // 键排序后拼接
        assert_eq!(text, "KEYS=order-1;TAGS=create;UNIQ_KEY=ABC");
        assert!(parse_properties("").is_empty());
        assert!(parse_properties(";;;").is_empty());
        // 无等号的孤立键保留为空值
        let map = parse_properties("TRACE");
        assert_eq!(map["TRACE"], "");
    }

    /// IPv6 主机地址(sysflag 置位)解析
    #[test]
    fn decode_ipv6_host_message() {
        let born_octets = {
            let mut octets = [0u8; 16];
            octets[0..2].copy_from_slice(&0x2001u16.to_be_bytes());
            octets
        };
        let store_octets = {
            let mut octets = [0u8; 16];
            octets[15] = 1; // ::1
            octets
        };
        let bytes = build_message_bytes(
            BORNHOST_V6_FLAG | STOREHOSTADDRESS_V6_FLAG,
            ipv6_host(born_octets, 52300),
            ipv6_host(store_octets, 10911),
        );
        let message = decode_stored_message(&bytes).expect("IPv6 消息解析失败");
        assert_eq!(message.born_host, "[2001::]:52300");
        assert_eq!(message.store_host, "[::1]:10911");
        assert_eq!(message.born_timestamp, 1735710000000);
    }
}
