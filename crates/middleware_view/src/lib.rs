//! 中间件通用管理视图(基于 gpui-component)
//!
//! 面向 `dyn MiddlewareAdmin` 编程的通用四页管理组件(概览/Topic/订阅组/消息查询),
//! 布局与属性列对齐 RocketMQ Console;组件不感知具体中间件类型,
//! 全部按 [`middleware_runtime::MiddlewareCapabilities`] 降级渲染。

rust_i18n::i18n!("locales", fallback = "zh-CN");

mod common;
mod groups_page;
mod messages_page;
mod overview_page;
mod registry;
mod topics_page;

pub use groups_page::MiddlewareGroupsPage;
pub use messages_page::MiddlewareMessagesPage;
pub use overview_page::MiddlewareOverviewPage;
pub use registry::MiddlewarePages;
pub use topics_page::MiddlewareTopicsPage;

/// 中间件管理接口句柄(跨线程共享的适配器实例)
pub type MiddlewareAdminHandle = std::sync::Arc<dyn middleware_runtime::MiddlewareAdmin>;
