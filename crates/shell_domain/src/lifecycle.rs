//! 终端领域的应用生命周期：SSH 会话服务与远程桌面关停。
//!
//! `ssh` crate 不依赖 GPUI；这里安装唯一的应用级 `SshSessionService`，
//! 并在退出时按「先原生 RDP、后 SSH 传输」的顺序做有界关停。

use gpui::{App, SharedString, Task};
use one_core::gpui_tokio::{JoinError, Tokio};
use ssh::{SshSessionService, SshSessionShutdownReport};

/// 应用启动到退出期间唯一的 SSH 会话服务持有者。
#[derive(Clone)]
pub(crate) struct GlobalSshSessionService {
    service: SshSessionService,
}

impl gpui::Global for GlobalSshSessionService {}

impl GlobalSshSessionService {
    fn new() -> Self {
        Self {
            service: SshSessionService::new(),
        }
    }

    fn service(&self) -> SshSessionService {
        self.service.clone()
    }
}

/// 安装 SSH 会话服务，并注册平台退出路径的兜底关停回调。
pub(crate) fn init(cx: &mut App) {
    assert!(
        cx.try_global::<GlobalSshSessionService>().is_none(),
        "SSH session service must have exactly one application owner"
    );

    let global = GlobalSshSessionService::new();
    let fallback_service = global.service();
    cx.set_global(global);

    // 正常退出路径会先 await 关停再退出；这里只是平台强制退出的幂等兜底。
    // GPUI 给退出回调的时间很短，因此它不能承担主要关停职责。
    cx.on_app_quit(move |cx| {
        let rdp_shutdown_report =
            remote_desktop_view::fail_closed_windows_native_rdp_for_platform_quit(cx);
        log_windows_native_rdp_shutdown("gpui quit fallback", rdp_shutdown_report);

        let service = fallback_service.clone();
        let shutdown_task = Tokio::spawn(cx, async move { service.shutdown().await });
        async move {
            log_ssh_session_shutdown("gpui quit fallback", shutdown_task.await);
        }
    })
    .detach();
}

/// 有界关停领域资源：先排空原生 RDP 宿主，再关停 SSH 传输。
pub(crate) fn shutdown(cx: &mut App, reason: SharedString) -> Task<()> {
    let rdp_shutdown_task = remote_desktop_view::shutdown_windows_native_rdp(cx);

    cx.spawn(async move |cx| {
        let report = rdp_shutdown_task.await;
        log_windows_native_rdp_shutdown(reason.as_ref(), report);

        let ssh_shutdown_task = cx.update(spawn_ssh_session_shutdown);
        match ssh_shutdown_task {
            Some(task) => log_ssh_session_shutdown(reason.as_ref(), task.await),
            None => tracing::error!(
                reason = %reason,
                "SSH session service global is missing; quitting after remaining application teardown"
            ),
        }
    })
}

fn spawn_ssh_session_shutdown(
    cx: &mut App,
) -> Option<Task<Result<SshSessionShutdownReport, JoinError>>> {
    let service = cx.try_global::<GlobalSshSessionService>()?.service();
    Some(Tokio::spawn(cx, async move { service.shutdown().await }))
}

fn log_ssh_session_shutdown(reason: &str, result: Result<SshSessionShutdownReport, JoinError>) {
    match result {
        Ok(report)
            if report.timed_out
                || report.manager_failures > 0
                || report.managers_remaining > 0
                || report.registry_tasks_remaining > 0 =>
        {
            tracing::warn!(
                reason,
                timed_out = report.timed_out,
                managers_requested = report.managers_requested,
                managers_completed = report.managers_completed,
                manager_failures = report.manager_failures,
                managers_remaining = report.managers_remaining,
                registry_tasks_remaining = report.registry_tasks_remaining,
                "SSH session service shutdown completed with incomplete cleanup"
            );
        }
        Ok(report) => {
            tracing::info!(
                reason,
                managers_requested = report.managers_requested,
                managers_completed = report.managers_completed,
                "SSH session service shutdown completed"
            );
        }
        Err(error) => {
            tracing::error!(
                reason,
                %error,
                "SSH session service shutdown task failed"
            );
        }
    }
}

fn log_windows_native_rdp_shutdown(
    reason: &str,
    report: remote_desktop_view::WindowsNativeRdpShutdownReport,
) {
    if report.incomplete() {
        tracing::warn!(
            reason,
            requested = report.requested(),
            destroyed = report.destroyed(),
            timed_out_leaked = report.timed_out_leaked(),
            owner_lost = report.owner_lost(),
            controller_unavailable = report.controller_unavailable(),
            "Windows native RDP shutdown completed with incomplete cleanup"
        );
    } else {
        tracing::info!(
            reason,
            requested = report.requested(),
            destroyed = report.destroyed(),
            "Windows native RDP shutdown completed"
        );
    }
}

#[cfg(test)]
mod tests {
    /// 平台退出兜底必须「先 fail-closed 原生 RDP、后 SSH」，并且不能递归退出：
    /// 它只是幂等兜底，主关停路径由 `shutdown` 承担。
    #[test]
    fn platform_quit_fails_closed_native_rdp_before_ssh_without_recursive_quit() {
        let source = include_str!("lifecycle.rs").replace("\r\n", "\n");
        let callback_start = source
            .find("cx.on_app_quit(move |cx|")
            .expect("platform quit callback");
        let callback_end = source[callback_start..]
            .find("\n}\n\n/// 有界关停领域资源")
            .map(|offset| callback_start + offset)
            .expect("platform quit callback end");
        let callback = &source[callback_start..callback_end];
        let fail_closed_rdp = callback
            .find("remote_desktop_view::fail_closed_windows_native_rdp_for_platform_quit(cx)")
            .expect("Native RDP platform-quit fail-closed fallback");
        let spawn_ssh = callback
            .find("Tokio::spawn(cx")
            .expect("SSH platform-quit fallback");

        assert!(fail_closed_rdp < spawn_ssh);
        assert!(callback.contains("log_windows_native_rdp_shutdown"));
        assert!(!callback.contains("shutdown_windows_native_rdp(cx)"));
        assert!(!callback.contains("cx.quit()"));
    }

    /// 主关停路径必须先 await 原生 RDP 排空，再关停 SSH 传输。
    #[test]
    fn shutdown_order_is_rdp_before_ssh() {
        let source = include_str!("lifecycle.rs").replace("\r\n", "\n");
        let start = source.find("pub(crate) fn shutdown").expect("shutdown fn");
        let body = &source[start..];
        let rdp = body.find("rdp_shutdown_task.await").expect("rdp await");
        let ssh = body.find("spawn_ssh_session_shutdown").expect("ssh spawn");
        assert!(rdp < ssh);
    }
}
