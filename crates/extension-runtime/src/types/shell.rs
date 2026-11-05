use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use crate::extension::manifest::ShellHostModule;

/// 一个扩展的 nsh app（`contributes.shellApp`）。
///
/// 一个扩展最多一个 app：插件内的多页面由 app 自己用路由表达，宿主只认
/// 「打开这个扩展的 app，初始 route 是 X」。旧模型（每个页面一个 shellView /
/// 一个 nsh 包）已删除。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredShellAppContribution {
    pub extension_id: String,
    pub extension_version: String,
    /// 清单 `shellApp.title`，缺省回退清单的 `name`。
    pub title: String,
    pub description: Option<String>,
    pub icon_path: Option<PathBuf>,
    pub extension_root: PathBuf,
    pub entry_path: PathBuf,
    /// 工具箱卡片分类（只有没有连接的 app 才进工具箱）。
    pub category: Option<String>,
    pub keywords: Vec<String>,
    pub backends: BTreeMap<String, String>,
    pub modules: BTreeSet<ShellHostModule>,
    pub permissions: Vec<String>,
    pub shell_api_version: String,
    /// 声明的 nsh 版本门；`Some` 表示这个 app 由 nsh 宿主装载。
    pub required_nsh_version: Option<String>,
}
