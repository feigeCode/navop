//! DISPLAY 字符串解析，并在解析期直接求出本机 X server 的连接端点。
//!
//! 语法（X 协议约定）：`[协议/]主机:display[.screen]`
//! - 主机为空、`unix` 或以 `/unix` 结尾 → 本机 Unix 套接字 `/tmp/.X11-unix/X<n>`
//! - 主机部分以 `/` 开头 → 直接使用该套接字路径（macOS XQuartz 的 launchd 形式）
//! - 其余按 TCP 处理，端口为 `6000 + display`；`tcp/`、`inet/`、`inet6/` 为协议修饰

use std::path::PathBuf;

use crate::{X11Error, X11Result};

/// 本机 X server 连接端点（解析 DISPLAY 时一并求出）。
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ServerEndpoint {
    Unix(PathBuf),
    Inet { host: String, port: u16 },
}

impl ServerEndpoint {
    pub fn is_loopback(&self) -> bool {
        match self {
            ServerEndpoint::Unix(_) => true,
            ServerEndpoint::Inet { host, .. } => {
                matches!(host.as_str(), "localhost" | "127.0.0.1" | "::1")
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DisplayAddress {
    endpoint: ServerEndpoint,
    display: u16,
    screen: u16,
}

impl DisplayAddress {
    pub fn parse(text: &str) -> X11Result<Self> {
        let text = text.trim();
        if text.is_empty() {
            return Err(X11Error::DisplayMalformed("empty value".into()));
        }
        let colon = text
            .rfind(':')
            .ok_or_else(|| X11Error::DisplayMalformed("missing ':' separator".into()))?;
        let host_part = &text[..colon];
        let (display, screen) = split_numbers(&text[colon + 1..])?;
        let endpoint = endpoint_for(host_part, display)?;
        Ok(Self {
            endpoint,
            display,
            screen,
        })
    }

    pub fn endpoint(&self) -> &ServerEndpoint {
        &self.endpoint
    }

    pub fn screen(&self) -> u32 {
        self.screen as u32
    }

    /// Xauthority 记录里的 display 编号（字符串形式，不含 screen）。
    pub fn display_id(&self) -> String {
        self.display.to_string()
    }

    /// 是否指向本机（Unix 套接字或回环地址）。
    pub fn serves_local_host(&self) -> bool {
        self.endpoint.is_loopback()
    }
}

/// 解析 `display[.screen]` 数字段，screen 缺省为 0。
fn split_numbers(text: &str) -> X11Result<(u16, u16)> {
    let (display_text, screen_text) = match text.split_once('.') {
        Some((d, s)) => (d, s),
        None => (text, "0"),
    };
    Ok((number(display_text)?, number(screen_text)?))
}

fn number(text: &str) -> X11Result<u16> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return Err(X11Error::DisplayMalformed(format!(
            "'{text}' is not a non-negative integer"
        )));
    }
    text.parse::<u16>()
        .map_err(|_| X11Error::DisplayMalformed(format!("'{text}' out of range")))
}

fn endpoint_for(host_part: &str, display: u16) -> X11Result<ServerEndpoint> {
    // 本机 Unix 域：空主机、unix、任意以 /unix 结尾的形式。
    if host_part.is_empty() || host_part == "unix" || host_part.ends_with("/unix") {
        return Ok(ServerEndpoint::Unix(PathBuf::from(format!(
            "/tmp/.X11-unix/X{display}"
        ))));
    }
    // 绝对路径形式（macOS launchd 套接字，如 /private/tmp/.../org.xquartz）。
    if host_part.starts_with('/') {
        return Ok(ServerEndpoint::Unix(PathBuf::from(format!(
            "{host_part}:{display}"
        ))));
    }
    // 协议修饰：tcp/host、host/inet 等。
    let host = match host_part.split_once('/') {
        Some((left, right)) if is_inet_scheme(left) => right,
        Some((left, right)) if is_inet_scheme(right) => left,
        _ => host_part,
    };
    let host = strip_ipv6_brackets(host).to_string();
    if host.is_empty() {
        return Err(X11Error::DisplayMalformed("empty TCP host".into()));
    }
    let port = 6000u16
        .checked_add(display)
        .ok_or(X11Error::DisplayPortOverflow(display))?;
    Ok(ServerEndpoint::Inet { host, port })
}

fn is_inet_scheme(text: &str) -> bool {
    matches!(text, "tcp" | "inet" | "inet6")
}

fn strip_ipv6_brackets(host: &str) -> &str {
    host.strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_colon_display_uses_default_unix_dir() {
        let addr = DisplayAddress::parse(" :0 ").unwrap();
        assert_eq!(
            addr.endpoint,
            ServerEndpoint::Unix(PathBuf::from("/tmp/.X11-unix/X0"))
        );
        assert_eq!(addr.display_id(), "0");
        assert_eq!(addr.screen(), 0);
        assert!(addr.serves_local_host());
    }

    #[test]
    fn unix_variants_all_map_to_unix_dir() {
        for text in ["unix:2.1", "localhost/unix:3", ":11"] {
            let addr = DisplayAddress::parse(text).unwrap();
            assert!(matches!(addr.endpoint, ServerEndpoint::Unix(_)), "{text}");
        }
        assert_eq!(DisplayAddress::parse("unix:2.1").unwrap().screen(), 1);
    }

    #[test]
    fn launchd_socket_path_is_used_verbatim() {
        let addr =
            DisplayAddress::parse("/private/tmp/com.apple.launchd.xy12/org.xquartz:0").unwrap();
        assert_eq!(
            addr.endpoint,
            ServerEndpoint::Unix(PathBuf::from(
                "/private/tmp/com.apple.launchd.xy12/org.xquartz:0"
            ))
        );
    }

    #[test]
    fn tcp_display_maps_to_6000_plus_display() {
        let addr = DisplayAddress::parse("localhost:10.0").unwrap();
        assert_eq!(
            addr.endpoint,
            ServerEndpoint::Inet {
                host: "localhost".into(),
                port: 6010
            }
        );
        assert!(addr.serves_local_host());
    }

    #[test]
    fn inet_scheme_and_ipv6_brackets_are_normalized() {
        assert_eq!(
            DisplayAddress::parse("tcp/[::1]:4").unwrap().endpoint,
            ServerEndpoint::Inet {
                host: "::1".into(),
                port: 6004
            }
        );
        assert_eq!(
            DisplayAddress::parse("192.168.1.9/inet:1")
                .unwrap()
                .endpoint,
            ServerEndpoint::Inet {
                host: "192.168.1.9".into(),
                port: 6001
            }
        );
    }

    #[test]
    fn rejects_garbage() {
        assert!(DisplayAddress::parse("").is_err());
        assert!(DisplayAddress::parse("no-colon").is_err());
        assert!(DisplayAddress::parse(":1x").is_err());
        assert!(DisplayAddress::parse("tcp/:1").is_err());
        assert!(DisplayAddress::parse("host:65535").is_err());
    }
}
