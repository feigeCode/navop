//! 自动更新入口。
//!
//! 实现已迁入 [`app_kit::update`]：独立产品（`navop-shell` / `navop-data`）与本产品
//! 共用同一套更新逻辑，更新 feed 由各自的 `AppIdentity` 派生。这里只做转发，
//! 使 `crate::update::…` 的调用点与启动顺序断言保持原样。

pub use app_kit::update::*;
