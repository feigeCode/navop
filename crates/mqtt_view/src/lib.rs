//! MQTT 视图层
//!
//! 提供 MQTT 连接管理的用户界面组件,包括:
//! - 连接管理与全局状态(GlobalMqttState)
//! - 连接树视图(连接/订阅节点)
//! - 单连接操作页(订阅/消息流/发布)
//! - 连接表单窗口
//! - 侧边栏(AI 聊天)

use gpui::App;
use mqtt_runtime::MqttConnectionFactory;

rust_i18n::i18n!("locales", fallback = "zh-CN");

// 核心模块
pub mod manager;

// 视图模块
pub mod mqtt_form_window;
pub mod mqtt_tab;
pub mod mqtt_tree_view;
pub mod sidebar;
pub mod subscribe_view;

// 核心导出
pub use manager::{GlobalMqttState, MqttManager};
pub use mqtt_runtime::{MqttConnection, MqttError};

// 视图导出
pub use mqtt_form_window::{MqttFormConfig, MqttFormSavedCallback, MqttFormWindow};
pub use mqtt_tab::MqttTabView;
pub use mqtt_tree_view::{MqttTreeView, MqttTreeViewEvent};
pub use sidebar::{MqttSidebar, MqttSidebarEvent};
pub use subscribe_view::{MqttSubscribeView, MqttSubscribeViewEvent};

/// 初始化 MQTT 模块(默认工厂:按编译 feature 选择后端)
pub fn init(cx: &mut App) {
    cx.set_global(GlobalMqttState::new(
        MqttConnectionFactory::default_factory(),
    ));
}

/// 初始化 MQTT 模块(指定连接工厂,用于测试或外置驱动)
pub fn init_with_factory(cx: &mut App, factory: MqttConnectionFactory) {
    cx.set_global(GlobalMqttState::new(factory));
}
