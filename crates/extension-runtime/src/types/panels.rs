use std::path::PathBuf;

/// 一个扩展贡献的面板（`contributes.panels` 的注册态）。
///
/// 面板是同一 nsh app 的附加 surface：入口复用 app 的 entry，宿主以
/// `host.params.surface = "panel"` + `host.params.panel = <id>` 启动独立的
/// AppView 实例。`anchor` 决定挂到宿主哪个 UI 锚点（当前仅终端侧边栏）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredPanelContribution {
    pub extension_id: String,
    pub extension_version: String,
    /// 面板 id（扩展内唯一）。
    pub id: String,
    /// 面板标题；缺省回退 shellApp.title（注册时已解析）。
    pub title: String,
    /// 图标文件路径（相对扩展根解析后的绝对路径）。
    pub icon_path: Option<PathBuf>,
    /// 宿主 UI 锚点（`terminal-sidebar` / `db-sidebar`）。
    pub anchor: String,
    /// 同锚点内排序权重，小的在前。
    pub order: i64,
    /// 显示条件（when 子句），空串恒可见。
    pub when_clause: Option<String>,
    /// 所属 app 的 entry（面板与 tab 共用同一 bundle 入口）。
    pub entry_path: PathBuf,
    /// 面板需要的能力模块（继承 shellApp.modules）。
    pub modules: std::collections::BTreeSet<crate::extension::manifest::ShellHostModule>,
}
