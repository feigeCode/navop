use std::path::PathBuf;

use crate::extension::manifest::{ResourceConnectionForm, ResourceConnectionTerminal};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredResourceConnectionContribution {
    pub extension_id: String,
    pub extension_root: PathBuf,
    pub id: String,
    pub label: String,
    pub description: Option<String>,
    pub icon_path: Option<PathBuf>,
    /// 提供者运行时 key;终端型连接(`terminal` 为 `Some`)为空字符串。
    pub runtime_id: String,
    pub resource_type: String,
    pub shell_view_id: Option<String>,
    /// 终端型连接的启动声明;`Some` 时连接不经过提供者进程,由宿主直接开终端。
    pub terminal: Option<ResourceConnectionTerminal>,
    pub form: ResourceConnectionForm,
}
