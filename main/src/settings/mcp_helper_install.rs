use crate::settings::mcp_helper_progress::{
    McpHelperInstallProgressStatus, begin_helper_install_progress, clear_helper_install_progress,
    helper_install_progress_callback, helper_install_progress_snapshot,
};
use anyhow::Error;
use extension_runtime::mcp_helper_install::install_mcp_helper_from_marketplace_with_progress;
use gpui::{App, AppContext, AsyncApp, ParentElement, Styled, Window, div};
use gpui_component::{
    ActiveTheme, Disableable, WindowExt,
    button::{Button, ButtonVariants as _},
    h_flex,
    notification::Notification,
    setting::{SettingField, SettingItem},
    v_flex,
};
use public_mcp::client_config::{
    ClientConfigHealth, ClientConfigInstall, helper_unavailable_health,
};
use rust_i18n::t;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

static MCP_HELPER_INSTALLING: AtomicBool = AtomicBool::new(false);
const MCP_HELPER_PROGRESS_REFRESH_INTERVAL: Duration = Duration::from_millis(120);

pub(crate) fn mcp_helper_install_item() -> SettingItem {
    SettingItem::new(
        t!("Settings.General.Mcp.install_helper"),
        SettingField::render(|_, _, cx| {
            let installing = helper_install_in_progress();
            let installed = !installing && helper_is_installed();
            let status_text = helper_install_status();
            let mut container = v_flex().gap_1();
            if !installed {
                container = container.child(
                    h_flex().child(
                        Button::new(mcp_helper_install_item_id())
                            .primary()
                            .label(t!("Settings.General.Mcp.install_helper_button"))
                            .disabled(installing)
                            .on_click(|_, window, cx| install_mcp_helper(window, cx)),
                    ),
                );
            }
            container.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(status_text),
            )
        }),
    )
    .description(t!("Settings.General.Mcp.install_helper_desc").to_string())
}

pub(crate) fn mcp_helper_install_item_id() -> &'static str {
    "mcp-install-helper"
}

fn helper_install_status() -> String {
    helper_install_status_for_install(
        helper_install_in_progress(),
        ClientConfigInstall::from_current_app(),
    )
}

fn helper_install_status_for_install(
    install_in_progress: bool,
    install: anyhow::Result<ClientConfigInstall>,
) -> String {
    if install_in_progress {
        return helper_install_progress_status_message(helper_install_progress_snapshot().status());
    }
    match install {
        Ok(install) => helper_install_status_for_resolved_install(&install),
        Err(error) => error.to_string(),
    }
}

fn helper_install_progress_status_message(status: McpHelperInstallProgressStatus) -> String {
    match status {
        McpHelperInstallProgressStatus::Idle | McpHelperInstallProgressStatus::Preparing => {
            t!("Settings.General.Mcp.helper_status_installing").to_string()
        }
        McpHelperInstallProgressStatus::Connecting => {
            t!("Settings.General.Mcp.helper_status_connecting").to_string()
        }
        McpHelperInstallProgressStatus::Downloading {
            percent: Some(percent),
        } => t!(
            "Settings.General.Mcp.helper_status_downloading",
            percent = percent
        )
        .to_string(),
        McpHelperInstallProgressStatus::Downloading { percent: None } => {
            t!("Settings.General.Mcp.helper_status_downloading_unknown").to_string()
        }
        McpHelperInstallProgressStatus::Retrying => {
            t!("Settings.General.Mcp.helper_status_retrying").to_string()
        }
        McpHelperInstallProgressStatus::Installing => {
            t!("Settings.General.Mcp.helper_status_installing_package").to_string()
        }
    }
}

fn helper_install_status_for_resolved_install(install: &ClientConfigInstall) -> String {
    let path = install.launcher_path.display().to_string();
    match helper_unavailable_health(&install.launcher_path) {
        Ok(None) => t!("Settings.General.Mcp.helper_status_installed", path = path).to_string(),
        Ok(Some(ClientConfigHealth::MissingHelper)) => {
            t!("Settings.General.Mcp.helper_status_missing", path = path).to_string()
        }
        Ok(Some(ClientConfigHealth::UnusableHelper)) => {
            t!("Settings.General.Mcp.helper_status_unusable", path = path).to_string()
        }
        Ok(Some(_)) => t!("Settings.General.Mcp.helper_status_missing", path = path).to_string(),
        Err(error) => error.to_string(),
    }
}

fn helper_is_installed() -> bool {
    match ClientConfigInstall::from_current_app() {
        Ok(install) => matches!(helper_unavailable_health(&install.launcher_path), Ok(None)),
        Err(_) => false,
    }
}

fn install_mcp_helper(window: &mut Window, cx: &mut App) {
    if !try_begin_install(&MCP_HELPER_INSTALLING) {
        window.push_notification(
            Notification::info(
                t!("Settings.General.Mcp.install_helper_already_running").to_string(),
            )
            .autohide(true),
            cx,
        );
        return;
    }
    begin_helper_install_progress();
    let http_client = cx.http_client();
    let progress_window_handle = window.window_handle();
    let finish_window_handle = window.window_handle();
    let progress_finished = Arc::new(AtomicBool::new(false));
    let progress_finished_for_watcher = Arc::clone(&progress_finished);
    window.refresh();
    window.push_notification(
        Notification::info(t!("Settings.General.Mcp.install_helper_started").to_string())
            .autohide(true),
        cx,
    );

    cx.spawn(async move |cx: &mut AsyncApp| {
        loop {
            if progress_finished_for_watcher.load(Ordering::Acquire) {
                break;
            }
            cx.background_executor()
                .timer(MCP_HELPER_PROGRESS_REFRESH_INTERVAL)
                .await;
            refresh_helper_install_window(progress_window_handle, cx);
        }
        refresh_helper_install_window(progress_window_handle, cx);
    })
    .detach();

    let progress_callback = helper_install_progress_callback();
    cx.spawn(async move |cx: &mut AsyncApp| {
        let outcome =
            install_mcp_helper_from_marketplace_with_progress(http_client, progress_callback)
                .await
                .map(|summary| summary.path.display().to_string())
                .map_err(format_install_error);
        progress_finished.store(true, Ordering::Release);
        clear_helper_install_progress();
        finish_install(&MCP_HELPER_INSTALLING);
        let _ = cx.update_window(finish_window_handle, |_, window, cx| {
            notify_mcp_helper_install_outcome(window, cx, outcome);
            window.refresh();
        });
    })
    .detach();
}

fn refresh_helper_install_window(window_handle: gpui::AnyWindowHandle, cx: &mut AsyncApp) {
    let _ = cx.update_window(window_handle, |_, window, _| {
        window.refresh();
    });
}

fn notify_mcp_helper_install_outcome(
    window: &mut Window,
    cx: &mut App,
    outcome: Result<String, String>,
) {
    match outcome {
        Ok(path) => window.push_notification(
            Notification::success(
                t!("Settings.General.Mcp.install_helper_success", path = path).to_string(),
            )
            .autohide(true),
            cx,
        ),
        Err(error) => window.push_notification(
            Notification::error(
                t!("Settings.General.Mcp.install_helper_failed", error = error).to_string(),
            )
            .autohide(true),
            cx,
        ),
    }
}

fn format_install_error(error: Error) -> String {
    format!("{error:#}")
}

fn helper_install_in_progress() -> bool {
    install_in_progress(&MCP_HELPER_INSTALLING)
}

fn try_begin_install(state: &AtomicBool) -> bool {
    state
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
}

fn finish_install(state: &AtomicBool) {
    state.store(false, Ordering::Release);
}

fn install_in_progress(state: &AtomicBool) -> bool {
    state.load(Ordering::Acquire)
}

#[cfg(test)]
mod tests {
    use crate::settings::mcp_helper_progress::McpHelperInstallProgressStatus;
    use anyhow::anyhow;
    use public_mcp::client_config::ClientConfigInstall;
    use rust_i18n::t;
    use std::sync::atomic::AtomicBool;

    #[test]
    fn helper_install_error_message_uses_display_chain_without_debug_output() {
        let error = anyhow!("network down").context("download failed");

        let message = super::format_install_error(error);

        assert!(message.contains("download failed"));
        assert!(message.contains("network down"));
        assert!(!message.contains("Error {"));
    }

    #[test]
    fn install_state_prevents_duplicate_starts_and_resets_after_finish() {
        let state = AtomicBool::new(false);

        assert!(super::try_begin_install(&state));
        assert!(!super::try_begin_install(&state));

        super::finish_install(&state);

        assert!(super::try_begin_install(&state));
    }

    #[cfg(unix)]
    #[test]
    fn helper_install_status_reports_unusable_helper_for_non_executable_file() {
        let dir = tempfile::tempdir().unwrap();
        let helper = dir.path().join("onetcli-public-mcp");
        std::fs::write(&helper, "").unwrap();
        let install = ClientConfigInstall::from_helper_path(&helper, "/tmp/discovery.json");

        let status = super::helper_install_status_for_install(false, Ok(install));

        assert_eq!(
            t!(
                "Settings.General.Mcp.helper_status_unusable",
                path = helper.display().to_string()
            )
            .to_string(),
            status
        );
    }

    #[test]
    fn helper_install_status_uses_progress_snapshot_while_installing() {
        let status = super::helper_install_progress_status_message(
            McpHelperInstallProgressStatus::Downloading { percent: Some(42) },
        );

        assert_eq!(
            t!(
                "Settings.General.Mcp.helper_status_downloading",
                percent = 42
            )
            .to_string(),
            status
        );
    }
}
