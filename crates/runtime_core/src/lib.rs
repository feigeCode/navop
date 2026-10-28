//! 与具体连接域无关的运行时工具层。
//!
//! 这里只放应用信息、工作区等三产品都需要的能力；数据库、Redis、SFTP
//! 的工具分别放在各自的 runtime 包里。

pub mod app_info;
pub mod workspaces;

pub use app_info::{builtin_tool_registry, builtin_tool_registry_with_version};
