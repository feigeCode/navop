//! RocketMQ 自研运行时:Remoting 私有协议 + 中间件标准管理接口。
//!
//! - [`protocol`]:Java Remoting 协议帧编解码(4B 总长 + 4B 头描述 + JSON 头 + body),
//!   仅实现 JSON 序列化类型,兼容 4.x/5.x 服务端;
//! - [`remoting`]:tokio TCP 长连接管理(opaque 配对/超时/NameServer 轮询/SSH 隧道);
//! - [`acl`]:4.x ACL HMAC-SHA1 请求签名(5.x 兼容模式同规则);
//! - [`admin`]:[`middleware_runtime::MiddlewareAdmin`] 落地,四页管理数据源;
//! - [`message`]:commitlog 消息二进制解码与消息 ID 解析。
//!
//! 无 GPUI 依赖;不引入 JVM/本地 C 依赖。

rust_i18n::i18n!("../rocketmq_view/locales", fallback = "zh-CN");

pub mod acl;
pub mod admin;
pub mod message;
pub mod protocol;
pub mod remoting;
pub mod types;

pub use acl::AclCredentials;
pub use admin::RocketmqConnection;
pub use remoting::RemotingClient;
pub use types::{RocketmqError, RocketmqParams};

/// 后端类型(一期仅进程内自研实现,IPC sidecar 预留)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RocketmqBackendKind {
    /// 进程内 Remoting 协议实现
    Builtin,
    /// 不可用
    Unavailable,
}

/// 默认后端:始终为 Builtin(纯 Rust 自研实现,无外置依赖)
pub const fn default_backend_kind() -> RocketmqBackendKind {
    RocketmqBackendKind::Builtin
}

/// RocketMQ 连接工厂
#[derive(Clone, Copy, Debug)]
pub enum RocketmqConnectionFactory {
    /// 进程内实现
    Builtin,
    /// 不可用
    Unavailable,
}

impl RocketmqConnectionFactory {
    /// 默认工厂
    pub fn default_factory() -> Self {
        match default_backend_kind() {
            RocketmqBackendKind::Builtin => Self::Builtin,
            RocketmqBackendKind::Unavailable => Self::Unavailable,
        }
    }

    /// 后端类型
    pub fn backend_kind(&self) -> RocketmqBackendKind {
        match self {
            Self::Builtin => RocketmqBackendKind::Builtin,
            Self::Unavailable => RocketmqBackendKind::Unavailable,
        }
    }

    /// 创建管理连接(实现 [`middleware_runtime::MiddlewareAdmin`])
    pub fn create(
        &self,
        params: std::sync::Arc<RocketmqParams>,
    ) -> Result<Box<dyn middleware_runtime::MiddlewareAdmin>, RocketmqError> {
        match self {
            Self::Builtin => Ok(Box::new(RocketmqConnection::new(params)?)),
            Self::Unavailable => Err(RocketmqError::Config(
                "RocketMQ 后端不可用: builtin 为唯一实现".into(),
            )),
        }
    }

    /// 创建原生连接(需要 test_connection 等扩展方法时使用)
    pub fn create_native(
        &self,
        params: std::sync::Arc<RocketmqParams>,
    ) -> Result<RocketmqConnection, RocketmqError> {
        match self {
            Self::Builtin => RocketmqConnection::new(params),
            Self::Unavailable => Err(RocketmqError::Config(
                "RocketMQ 后端不可用: builtin 为唯一实现".into(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn factory_defaults_to_builtin() {
        let factory = RocketmqConnectionFactory::default_factory();
        assert_eq!(factory.backend_kind(), RocketmqBackendKind::Builtin);

        let mut params = RocketmqParams::default();
        params.namesrv_addrs = vec!["127.0.0.1:9876".into()];
        let connection = factory.create_native(std::sync::Arc::new(params));
        assert!(connection.is_ok());

        // trait 对象可用
        let admin = factory
            .create(std::sync::Arc::new(RocketmqParams::default()))
            .expect("工厂应产出 MiddlewareAdmin");
        let caps = admin.capabilities();
        assert!(caps.topics && caps.groups && caps.message_query);
    }
}
