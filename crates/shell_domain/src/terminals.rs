//! 终端标签的领域实现：本地终端（可带工作目录）与 SSH 终端的公共入口。

use domain_api::HomeContext;
use gpui::{App, AppContext as _, Window};
use one_core::settings::AppSettings;
use one_core::tab_container::TabItem;
use terminal_view::TerminalWorkspace;

/// 在指定目录打开本地终端（`working_dir` 为空时使用默认目录）。
pub(crate) fn open_local_terminal_with_dir(
    working_dir: Option<String>,
    home: &mut dyn HomeContext,
    window: &mut Window,
    cx: &mut App,
) -> bool {
    let Ok(config) = terminal::local_config_from_settings(AppSettings::global(cx), working_dir)
    else {
        return false;
    };
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    let tab_id = format!("local-terminal-{timestamp}");
    let terminal = cx.new(|cx| TerminalWorkspace::new_with_index(config, None, window, cx));
    let item = TabItem::new(tab_id, "terminal", terminal);
    home.open_tab_with_focus(item, window, cx)
}
