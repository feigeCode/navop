//! 托盘命令的执行侧：只在 GPUI 前台任务里运行，负责真正操作窗口与标签页。

use anyhow::Context as _;
use gpui::{AnyWindowHandle, App, AppContext as _, AsyncApp, Window};
use one_core::tab_container::GlobalTabContainer;

use super::{TrayCommand, host};

pub(super) fn execute_command(command: TrayCommand, cx: &mut AsyncApp) -> anyhow::Result<()> {
    tracing::debug!(?command, "执行托盘命令");

    let Some(handle) = resolve_main_window(cx) else {
        if command == TrayCommand::QuitApplication {
            // 主窗口 handle 失效说明「窗口始终被关闭 handler 保留」这条生命周期
            // invariant 已经被破坏。用户已经明确选择退出，不能让进程永久残留。
            tracing::error!("托盘退出时主窗口句柄已失效，直接退出进程");
            cx.update(|cx| cx.quit());
        }
        anyhow::bail!("主窗口句柄不可用，无法执行托盘命令");
    };

    match command {
        TrayCommand::ShowMainWindow => restore_main_window(handle, cx)?,
        TrayCommand::ActivateSession { id } => {
            // 会话在窗口不可见时也可能被选中，先恢复窗口再切标签。
            restore_main_window(handle, cx)?;
            // 用刚恢复的 handle，而不是 `cx.active_window()`：窗口刚从隐藏状态激活时
            // 激活请求还挂在 GPUI 前台执行器上，此刻 active_window 可能是 None。
            cx.update_window(handle, |_, window, cx| activate_tab(&id, window, cx))
                .context("托盘切换会话时无法操作主窗口")?;
        }
        TrayCommand::NewConnection => {
            restore_main_window(handle, cx)?;
            cx.update_window(handle, |_, window, cx| {
                crate::open_new_connection(window, cx);
            })
            .context("托盘新建连接时无法操作主窗口")?;
        }
        TrayCommand::OpenSettings => {
            restore_main_window(handle, cx)?;
            cx.update_window(handle, |_, window, cx| {
                crate::open_settings(window, cx);
            })
            .context("托盘打开设置时无法操作主窗口")?;
        }
        TrayCommand::QuitApplication => {
            // 退出确认必须落在用户看得见的窗口上。恢复失败也继续往下走：用户已经
            // 明确选择退出，卡在托盘上不响应比「确认框在一个看不见的窗口里」更糟。
            if let Err(error) = restore_main_window(handle, cx) {
                tracing::warn!(%error, "退出前恢复主窗口失败，仍继续走退出确认");
            }
            cx.update_window(handle, |_, window, cx| {
                request_window_quit(handle, window, cx);
            })
            .context("托盘退出时无法请求关闭主窗口")?;
        }
    }

    Ok(())
}

/// 恢复主窗口，拆成「借用内取句柄」+「借用外改原生状态」两步。
///
/// 原生 `makeKeyAndOrderFront:` / `NSApplication::activate` 会**同步**回调进 GPUI
/// （`on_active_status_change` → `handle.update(...)`）。在 `cx.update_window` 的借用里
/// 做这件事会让回调回头抢同一个 App 借用，只留下 `ERROR gpui::window: RefCell already
/// borrowed`；把原生调用挪到借用释放之后，回调重入时 App 已空闲（2026-09-17 真机实测）。
///
/// 所以真正的原生恢复由宿主的 [`super::TrayHost::show_main_window`] 接管 —— 它拿到的是
/// `&mut AsyncApp`，可以在自己的借用内取句柄、在借用外改原生状态。默认实现只做 GPUI
/// 自己的 `activate_window`（平台层把它投递到前台执行器，本身就不在借用内）。
fn restore_main_window(handle: AnyWindowHandle, cx: &mut AsyncApp) -> anyhow::Result<()> {
    if let Some(show) = host().show_main_window {
        return show(handle, cx);
    }
    cx.update_window(handle, |_, window, _| window.activate_window())
        .context("激活主窗口失败")?;
    tracing::debug!("主窗口已恢复显示");
    Ok(())
}

/// 按 id 激活标签；宿主没注入时读 [`GlobalTabContainer`]。
fn activate_tab(id: &str, window: &mut Window, cx: &mut App) {
    if let Some(activate) = host().activate_tab {
        activate(id, window, cx);
        return;
    }
    let Some(global) = cx.try_global::<GlobalTabContainer>() else {
        return;
    };
    let container = global.tab_container.clone();
    let index = container
        .read(cx)
        .tabs()
        .iter()
        .position(|tab| tab.id().as_ref() == id);
    if let Some(index) = index {
        container.update(cx, |container, cx| {
            container.set_active_index(index, window, cx);
        });
    }
}

/// 托盘「退出」；宿主没注入时走正常窗口关闭流程（触发 `on_window_should_close` 里的
/// 状态保存），单窗口产品在 `QuitMode::LastWindowClosed` 下随之退出进程。
///
/// 不直接终止进程：唯一的直接退出例外是上面 handle 失效的异常分支。
fn request_window_quit(handle: AnyWindowHandle, window: &mut Window, cx: &mut App) {
    if let Some(quit) = host().request_quit {
        quit(handle, window, cx);
        return;
    }
    let _ = window;
    one_core::window_close::request_close_window(handle, cx);
}

/// 只认宿主注册的主窗口，不用 active/window_stack 兜底：托盘恢复必须幂等，
/// 且不能作用到辅助窗口上。
fn resolve_main_window(cx: &AsyncApp) -> Option<AnyWindowHandle> {
    match host().main_window {
        Some(main_window) => main_window(),
        None => cx.update(|cx| cx.windows().first().copied()),
    }
}
