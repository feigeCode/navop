//! Xauthority 文件（`~/.Xauthority`）读取与条目挑选。
//!
//! 文件是一串二进制记录（X 协议约定，全部大端）：
//! `u16 family`，随后四个 `u16 长度 + 字节内容` 的字段，依次为
//! address、display 编号（ASCII）、认证协议名、认证数据。

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use crate::cookie::COOKIE_LEN;
use crate::{DisplayAddress, ForwardRequest, MagicCookie, ServerEndpoint, X11Error, X11Result};

/// 记录 family 字段的取值（X 协议固定编号）。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecordFamily {
    Internet,
    Internet6,
    Local,
    Wild,
    Other(u16),
}

impl From<u16> for RecordFamily {
    fn from(code: u16) -> Self {
        match code {
            0 => Self::Internet,
            6 => Self::Internet6,
            256 => Self::Local,
            65535 => Self::Wild,
            other => Self::Other(other),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthorityRecord {
    pub family: RecordFamily,
    pub address: Vec<u8>,
    pub display: String,
    pub auth_name: String,
    pub auth_data: Vec<u8>,
}

/// 逐条读出文件中的记录；文件末尾的半截记录报截断错误。
pub fn records_of(bytes: &[u8]) -> impl Iterator<Item = X11Result<AuthorityRecord>> + '_ {
    let mut offset = 0usize;
    std::iter::from_fn(move || {
        if offset >= bytes.len() {
            return None;
        }
        let record = read_one(bytes, &mut offset);
        if record.is_err() {
            // 出错后终止迭代，避免调用方反复拿到同一个错误。
            offset = bytes.len();
        }
        Some(record)
    })
}

fn read_one(bytes: &[u8], offset: &mut usize) -> X11Result<AuthorityRecord> {
    let family = RecordFamily::from(be_u16(bytes, offset)?);
    let address = counted(bytes, offset)?.to_vec();
    let display = text(counted(bytes, offset)?);
    let auth_name = text(counted(bytes, offset)?);
    let auth_data = counted(bytes, offset)?.to_vec();
    Ok(AuthorityRecord {
        family,
        address,
        display,
        auth_name,
        auth_data,
    })
}

fn be_u16(bytes: &[u8], offset: &mut usize) -> X11Result<u16> {
    let pair: [u8; 2] = bytes
        .get(*offset..*offset + 2)
        .ok_or(X11Error::AuthorityTruncated)?
        .try_into()
        .map_err(|_| X11Error::AuthorityTruncated)?;
    *offset += 2;
    Ok(u16::from_be_bytes(pair))
}

fn counted<'a>(bytes: &'a [u8], offset: &mut usize) -> X11Result<&'a [u8]> {
    let len = be_u16(bytes, offset)? as usize;
    let field = bytes
        .get(*offset..*offset + len)
        .ok_or(X11Error::AuthorityTruncated)?;
    *offset += len;
    Ok(field)
}

fn text(raw: &[u8]) -> String {
    String::from_utf8_lossy(raw).into_owned()
}

/// 匹配时的本机线索：主机名与本机 IP，用于核对记录里的 address 字段。
#[derive(Clone, Debug, Default)]
pub struct HostHints {
    pub hostname: Option<String>,
    pub ips: Vec<IpAddr>,
}

/// 记录与目标 DISPLAY 的匹配程度，数值越大越精确。
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum Fit {
    /// FamilyWild 兜底。
    Wildcard,
    /// FamilyLocal 但无法核对主机名。
    LocalUnverified,
    /// 记录是回环地址，目标也是本机 DISPLAY。
    Loopback,
    /// FamilyLocal 且主机名核对一致。
    LocalNamed,
    /// 记录的 IP 与目标主机（或本机 IP）完全一致。
    AddressExact,
}

fn fit(record: &AuthorityRecord, target: &DisplayAddress, hints: &HostHints) -> Option<Fit> {
    if record.display != target.display_id() {
        return None;
    }
    match record.family {
        RecordFamily::Wild => Some(Fit::Wildcard),
        RecordFamily::Local => {
            if !target.serves_local_host() {
                return None;
            }
            let named = record.address.is_empty()
                || record.address == b"localhost"
                || hints
                    .hostname
                    .as_deref()
                    .is_some_and(|name| record.address == name.as_bytes());
            Some(if named {
                Fit::LocalNamed
            } else {
                Fit::LocalUnverified
            })
        }
        RecordFamily::Internet | RecordFamily::Internet6 => {
            let recorded = record_ip(record)?;
            let target_ip = match target.endpoint() {
                ServerEndpoint::Inet { host, .. } => host.parse::<IpAddr>().ok(),
                ServerEndpoint::Unix(_) => None,
            };
            if target_ip == Some(recorded) || hints.ips.contains(&recorded) {
                Some(Fit::AddressExact)
            } else if target.serves_local_host() && recorded.is_loopback() {
                Some(Fit::Loopback)
            } else {
                None
            }
        }
        RecordFamily::Other(_) => None,
    }
}

fn record_ip(record: &AuthorityRecord) -> Option<IpAddr> {
    match (record.family, record.address.len()) {
        (RecordFamily::Internet, 4) => Some(IpAddr::V4(Ipv4Addr::new(
            record.address[0],
            record.address[1],
            record.address[2],
            record.address[3],
        ))),
        (RecordFamily::Internet6, 16) => {
            let octets: [u8; 16] = record.address.as_slice().try_into().ok()?;
            Some(IpAddr::V6(Ipv6Addr::from(octets)))
        }
        _ => None,
    }
}

/// 从记录流中挑出与目标 DISPLAY 匹配度最高的 MIT-MAGIC-COOKIE-1 cookie。
pub fn best_cookie(
    records: impl IntoIterator<Item = X11Result<AuthorityRecord>>,
    target: &DisplayAddress,
    hints: &HostHints,
) -> X11Result<MagicCookie> {
    let mut best: Option<(Fit, MagicCookie)> = None;
    for record in records {
        let record = record?;
        if record.auth_name != ForwardRequest::AUTH_NAME || record.auth_data.len() != COOKIE_LEN {
            continue;
        }
        let Some(fit) = fit(&record, target, hints) else {
            continue;
        };
        let cookie = MagicCookie::from_slice(&record.auth_data)?;
        if best.as_ref().is_none_or(|(current, _)| fit > *current) {
            best = Some((fit, cookie));
        }
    }
    best.map(|(_, cookie)| cookie)
        .ok_or(X11Error::AuthorityNoMatch)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode_record(
        out: &mut Vec<u8>,
        family: u16,
        address: &[u8],
        display: &str,
        auth_name: &str,
        auth_data: &[u8],
    ) {
        out.extend_from_slice(&family.to_be_bytes());
        for field in [address, display.as_bytes(), auth_name.as_bytes(), auth_data] {
            out.extend_from_slice(&(field.len() as u16).to_be_bytes());
            out.extend_from_slice(field);
        }
    }

    fn cookie(byte: u8) -> [u8; 16] {
        [byte; 16]
    }

    #[test]
    fn iterates_records_until_end() {
        let mut raw = Vec::new();
        encode_record(
            &mut raw,
            256,
            b"myhost",
            "0",
            "MIT-MAGIC-COOKIE-1",
            &cookie(0x11),
        );
        encode_record(
            &mut raw,
            65535,
            b"",
            "1",
            "MIT-MAGIC-COOKIE-1",
            &cookie(0x22),
        );

        let parsed: Vec<_> = records_of(&raw).collect::<X11Result<_>>().unwrap();

        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].family, RecordFamily::Local);
        assert_eq!(parsed[1].family, RecordFamily::Wild);
        assert_eq!(parsed[1].display, "1");
    }

    #[test]
    fn truncated_tail_record_is_an_error() {
        let mut raw = Vec::new();
        encode_record(
            &mut raw,
            256,
            b"myhost",
            "0",
            "MIT-MAGIC-COOKIE-1",
            &cookie(0x11),
        );
        raw.extend_from_slice(&[0, 1, 0]);

        let outcomes: Vec<_> = records_of(&raw).collect();
        assert_eq!(outcomes.len(), 2);
        assert!(outcomes[1].is_err());
    }

    #[test]
    fn prefers_exact_ip_over_wildcard() {
        let mut raw = Vec::new();
        encode_record(
            &mut raw,
            65535,
            b"",
            "10",
            "MIT-MAGIC-COOKIE-1",
            &cookie(0xaa),
        );
        encode_record(
            &mut raw,
            0,
            &[10, 20, 30, 40],
            "10",
            "MIT-MAGIC-COOKIE-1",
            &cookie(0xbb),
        );
        let target = DisplayAddress::parse("10.20.30.40:10").unwrap();

        let chosen = best_cookie(records_of(&raw), &target, &HostHints::default()).unwrap();

        assert_eq!(chosen.bytes(), &cookie(0xbb));
    }

    #[test]
    fn local_family_matches_hostname_hint() {
        let mut raw = Vec::new();
        encode_record(
            &mut raw,
            256,
            b"studio-mac",
            "0",
            "MIT-MAGIC-COOKIE-1",
            &cookie(0xcc),
        );
        let target = DisplayAddress::parse(":0").unwrap();
        let hints = HostHints {
            hostname: Some("studio-mac".into()),
            ips: vec![],
        };

        let chosen = best_cookie(records_of(&raw), &target, &hints).unwrap();

        assert_eq!(chosen.bytes(), &cookie(0xcc));
    }

    #[test]
    fn foreign_auth_names_and_lengths_are_ignored() {
        let mut raw = Vec::new();
        encode_record(
            &mut raw,
            256,
            b"h",
            "0",
            "XDM-AUTHORIZATION-1",
            &cookie(0xdd),
        );
        encode_record(&mut raw, 256, b"h", "0", "MIT-MAGIC-COOKIE-1", &[0u8; 8]);
        let target = DisplayAddress::parse(":0").unwrap();

        assert!(matches!(
            best_cookie(records_of(&raw), &target, &HostHints::default()),
            Err(X11Error::AuthorityNoMatch)
        ));
    }

    #[test]
    fn ipv6_record_matches_via_hint() {
        let mut raw = Vec::new();
        encode_record(
            &mut raw,
            6,
            &Ipv6Addr::LOCALHOST.octets(),
            "4",
            "MIT-MAGIC-COOKIE-1",
            &cookie(0xee),
        );
        let target = DisplayAddress::parse("somehost:4").unwrap();
        let hints = HostHints {
            hostname: None,
            ips: vec![IpAddr::V6(Ipv6Addr::LOCALHOST)],
        };

        let chosen = best_cookie(records_of(&raw), &target, &hints).unwrap();

        assert_eq!(chosen.bytes(), &cookie(0xee));
    }
}
