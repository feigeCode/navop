//! 托盘菜单的纯逻辑：菜单 id ↔ 命令映射、会话项截断、关闭按钮策略。
//!
//! 这里刻意不碰任何平台对象：全部是可用单测覆盖的纯函数，平台后端只负责把结果
//! 渲染成 `tray_icon::menu::Menu`。

use one_core::settings::CloseButtonBehavior;

use super::{MainWindowCloseAction, TrayCommand};

/// 托盘菜单项 id。
pub(super) const MENU_ID_SHOW: &str = "navop.tray.show";
pub(super) const MENU_ID_QUIT: &str = "navop.tray.quit";
pub(super) const MENU_ID_NEW_CONNECTION: &str = "navop.tray.new-connection";
pub(super) const MENU_ID_OPEN_SETTINGS: &str = "navop.tray.open-settings";
/// 「会话」菜单项 id 前缀，后面直接拼标签 id。
pub(super) const MENU_ID_SESSION_PREFIX: &str = "navop.tray.session.";

/// 菜单里最多列出的会话数：托盘菜单是一级平铺列表，放着不管会高到点不动。
pub(super) const TRAY_SESSION_LIMIT: usize = 8;
/// 会话标题最长保留的字符数，超出用省略号收尾。
pub(super) const TRAY_SESSION_LABEL_MAX_CHARS: usize = 40;

/// 关闭按钮的纯策略：只有托盘确实可用时才可能隐藏窗口，否则必须回退到退出确认，
/// 不能制造一个无法恢复的隐藏窗口。
///
/// 托盘可用时再按用户偏好细分：`Ask` 交回弹窗由用户当场选择（并可在弹窗里记住），
/// `MinimizeToTray` / `Quit` 是用户已经固化过的选择，不再打扰。
pub const fn main_window_close_action(
    tray_ready: bool,
    behavior: CloseButtonBehavior,
) -> MainWindowCloseAction {
    if !tray_ready {
        return MainWindowCloseAction::RequestQuit;
    }
    match behavior {
        CloseButtonBehavior::Ask => MainWindowCloseAction::AskUser,
        CloseButtonBehavior::MinimizeToTray => MainWindowCloseAction::HideToTray,
        CloseButtonBehavior::Quit => MainWindowCloseAction::RequestQuit,
    }
}

/// 标签列表 → 托盘菜单项 `(菜单 id, 显示文案)` 的纯映射：截断数量、压平换行、
/// 超长标题加省略号。超出上限的会话直接丢掉（托盘不是完整的标签管理器）。
pub(super) fn session_menu_entries(sessions: &[(String, String)]) -> Vec<(String, String)> {
    sessions
        .iter()
        .take(TRAY_SESSION_LIMIT)
        .map(|(id, title)| {
            (
                format!("{MENU_ID_SESSION_PREFIX}{id}"),
                truncate_session_label(title),
            )
        })
        .collect()
}

fn truncate_session_label(title: &str) -> String {
    let single_line: String = title
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let trimmed = single_line.trim();
    if trimmed.chars().count() <= TRAY_SESSION_LABEL_MAX_CHARS {
        return trimmed.to_owned();
    }
    let head: String = trimmed.chars().take(TRAY_SESSION_LABEL_MAX_CHARS).collect();
    format!("{head}…")
}

/// 菜单项 id → 命令的纯映射。未知 id（例如未来新增菜单项）一律忽略。
pub(super) fn command_for_menu_id(id: &str) -> Option<TrayCommand> {
    match id {
        MENU_ID_SHOW => Some(TrayCommand::ShowMainWindow),
        MENU_ID_QUIT => Some(TrayCommand::QuitApplication),
        MENU_ID_NEW_CONNECTION => Some(TrayCommand::NewConnection),
        MENU_ID_OPEN_SETTINGS => Some(TrayCommand::OpenSettings),
        _ => id
            .strip_prefix(MENU_ID_SESSION_PREFIX)
            .filter(|tab_id| !tab_id.is_empty())
            .map(|tab_id| TrayCommand::ActivateSession {
                id: tab_id.to_owned(),
            }),
    }
}
