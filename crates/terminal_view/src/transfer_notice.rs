//! 上传 / 下载的终态提示（issue #199）。
//!
//! 传输进度已经进了右上角的全局后台任务面板，但那个面板需要用户再点开一次才看得到
//! 结果，完成 / 失败时没有任何即时反馈。这里给终态补一条 toast。
//!
//! 文案拼装抽成不依赖 i18n / `Window` 的纯函数，方便直接单测；
//! 只有最外层的状态词与方向词走 `t!`。

use gpui_component::notification::Notification;
use rust_i18n::t;

/// 传输方向，决定 toast 文案前缀（上传 / 下载）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TransferAction {
    Upload,
    Download,
}

impl TransferAction {
    fn label(self) -> String {
        match self {
            Self::Upload => t!("TerminalZmodem.upload_title").to_string(),
            Self::Download => t!("TerminalZmodem.download_title").to_string(),
        }
    }
}

/// 终态状态词：成功用「完成」，失败用「失败」。
fn finish_state_label(error: Option<&str>) -> String {
    match error {
        Some(_) => t!("FileManager.transfer_failed").to_string(),
        None => t!("FileManager.transfer_done").to_string(),
    }
}

/// 在状态词后面附上失败原因。
fn append_failure_reason(message: &mut String, error: Option<&str>) {
    if let Some(error) = error.map(str::trim).filter(|error| !error.is_empty()) {
        message.push_str(": ");
        message.push_str(error);
    }
}

/// 拼装终态文案：`下载 · sarasa.zip · 完成`；文件名未知时退化为 `下载 · 完成`。
pub(crate) fn transfer_finish_message(
    action_label: &str,
    file_name: Option<&str>,
    state_label: &str,
) -> String {
    match file_name.map(str::trim).filter(|name| !name.is_empty()) {
        Some(name) => format!("{action_label} · {name} · {state_label}"),
        None => format!("{action_label} · {state_label}"),
    }
}

/// 构造传输终态 toast。
///
/// `error` 为 `None` 表示成功；`Some` 表示失败，并附上错误原因。
/// 失败通知不自动消失，避免错误信息还没读完就没了。
pub(crate) fn transfer_finish_notification(
    action: TransferAction,
    file_name: Option<&str>,
    error: Option<&str>,
) -> Notification {
    let mut message = transfer_finish_message(
        &action.label(),
        file_name,
        &finish_state_label(error),
    );
    append_failure_reason(&mut message, error);

    match error {
        Some(_) => Notification::error(message).autohide(false),
        None => Notification::success(message),
    }
}

#[cfg(test)]
mod tests {
    use super::{TransferAction, append_failure_reason, transfer_finish_message};

    #[test]
    fn message_includes_direction_file_name_and_state() {
        let message = transfer_finish_message("下载", Some("sarasa.zip"), "完成");

        assert_eq!("下载 · sarasa.zip · 完成", message);
    }

    #[test]
    fn message_degrades_to_direction_and_state_without_file_name() {
        assert_eq!("上传 · 失败", transfer_finish_message("上传", None, "失败"));
        assert_eq!(
            "上传 · 失败",
            transfer_finish_message("上传", Some("   "), "失败")
        );
    }

    #[test]
    fn message_trims_file_name() {
        assert_eq!(
            "下载 · a.txt · 完成",
            transfer_finish_message("下载", Some("  a.txt "), "完成")
        );
    }

    #[test]
    fn failure_reason_is_appended_only_when_present() {
        let mut with_reason = String::from("下载 · a.txt · 失败");
        append_failure_reason(&mut with_reason, Some("  connection reset  "));
        assert_eq!("下载 · a.txt · 失败: connection reset", with_reason);

        let mut blank_reason = String::from("下载 · a.txt · 失败");
        append_failure_reason(&mut blank_reason, Some("   "));
        append_failure_reason(&mut blank_reason, None);
        assert_eq!("下载 · a.txt · 失败", blank_reason);
    }

    #[test]
    fn direction_labels_are_distinct() {
        assert_ne!(TransferAction::Upload, TransferAction::Download);
    }
}
