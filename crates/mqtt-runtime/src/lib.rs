//! MQTT 领域契约与后端选择(无 GPUI 依赖)。

rust_i18n::i18n!("../mqtt_view/locales", fallback = "zh-CN");

pub mod connection;
pub mod pubsub;
pub mod types;

#[cfg(feature = "builtin-mqtt")]
mod builtin;

#[cfg(feature = "builtin-mqtt")]
pub use builtin::MqttConnectionImpl as BuiltinMqttConnection;

pub use connection::MqttConnection;
pub use pubsub::{MQTT_MESSAGE_CHANNEL_CAPACITY, MqttPubSubHandle};
pub use types::*;

/// MQTT 后端类型(一期仅提供 builtin;IPC sidecar 预留)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MqttBackendKind {
    /// 外置 IPC 驱动(预留)
    Ipc,
    /// 进程内 rumqttc 实现
    Builtin,
    /// 不可用
    Unavailable,
}

/// 默认后端:启用 builtin-mqtt feature 时为 Builtin,否则 Unavailable。
pub const fn default_backend_kind() -> MqttBackendKind {
    #[cfg(feature = "builtin-mqtt")]
    {
        MqttBackendKind::Builtin
    }
    #[cfg(not(feature = "builtin-mqtt"))]
    {
        MqttBackendKind::Unavailable
    }
}

/// MQTT 连接工厂:按后端创建连接。
#[derive(Clone, Copy, Debug)]
pub enum MqttConnectionFactory {
    /// 进程内 rumqttc 实现(仅 feature = "builtin-mqtt")
    Builtin,
    /// 不可用(未编译 builtin 且未安装外置驱动)
    Unavailable,
}

impl MqttConnectionFactory {
    pub fn default_factory() -> Self {
        match default_backend_kind() {
            MqttBackendKind::Builtin => Self::Builtin,
            _ => Self::Unavailable,
        }
    }

    pub fn backend_kind(&self) -> MqttBackendKind {
        match self {
            Self::Builtin => MqttBackendKind::Builtin,
            Self::Unavailable => MqttBackendKind::Unavailable,
        }
    }

    #[cfg_attr(not(feature = "builtin-mqtt"), allow(unused_variables))]
    pub fn create(
        &self,
        config: MqttConnectionConfig,
    ) -> Result<Box<dyn MqttConnection>, MqttError> {
        match self {
            #[cfg(feature = "builtin-mqtt")]
            Self::Builtin => Ok(Box::new(crate::builtin::MqttConnectionImpl::new(config))),
            #[cfg(not(feature = "builtin-mqtt"))]
            Self::Builtin => Err(MqttError::Config(
                "builtin-mqtt feature is not enabled".to_string(),
            )),
            Self::Unavailable => Err(MqttError::Config(
                "mqtt backend unavailable: build with `builtin-mqtt` or install the mqtt driver"
                    .to_string(),
            )),
        }
    }
}
