use serde::{Deserialize, Serialize};

/// 扩展向宿主 UI 锚点贡献的面板（`contributes.panels`）。
///
/// 一个面板 = 同一 nsh app 的第二个 surface：入口复用 `shellApp.entry`，
/// 宿主以 `host.params.surface = "panel"` + `host.params.panel = <id>` 启动
/// 一个独立的 AppView 实例，页面按 surface 分支渲染。
///
/// 锚点可选终端侧边栏（`anchor: "terminal-sidebar"`，默认）或数据库侧边栏
/// （`anchor: "db-sidebar"`）。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Default)]
pub struct PanelContrib {
    /// 面板 id（扩展内唯一），进入 `host.params.panel` 与持久化 key。
    pub id: String,
    /// 侧栏按钮与面板标题；缺省回退 shellApp.title。
    pub title: Option<String>,
    /// 图标路径（相对扩展根）；缺省用扩展连接图标。
    pub icon: Option<String>,
    /// 宿主 UI 锚点；支持 `terminal-sidebar`（默认）与 `db-sidebar`。
    #[serde(default = "default_panel_anchor")]
    pub anchor: String,
    /// 同锚点内的排序权重，小的在前。
    #[serde(default)]
    pub order: i64,
    /// 显示条件（when 子句），空串恒可见。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<String>,
}

pub fn default_panel_anchor() -> String {
    "terminal-sidebar".to_string()
}
