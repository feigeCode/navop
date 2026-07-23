//! X11 连接建立（setup）报文的编解码。
//!
//! 客户端首个报文格式（X 协议规定，整数按首字节声明的字节序）：
//! 12 字节固定头（字节序标记、主次版本号、认证协议名长度 n、认证数据
//! 长度 d），随后是 4 字节对齐的协议名与认证数据。
//!
//! 转发时先完整解码成 [`ClientHello`]，校验 cookie 后再用本机真实
//! cookie 重新编码，不做原位修改。

use crate::{ForwardRequest, MagicCookie, X11Error, X11Result};

/// 固定头长度（字节序标记 + unused + 4 个 u16）。
pub const HEADER_LEN: usize = 12;

/// 认证区（协议名 + 数据，含对齐填充）的体积上限，防止恶意长度字段。
pub const MAX_AUTH_SECTION: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ByteOrder {
    Msb,
    Lsb,
}

impl ByteOrder {
    fn from_mark(mark: u8) -> X11Result<Self> {
        match mark {
            b'B' => Ok(Self::Msb),
            b'l' => Ok(Self::Lsb),
            other => Err(X11Error::BadByteOrderMark(other)),
        }
    }

    fn u16_of(self, pair: &[u8]) -> u16 {
        match self {
            Self::Msb => u16::from_be_bytes([pair[0], pair[1]]),
            Self::Lsb => u16::from_le_bytes([pair[0], pair[1]]),
        }
    }

    fn push_u16(self, out: &mut Vec<u8>, value: u16) {
        out.extend_from_slice(&match self {
            Self::Msb => value.to_be_bytes(),
            Self::Lsb => value.to_le_bytes(),
        });
    }
}

/// 4 字节对齐（X 协议填充规则）。
fn align4(len: usize) -> usize {
    (len + 3) & !3
}

/// 解码后的客户端 setup 报文。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClientHello {
    order: ByteOrder,
    pub major: u16,
    pub minor: u16,
    pub auth_name: String,
    pub auth_data: Vec<u8>,
}

impl ClientHello {
    #[cfg(test)]
    pub(crate) fn forge(order: ByteOrder, auth_name: &str, auth_data: &[u8]) -> Self {
        Self {
            order,
            major: 11,
            minor: 0,
            auth_name: auth_name.to_string(),
            auth_data: auth_data.to_vec(),
        }
    }

    /// 从 12 字节固定头计算认证区（含填充）长度。
    pub fn auth_section_len(header: &[u8; HEADER_LEN]) -> X11Result<usize> {
        let order = ByteOrder::from_mark(header[0])?;
        let name_len = order.u16_of(&header[6..8]) as usize;
        let data_len = order.u16_of(&header[8..10]) as usize;
        let total = align4(name_len) + align4(data_len);
        if total > MAX_AUTH_SECTION {
            return Err(X11Error::SetupOversized(MAX_AUTH_SECTION));
        }
        Ok(total)
    }

    /// 解码完整报文（头 + 认证区），长度必须精确吻合。
    pub fn decode(packet: &[u8]) -> X11Result<Self> {
        if packet.len() < HEADER_LEN {
            return Err(X11Error::SetupTruncated);
        }
        let order = ByteOrder::from_mark(packet[0])?;
        let name_len = order.u16_of(&packet[6..8]) as usize;
        let data_len = order.u16_of(&packet[8..10]) as usize;
        if packet.len() != HEADER_LEN + align4(name_len) + align4(data_len) {
            return Err(X11Error::SetupTruncated);
        }

        let mut cursor = Cursor::at(packet, HEADER_LEN);
        let auth_name = String::from_utf8_lossy(cursor.take(name_len)?).into_owned();
        cursor.skip(align4(name_len) - name_len)?;
        let auth_data = cursor.take(data_len)?.to_vec();

        Ok(Self {
            order,
            major: order.u16_of(&packet[2..4]),
            minor: order.u16_of(&packet[4..6]),
            auth_name,
            auth_data,
        })
    }

    /// 取出客户端出示的 MIT-MAGIC-COOKIE-1。
    pub fn presented_cookie(&self) -> X11Result<MagicCookie> {
        if self.auth_name != ForwardRequest::AUTH_NAME {
            return Err(X11Error::UnknownAuthName(self.auth_name.clone()));
        }
        MagicCookie::from_slice(&self.auth_data)
    }

    /// 用替换后的认证数据重新编码整个报文。
    pub fn encode_with(&self, auth_data: &[u8]) -> Vec<u8> {
        let mut out =
            Vec::with_capacity(HEADER_LEN + align4(self.auth_name.len()) + align4(auth_data.len()));
        out.push(match self.order {
            ByteOrder::Msb => b'B',
            ByteOrder::Lsb => b'l',
        });
        out.push(0);
        self.order.push_u16(&mut out, self.major);
        self.order.push_u16(&mut out, self.minor);
        self.order.push_u16(&mut out, self.auth_name.len() as u16);
        self.order.push_u16(&mut out, auth_data.len() as u16);
        self.order.push_u16(&mut out, 0);
        out.extend_from_slice(self.auth_name.as_bytes());
        pad_zeros(&mut out, self.auth_name.len());
        out.extend_from_slice(auth_data);
        pad_zeros(&mut out, auth_data.len());
        out
    }

    /// 协议级 setup 失败应答（状态字节 0），用于拒绝未通过校验的客户端。
    pub fn failure_reply(&self, reason: &str) -> Vec<u8> {
        let reason = reason.as_bytes();
        let capped = &reason[..reason.len().min(u8::MAX as usize)];
        let mut out = Vec::with_capacity(8 + align4(capped.len()));
        out.push(0);
        out.push(capped.len() as u8);
        self.order.push_u16(&mut out, self.major);
        self.order.push_u16(&mut out, self.minor);
        self.order
            .push_u16(&mut out, (align4(capped.len()) / 4) as u16);
        out.extend_from_slice(capped);
        pad_zeros(&mut out, capped.len());
        out
    }
}

/// 按字段原始长度补齐 4 字节对齐的零填充。
fn pad_zeros(out: &mut Vec<u8>, field_len: usize) {
    out.resize(out.len() + (align4(field_len) - field_len), 0);
}

/// 顺序读取游标（带边界检查）。
struct Cursor<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn at(buf: &'a [u8], pos: usize) -> Self {
        Self { buf, pos }
    }

    fn take(&mut self, len: usize) -> X11Result<&'a [u8]> {
        let slice = self
            .buf
            .get(self.pos..self.pos + len)
            .ok_or(X11Error::SetupTruncated)?;
        self.pos += len;
        Ok(slice)
    }

    fn skip(&mut self, len: usize) -> X11Result<()> {
        self.take(len).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hello(order: ByteOrder, cookie: &[u8; 16]) -> ClientHello {
        ClientHello {
            order,
            major: 11,
            minor: 0,
            auth_name: ForwardRequest::AUTH_NAME.to_string(),
            auth_data: cookie.to_vec(),
        }
    }

    #[test]
    fn encode_decode_round_trip_both_orders() {
        for order in [ByteOrder::Msb, ByteOrder::Lsb] {
            let original = hello(order, &[0x42; 16]);
            let packet = original.encode_with(&original.auth_data.clone());

            let header: &[u8; HEADER_LEN] = packet[..HEADER_LEN].try_into().unwrap();
            assert_eq!(
                ClientHello::auth_section_len(header).unwrap(),
                packet.len() - HEADER_LEN
            );

            let decoded = ClientHello::decode(&packet).unwrap();
            assert_eq!(decoded, original);
        }
    }

    #[test]
    fn reencode_with_different_cookie_changes_only_auth_data() {
        let original = hello(ByteOrder::Lsb, &[0x01; 16]);
        let packet = original.encode_with(&[0x99; 16]);

        let decoded = ClientHello::decode(&packet).unwrap();
        assert_eq!(decoded.auth_data, vec![0x99; 16]);
        assert_eq!(decoded.major, 11);
        assert_eq!(decoded.auth_name, ForwardRequest::AUTH_NAME);
    }

    #[test]
    fn presented_cookie_enforces_protocol_name() {
        let mut bad = hello(ByteOrder::Msb, &[0x42; 16]);
        bad.auth_name = "XC-QUERY-SECURITY-1".into();
        assert!(matches!(
            bad.presented_cookie(),
            Err(X11Error::UnknownAuthName(_))
        ));

        let good = hello(ByteOrder::Msb, &[0x42; 16]);
        assert_eq!(good.presented_cookie().unwrap().bytes(), &[0x42; 16]);
    }

    #[test]
    fn decode_rejects_length_mismatch_and_bad_mark() {
        let good = hello(ByteOrder::Lsb, &[0x42; 16]);
        let mut packet = good.encode_with(&good.auth_data.clone());
        packet.push(0);
        assert!(matches!(
            ClientHello::decode(&packet),
            Err(X11Error::SetupTruncated)
        ));

        let mut broken = hello(ByteOrder::Lsb, &[0x42; 16]).encode_with(&[0x42; 16]);
        broken[0] = b'x';
        assert!(matches!(
            ClientHello::decode(&broken),
            Err(X11Error::BadByteOrderMark(b'x'))
        ));
    }

    #[test]
    fn failure_reply_starts_with_denied_status() {
        let original = hello(ByteOrder::Msb, &[0x42; 16]);
        let reply = original.failure_reply("denied");

        assert_eq!(reply[0], 0);
        assert_eq!(reply[1], 6);
        assert_eq!(&reply[8..14], b"denied");
        assert_eq!(reply.len() % 4, 0);
    }

    #[test]
    fn oversized_auth_section_is_capped() {
        let mut header = [0u8; HEADER_LEN];
        header[0] = b'B';
        header[6..8].copy_from_slice(&u16::MAX.to_be_bytes());
        header[8..10].copy_from_slice(&u16::MAX.to_be_bytes());
        assert!(matches!(
            ClientHello::auth_section_len(&header),
            Err(X11Error::SetupOversized(_))
        ));
    }
}
