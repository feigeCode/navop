//! AI 工作台的终端面板提供者：把原生终端视图注入共享 AI 工作台。
//!
//! 扩展页面的终端**不**走这里 —— 它是 nsh 的原生组件
//! （`crate::native_terminal`）。本文件只负责 `ai_chat_view` 的工作台面板，
//! 与扩展机制无关。

use gpui::{App, AppContext as _, Window};

/// 工作台终端面板提供者：把原生终端视图注入共享 AI 工作台。
pub(crate) fn install_workbench_terminal_provider() {
    ai_chat_view::set_workbench_terminal_provider(std::sync::Arc::new(
        ShellWorkbenchTerminalProvider,
    ));
}

struct ShellWorkbenchTerminalProvider;

fn terminal_config(root: &std::path::Path) -> terminal::LocalConfig {
    terminal::LocalConfig {
        shell: None,
        args: Vec::new(),
        working_dir: Some(root.to_string_lossy().into_owned()),
        env: Vec::new(),
    }
}

impl ai_chat_view::WorkbenchTerminalProvider for ShellWorkbenchTerminalProvider {
    fn terminal_panel(
        &self,
        root: &std::path::Path,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<gpui::AnyView> {
        let config = terminal_config(root);
        Some(
            cx.new(|cx| terminal_view::TerminalView::new(config, window, cx).with_workspace_pane())
                .into(),
        )
    }

    fn panel_factory(&self) -> Option<ai_chat_view::TerminalPanelFactory> {
        Some(std::sync::Arc::new(
            |root: &std::path::Path, window: &mut Window, cx: &mut App| {
                let config = terminal_config(root);
                cx.new(|cx| {
                    terminal_view::TerminalView::new(config, window, cx).with_workspace_pane()
                })
                .into()
            },
        ))
    }
}
