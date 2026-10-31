//! 文件关联注册。
//!
//! 实现已迁入 [`app_kit::file_association`]：接管哪些扩展名由各产品的
//! `AppIdentity::file_associations` 决定。这里只做转发，调用点保持原样。

pub use app_kit::file_association::*;
