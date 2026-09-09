//! RocketMQ 视图层
//!
//! 提供 RocketMQ 连接管理的用户界面组件,包括:
//! - 连接管理与全局状态(GlobalRocketmqState)
//! - 连接树视图(连接 -> Broker/Topic 层级,四态连接节点)
//! - 连接页签(左侧连接树 + 右侧 TabContainer 承载标准四页管理组件)
//! - 连接表单窗口(复用 connection-form 声明式中间件表单引擎)

use gpui::App;
use rocketmq_runtime::RocketmqConnectionFactory;

rust_i18n::i18n!("locales", fallback = "zh-CN");

// 核心模块
pub mod manager;

// 视图模块
pub mod rocketmq_form_window;
pub mod rocketmq_tab;
pub mod rocketmq_tree_view;

// 核心导出
pub use manager::{GlobalRocketmqState, RocketmqManager};
pub use rocketmq_runtime::{RocketmqConnection, RocketmqError};

// 视图导出
pub use rocketmq_form_window::{
    RocketmqFormAdapter, RocketmqFormConfig, RocketmqFormSavedCallback, RocketmqFormWindow,
    rocketmq_form_tab_groups,
};
pub use rocketmq_tab::RocketmqTabView;
pub use rocketmq_tree_view::{RocketmqTreeView, RocketmqTreeViewEvent};

/// 初始化 RocketMQ 模块(默认工厂:进程内 Remoting 协议实现)
pub fn init(cx: &mut App) {
    cx.set_global(GlobalRocketmqState::new(
        RocketmqConnectionFactory::default_factory(),
    ));
}

/// 初始化 RocketMQ 模块(指定连接工厂,用于测试)
pub fn init_with_factory(cx: &mut App, factory: RocketmqConnectionFactory) {
    cx.set_global(GlobalRocketmqState::new(factory));
}
