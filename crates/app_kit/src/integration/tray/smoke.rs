//! 无人值守冒烟钩子（仅 debug 构建 + `NAVOP_TRAY_SMOKE=1` 时安装）。
//!
//! 托盘的两条关键路径都只能由真实点击触发，本机/CI 没有任何自动化入口，而「借用重入」
//! 这类缺陷恰恰只在「窗口已经隐藏再恢复」这一个状态迁移上出现。打开这个开关后进程会在
//! 启动几秒后自动跑一遍「隐藏 → 托盘恢复 → 退出」，结果直接进日志：
//! 只要没有 `RefCell already borrowed`、且（`RUST_LOG=debug` 下）看到「主窗口已恢复显示」，
//! 说明恢复路径干净。

use gpui::{App, AppContext as _, AsyncApp};
use std::time::Duration;

use super::{TrayCommand, dispatch, host};

pub(super) fn install(cx: &mut App) {
    cx.spawn(move |cx: &mut AsyncApp| {
        let mut cx = cx.clone();
        async move {
            run(&mut cx).await;
        }
    })
    .detach();
}

async fn run(cx: &mut AsyncApp) {
    sleep(cx, 3_000).await;
    let handle = cx.update(|_| host().main_window.and_then(|main_window| main_window()));
    let Some(handle) = handle else {
        tracing::error!("托盘冒烟：主窗口句柄不可用");
        return;
    };

    // 隐藏走与关闭按钮同一段原生代码，只是从 GPUI 借用外调用。
    match cx.update_window(handle, |_, window, _| {
        host().hide_main_window.map_or_else(
            || anyhow::bail!("未注入隐藏主窗口的实现"),
            |hide| hide(window),
        )
    }) {
        Ok(Ok(())) => tracing::info!("托盘冒烟：主窗口已隐藏"),
        other => tracing::warn!(?other, "托盘冒烟：隐藏主窗口失败"),
    }

    // 恢复走完整生产路径：托盘命令 → 前台消费任务 → `execute_command`。
    sleep(cx, 1_200).await;
    dispatch(TrayCommand::ShowMainWindow);

    sleep(cx, 1_500).await;
    tracing::info!("托盘冒烟：隐藏→恢复已跑完，可以结束进程了");
}

/// 独立函数而不是闭包：闭包会把 `cx` 借满整个 async 块，后面 `cx.update_window`
/// 就没法可变借用了（E0502）。
async fn sleep(cx: &AsyncApp, ms: u64) {
    cx.background_executor()
        .timer(Duration::from_millis(ms))
        .await;
}
