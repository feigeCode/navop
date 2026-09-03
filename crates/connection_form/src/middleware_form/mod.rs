//! 通用中间件声明式连接表单引擎
//!
//! 与数据库连接表单(`db_view::DbConnectionForm`)同页面模式的通用实现:
//! 声明式 `TabGroup`/`FormField` 配置驱动标签页渲染,固定区块(钥匙串/
//! 工作区/团队/云同步/SSH 隧道)由引擎统一处理。各中间件(MQTT,后续
//! RocketMQ/Kafka 等)只需提供一个 `MiddlewareFormAdapter` 实现参数
//! 映射,并声明自己的标签页配置,即可复用整套页面与保存/测试流程。

pub mod adapter;
pub mod declarative;
pub mod form;
pub mod window;

pub use adapter::{FormSnapshot, MiddlewareFormAdapter, MiddlewareFormSavedCallback};
pub use declarative::{
    FormField, FormFieldType, FormVisibilityRule, TabGroup, notes_tab_group, ssh_tab_group,
};
pub use form::{MiddlewareConnectionForm, MiddlewareFormConfig, MiddlewareFormEvent};
pub use window::{MiddlewareFormWindow, MiddlewareFormWindowConfig};
