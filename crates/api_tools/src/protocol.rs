//! 接口测试支持的传输协议及其基础行为。

/// 请求使用的协议。
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    #[default]
    Http,
    Graphql,
    Sse,
    WebSocket,
    Tcp,
    #[serde(rename = "grpc_web")]
    GrpcWeb,
    SocketIo,
}

impl Protocol {
    pub const ALL: &'static [Self] = &[
        Self::Http,
        Self::Graphql,
        Self::Sse,
        Self::WebSocket,
        Self::Tcp,
        Self::GrpcWeb,
        Self::SocketIo,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Http => "HTTP",
            Self::Graphql => "GraphQL",
            Self::Sse => "SSE",
            Self::WebSocket => "WebSocket",
            Self::Tcp => "TCP",
            Self::GrpcWeb => "gRPC-Web",
            Self::SocketIo => "Socket.IO",
        }
    }

    pub fn uses_http_method(self) -> bool {
        matches!(self, Self::Http | Self::Graphql)
    }

    pub fn default_scheme(self) -> &'static str {
        match self {
            Self::WebSocket => "ws",
            Self::Tcp => "tcp",
            Self::SocketIo => "socketio",
            Self::Http | Self::Graphql | Self::Sse | Self::GrpcWeb => "http",
        }
    }

    pub fn badge_label(self) -> Option<&'static str> {
        match self {
            Self::Http | Self::Graphql => None,
            Self::Sse => Some("SSE"),
            Self::WebSocket => Some("WS"),
            Self::Tcp => Some("TCP"),
            Self::GrpcWeb => Some("gRPC-Web"),
            Self::SocketIo => Some("Socket.IO"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_names_are_stable() {
        assert_eq!(
            serde_json::to_string(&Protocol::WebSocket).unwrap(),
            r#""websocket""#
        );
        assert_eq!(
            serde_json::to_string(&Protocol::GrpcWeb).unwrap(),
            r#""grpc_web""#
        );
    }

    #[test]
    fn only_http_shaped_protocols_use_methods() {
        assert!(Protocol::Http.uses_http_method());
        assert!(Protocol::Graphql.uses_http_method());
        assert!(!Protocol::Sse.uses_http_method());
        assert!(!Protocol::WebSocket.uses_http_method());
    }

    #[test]
    fn compact_badges_distinguish_non_http_protocols() {
        assert_eq!(Protocol::Http.badge_label(), None);
        assert_eq!(Protocol::Graphql.badge_label(), None);
        assert_eq!(Protocol::Sse.badge_label(), Some("SSE"));
        assert_eq!(Protocol::WebSocket.badge_label(), Some("WS"));
        assert_eq!(Protocol::Tcp.badge_label(), Some("TCP"));
        assert_eq!(Protocol::GrpcWeb.badge_label(), Some("gRPC-Web"));
        assert_eq!(Protocol::SocketIo.badge_label(), Some("Socket.IO"));
    }
}
