//! 工作台终端面板的提供者契约。
//!
//! 终端视图属于 shell 领域，而工作台是共享 AI 能力：外壳只负责摆放面板，
//! 由产品在启动时注册提供者，避免共享 crate 反向依赖终端实现。

use std::path::Path;
use std::sync::{Arc, OnceLock};

use gpui::{AnyView, App, Window};

/// 「新建页签 → 终端」用的面板工厂。
pub type TerminalPanelFactory = Arc<dyn Fn(&Path, &mut Window, &mut App) -> AnyView + Send + Sync>;

/// 工作台终端面板提供者。
pub trait WorkbenchTerminalProvider: Send + Sync {
    /// 初始终端面板（工作区根目录）；返回 `None` 表示当前产品不提供终端。
    fn terminal_panel(&self, _root: &Path, _window: &mut Window, _cx: &mut App) -> Option<AnyView> {
        None
    }

    /// 多例面板工厂；`None` 时「新建页签 → 终端」不可用。
    fn panel_factory(&self) -> Option<TerminalPanelFactory> {
        None
    }
}

static PROVIDER: OnceLock<Arc<dyn WorkbenchTerminalProvider>> = OnceLock::new();

pub fn set_workbench_terminal_provider(provider: Arc<dyn WorkbenchTerminalProvider>) {
    let _ = PROVIDER.set(provider);
}

pub fn workbench_terminal_provider() -> Option<&'static Arc<dyn WorkbenchTerminalProvider>> {
    PROVIDER.get()
}
