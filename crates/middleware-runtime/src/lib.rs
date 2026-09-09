//! 中间件标准契约(无 GPUI 依赖):统一的数据模型、能力位与管理接口。
//!
//! 标准属性提取自 RocketMQ Console(apache/rocketmq-dashboard)的页面模型,
//! 由 MQTT/RocketMQ 等消息中间件 runtime 实现,供通用管理视图(middleware_view)消费。

rust_i18n::i18n!("../middleware_view/locales", fallback = "zh-CN");

pub mod connection;
pub mod types;

pub use connection::MiddlewareAdmin;
pub use types::*;
