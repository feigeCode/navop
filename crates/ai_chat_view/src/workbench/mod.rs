//! AI 工作台外壳。
//!
//! 把「会话 / 审阅 / 文件 / 终端」四个面板组织成可停靠工作台：
//! 左侧会话导航可折叠，中间内容区单选切换，右侧与底部共享三边停靠。
//!
//! 外壳本身不含业务逻辑：面板以 [`WorkbenchPanelEntry`] 注入，
//! 会话与资源状态仍由各自 crate 持有。

mod shell;
mod state;

pub use shell::{
    WorkbenchPanelEntry, WorkbenchShell, WorkbenchShellConfig,
};
pub use state::{WorkbenchDockLayout, WorkbenchPanelKind, WorkbenchState, dock_region_width};
