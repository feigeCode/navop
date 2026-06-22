use anyhow::{Result, bail};
use std::net::{Ipv4Addr, Ipv6Addr};

const SOCKS_VERSION_5: u8 = 0x05;
const METHOD_NO_AUTH: u8 = 0x00;
const COMMAND_CONNECT: u8 = 0x01;
const ATYP_IPV4: u8 = 0x01;
const ATYP_DOMAIN: u8 = 0x03;
const ATYP_IPV6: u8 = 0x04;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Socks5ConnectRequest {
    pub host: String,
    pub port: u16,
}

pub(crate) fn select_no_auth_method(greeting: &[u8]) -> Result<u8> {
    if greeting.len() < 2 || greeting[0] != SOCKS_VERSION_5 {
        bail!("invalid SOCKS5 greeting");
    }
    let method_count = greeting[1] as usize;
    if greeting.len() != method_count + 2 {
        bail!("invalid SOCKS5 method list length");
    }
    if greeting[2..].contains(&METHOD_NO_AUTH) {
        return Ok(METHOD_NO_AUTH);
    }
    bail!("SOCKS5 no-auth method is not offered");
}

pub(crate) fn parse_connect_request(bytes: &[u8]) -> Result<Socks5ConnectRequest> {
    if bytes.len() < 7 || bytes[0] != SOCKS_VERSION_5 {
        bail!("invalid SOCKS5 request");
    }
    if bytes[1] != COMMAND_CONNECT {
        bail!("only SOCKS5 CONNECT command is supported");
    }
    if bytes[2] != 0 {
        bail!("invalid SOCKS5 reserved byte");
    }
    match bytes[3] {
        ATYP_IPV4 => parse_ipv4_request(bytes),
        ATYP_DOMAIN => parse_domain_request(bytes),
        ATYP_IPV6 => parse_ipv6_request(bytes),
        _ => bail!("unsupported SOCKS5 address type"),
    }
}

pub(crate) fn socks5_method_selection(method: u8) -> [u8; 2] {
    [SOCKS_VERSION_5, method]
}

pub(crate) fn socks5_reply(reply: u8) -> [u8; 10] {
    [SOCKS_VERSION_5, reply, 0, ATYP_IPV4, 0, 0, 0, 0, 0, 0]
}

fn parse_ipv4_request(bytes: &[u8]) -> Result<Socks5ConnectRequest> {
    if bytes.len() != 10 {
        bail!("invalid SOCKS5 IPv4 request length");
    }
    let host = Ipv4Addr::new(bytes[4], bytes[5], bytes[6], bytes[7]).to_string();
    Ok(Socks5ConnectRequest {
        host,
        port: read_port(bytes, 8)?,
    })
}

fn parse_domain_request(bytes: &[u8]) -> Result<Socks5ConnectRequest> {
    let len = *bytes
        .get(4)
        .ok_or_else(|| anyhow::anyhow!("missing domain length"))? as usize;
    let port_offset = 5 + len;
    if bytes.len() != port_offset + 2 {
        bail!("invalid SOCKS5 domain request length");
    }
    let host = std::str::from_utf8(&bytes[5..port_offset])?.to_string();
    Ok(Socks5ConnectRequest {
        host,
        port: read_port(bytes, port_offset)?,
    })
}

fn parse_ipv6_request(bytes: &[u8]) -> Result<Socks5ConnectRequest> {
    if bytes.len() != 22 {
        bail!("invalid SOCKS5 IPv6 request length");
    }
    let mut octets = [0u8; 16];
    octets.copy_from_slice(&bytes[4..20]);
    Ok(Socks5ConnectRequest {
        host: Ipv6Addr::from(octets).to_string(),
        port: read_port(bytes, 20)?,
    })
}

fn read_port(bytes: &[u8], offset: usize) -> Result<u16> {
    let port = bytes
        .get(offset..offset + 2)
        .ok_or_else(|| anyhow::anyhow!("missing SOCKS5 port"))?;
    Ok(u16::from_be_bytes([port[0], port[1]]))
}

#[cfg(test)]
mod tests {
    use super::{parse_connect_request, select_no_auth_method};

    #[test]
    fn select_no_auth_accepts_no_auth_method() {
        assert_eq!(
            select_no_auth_method(&[0x05, 0x02, 0x02, 0x00]).unwrap(),
            0x00
        );
    }

    #[test]
    fn parse_connect_request_supports_domain_name() {
        let mut bytes = vec![0x05, 0x01, 0x00, 0x03, 11];
        bytes.extend_from_slice(b"example.com");
        bytes.extend_from_slice(&443u16.to_be_bytes());

        let request = parse_connect_request(&bytes).unwrap();

        assert_eq!(request.host, "example.com");
        assert_eq!(request.port, 443);
    }

    #[test]
    fn parse_connect_request_supports_ipv4() {
        let bytes = [0x05, 0x01, 0x00, 0x01, 127, 0, 0, 1, 0x1f, 0x90];

        let request = parse_connect_request(&bytes).unwrap();

        assert_eq!(request.host, "127.0.0.1");
        assert_eq!(request.port, 8080);
    }

    #[test]
    fn parse_connect_request_rejects_udp_associate() {
        let bytes = [0x05, 0x03, 0x00, 0x01, 127, 0, 0, 1, 0x1f, 0x90];

        let error = parse_connect_request(&bytes).unwrap_err();

        assert!(error.to_string().contains("CONNECT"));
    }
}
