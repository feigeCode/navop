//! 系统托盘：把平台托盘回调安全桥接进 GPUI 主线程。
//!
//! 分工是刻意的：
//!
//! - 平台回调（AppKit / Win32 消息线程 / ksni D-Bus 线程）只能往 `smol::channel`
//!   里塞一个 [`TrayCommand`]，**不许**碰任何 GPUI entity 或 `Window`；
//! - 一个 GPUI 前台任务消费命令，在 `AsyncApp` 上下文里操作已有主窗口；
//! - 托盘是否可用 → 由纯函数 [`main_window_close_action`] 决定关闭按钮行为；
//!   托盘不可用时行为与改造前完全一致（走既有退出确认）。
//!
//! 三个已经踩实、不能再踩的坑（都有源码守卫测试盯着）：
//!
//! 1. `MenuEvent::receiver()` / `TrayIconEvent::receiver()` 与
//!    `set_event_handler(Some(_))` **互斥**——设了 handler 之后就再也不会往 channel
//!    发事件，只能二选一，不能混用。
//! 2. `TrayIcon` 内部是 `Rc<RefCell<..>>`，**全平台 `!Send`**，只能放 `thread_local!`；
//!    而且 drop 掉最后一个实例图标会立刻消失（「图标闪一下」就是这个）。
//! 3. GPUI 前台任务就是主线程上的 runnable ⇒ 绝不能在 `cx.spawn` 里阻塞等待。
//!    `recv().await` 是让出线程的异步等待，可以；`MenuEvent::receiver().recv()`
//!    那种阻塞写法会把主线程连同「必须由它派发菜单事件」的那条线程一起堵死。
//! 4. 原生可见性/激活修改必须留在 GPUI 窗口借用**之外**（见 [`restore_main_window`]
//!    与 `window_visibility` 的模块头注释）：AppKit 会同步回调 `gpui::window` 里的
//!    `handle.update(...)`，在借用内改原生状态必定二次借用失败。

use anyhow::Context as _;
use gpui::{AnyWindowHandle, App, AppContext as _, AsyncApp};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(debug_assertions)]
use std::time::Duration;

/// 托盘回调允许产出的命令。刻意保持最小集合。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TrayCommand {
    ShowMainWindow,
    QuitApplication,
}

/// 主窗口关闭按钮的行为。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MainWindowCloseAction {
    /// 托盘可用：隐藏窗口，保留进程、标签页和后台任务。
    HideToTray,
    /// 托盘不可用（或初始化/隐藏失败）：走既有退出确认。
    RequestQuit,
}

/// 托盘菜单项 id。
pub(crate) const MENU_ID_SHOW: &str = "navop.tray.show";
pub(crate) const MENU_ID_QUIT: &str = "navop.tray.quit";

/// 关闭按钮的纯策略：只有托盘确实可用时才隐藏窗口，否则必须回退到退出确认，
/// 不能制造一个无法恢复的隐藏窗口。
pub(crate) const fn main_window_close_action(tray_ready: bool) -> MainWindowCloseAction {
    if tray_ready {
        MainWindowCloseAction::HideToTray
    } else {
        MainWindowCloseAction::RequestQuit
    }
}

/// 菜单项 id → 命令的纯映射。未知 id（例如未来新增菜单项）一律忽略。
pub(crate) fn command_for_menu_id(id: &str) -> Option<TrayCommand> {
    match id {
        MENU_ID_SHOW => Some(TrayCommand::ShowMainWindow),
        MENU_ID_QUIT => Some(TrayCommand::QuitApplication),
        _ => None,
    }
}

/// 托盘图标点击 → 命令的纯映射。
///
/// 只认**左键抬起**：按下事件会先到，按下就显示会导致「右键弹出菜单时窗口被顺手
/// 拉到前台」；左键抬起既覆盖单击也覆盖双击场景。
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub(crate) fn command_for_tray_click(
    button: tray_icon::MouseButton,
    state: tray_icon::MouseButtonState,
) -> Option<TrayCommand> {
    matches!(
        (button, state),
        (
            tray_icon::MouseButton::Left,
            tray_icon::MouseButtonState::Up
        )
    )
    .then_some(TrayCommand::ShowMainWindow)
}

/// 初始化系统托盘。返回托盘是否可用。重复调用返回已有状态。
///
/// 必须在「平台事件循环已经在跑」的主线程上调用（macOS 的 `NSStatusItem` 与
/// ksni 的 D-Bus service 都要求如此）。GPUI 的 `Application::run` 闭包满足该前提：
/// `on_finish_launching` 已经执行过 `did_finish_launching`。
pub(crate) fn init(cx: &mut App) -> bool {
    if INITIALIZED.set(()).is_err() {
        return is_available();
    }

    install_dispatcher(cx);

    match desktop::install() {
        Ok(()) => {
            TRAY_READY.store(true, Ordering::SeqCst);
            tracing::info!("系统托盘已就绪");
        }
        Err(error) => {
            // 托盘不可用不是致命错误：关闭按钮继续走既有退出确认。
            tracing::warn!(%error, "系统托盘初始化失败，关闭按钮继续走退出确认");
        }
    }

    #[cfg(debug_assertions)]
    if std::env::var_os("NAVOP_TRAY_SMOKE").is_some() {
        install_smoke_hook(cx);
    }

    is_available()
}

/// 托盘是否可用。只在初始化完成后为真；初始化失败或平台不支持时恒为假。
pub(crate) fn is_available() -> bool {
    TRAY_READY.load(Ordering::SeqCst)
}

static COMMAND_TX: OnceLock<smol::channel::Sender<TrayCommand>> = OnceLock::new();
static TRAY_READY: AtomicBool = AtomicBool::new(false);
static INITIALIZED: OnceLock<()> = OnceLock::new();

/// 安装唯一的前台消费任务。
///
/// 发送端常驻在 `COMMAND_TX` 里，所以这个循环与进程同寿；通道真正断开只可能发生在
/// 进程收尾阶段，此时记一条日志退出即可（不会形成无界错误循环）。
fn install_dispatcher(cx: &mut App) {
    let (tx, rx) = smol::channel::unbounded();
    if COMMAND_TX.set(tx).is_err() {
        return;
    }

    cx.spawn(async move |cx: &mut AsyncApp| {
        let mut cx = cx.clone();
        while let Ok(command) = rx.recv().await {
            if let Err(error) = execute_command(command, &mut cx) {
                tracing::warn!(%error, ?command, "托盘命令执行失败");
            }
        }
        tracing::debug!("托盘命令通道已关闭，停止消费");
    })
    .detach();
}

/// 平台回调侧唯一的出口：只入队，不执行。
fn dispatch(command: TrayCommand) {
    let Some(tx) = COMMAND_TX.get() else {
        tracing::warn!(?command, "托盘命令通道尚未安装，丢弃命令");
        return;
    };
    if let Err(error) = tx.try_send(command) {
        tracing::warn!(%error, ?command, "托盘命令入队失败");
    }
}

fn execute_command(command: TrayCommand, cx: &mut AsyncApp) -> anyhow::Result<()> {
    tracing::info!(?command, "执行托盘命令");

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
        TrayCommand::ShowMainWindow => {
            restore_main_window(handle, cx)?;
        }
        TrayCommand::QuitApplication => {
            // 退出确认必须落在用户看得见的窗口上。恢复失败也继续往下走：用户已经
            // 明确选择退出，卡在托盘上不响应比「确认框在一个看不见的窗口里」更糟。
            if let Err(error) = restore_main_window(handle, cx) {
                tracing::warn!(%error, "退出前恢复主窗口失败，仍继续走退出确认");
            }
            cx.update_window(handle, |_, window, cx| {
                // 不直接终止进程：走既有退出入口（`request_window_quit` → 确认 →
                // `close_all_tabs`），唯一的直接退出例外是上面 handle 失效的分支。
                crate::onetcli_app::request_window_quit(window, cx);
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
/// `window.activate_window()` 可以留在借用内：GPUI 平台层把 `makeKeyAndOrderFront`
/// 投递到前台执行器执行，本身就不在借用内。
fn restore_main_window(handle: AnyWindowHandle, cx: &mut AsyncApp) -> anyhow::Result<()> {
    let target = cx
        .update_window(handle, |_, window, _| {
            window.activate_window();
            crate::window_visibility::main_window_target(window)
        })
        .context("读取主窗口原生句柄失败")??;

    crate::window_visibility::show_main_window(target)?;
    tracing::info!("主窗口已恢复显示");
    Ok(())
}

/// 无人值守冒烟钩子（仅 debug 构建 + `NAVOP_TRAY_SMOKE=1` 时安装）。
///
/// 托盘的两条关键路径都只能由真实点击触发，本机/CI 没有任何自动化入口，而「借用重入」
/// 这类缺陷恰恰只在「窗口已经隐藏再恢复」这一个状态迁移上出现。打开这个开关后进程会在
/// 启动几秒后自动跑一遍「隐藏 → 托盘恢复 → 退出」，结果直接进日志：
/// 只要没有 `RefCell already borrowed`、且看到「主窗口已恢复显示」，说明恢复路径干净。
#[cfg(debug_assertions)]
fn install_smoke_hook(cx: &mut App) {
    // 独立函数而不是闭包：闭包会把 `cx` 借满整个 async 块，后面 `cx.update_window`
    // 就没法可变借用了（E0502）。
    async fn sleep(cx: &AsyncApp, ms: u64) {
        cx.background_executor()
            .timer(Duration::from_millis(ms))
            .await;
    }

    cx.spawn(move |cx: &mut AsyncApp| {
        let mut cx = cx.clone();
        async move {
            sleep(&cx, 3_000).await;
            let handle = cx.update(|_| crate::app_init::main_window_handle());
            let Some(handle) = handle else {
                tracing::error!("托盘冒烟：主窗口句柄不可用");
                return;
            };

            // 隐藏走与关闭按钮同一段原生代码，只是从 GPUI 借用外调用。
            match cx.update_window(handle, |_, window, _| {
                crate::window_visibility::hide_main_window(window)
            }) {
                Ok(Ok(())) => tracing::info!("托盘冒烟：主窗口已隐藏"),
                other => tracing::warn!(?other, "托盘冒烟：隐藏主窗口失败"),
            }

            sleep(&cx, 1_200).await;
            // 恢复走完整生产路径：托盘命令 → 前台消费任务 → `execute_command`。
            dispatch(TrayCommand::ShowMainWindow);

            sleep(&cx, 1_500).await;
            tracing::info!("托盘冒烟：隐藏→恢复已跑完，可以结束进程了");
        }
    })
    .detach();
}

/// 只认 `app_init` 注册的主窗口，不用 active/window_stack 兜底：
/// 托盘恢复必须是幂等的、且不能作用到辅助窗口上。
fn resolve_main_window(cx: &AsyncApp) -> Option<AnyWindowHandle> {
    cx.update(|_| crate::app_init::main_window_handle())
}

/// 托盘图标边长。源资源是 1024×1024 的应用图标，直接交给平台会被按菜单栏/通知
/// 区域高度做一次质量不可控的缩放，所以这里先缩到常见托盘尺寸。
const TRAY_ICON_EDGE: u32 = 32;

/// 内嵌品牌图标：不依赖运行目录或安装包里的相对路径。
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
const TRAY_ICON_PNG: &[u8] = include_bytes!("../../resources/navop-icon.png");

/// PNG 字节 → `(RGBA, width, height)`。
///
/// 纯函数，单测覆盖通道数与尺寸；解码失败视为托盘初始化失败——宁可不显示托盘，
/// 也不做一个没有图标、用户找不到的隐形入口。
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn decode_tray_icon(png: &[u8]) -> anyhow::Result<(Vec<u8>, u32, u32)> {
    let decoded = image::load_from_memory(png)
        .context("托盘图标 PNG 解码失败")?
        .into_rgba8();
    let (width, height) = decoded.dimensions();

    let scaled = if width > TRAY_ICON_EDGE || height > TRAY_ICON_EDGE {
        let ratio = f64::from(TRAY_ICON_EDGE) / f64::from(width.max(height));
        let target_width = ((f64::from(width) * ratio).round() as u32).max(1);
        let target_height = ((f64::from(height) * ratio).round() as u32).max(1);
        image::imageops::resize(
            &decoded,
            target_width,
            target_height,
            image::imageops::FilterType::Lanczos3,
        )
    } else {
        decoded
    };

    let (width, height) = scaled.dimensions();
    Ok((scaled.into_raw(), width, height))
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
mod desktop {
    use super::*;
    use std::cell::RefCell;
    use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
    use tray_icon::{TrayIconBuilder, TrayIconEvent};

    thread_local! {
        /// 托盘句柄必须留在**创建它的线程**上：`TrayIcon` 内部是
        /// `Rc<RefCell<..>>`（全平台 `!Send`），而且 drop 掉最后一个实例图标会
        /// 立刻从托盘区消失。这就是「图标闪一下就没了」的全部原因。
        static TRAY_ICON: RefCell<Option<tray_icon::TrayIcon>> = const { RefCell::new(None) };
    }

    pub(super) fn install() -> anyhow::Result<()> {
        if TRAY_ICON.with(|slot| slot.borrow().is_some()) {
            return Ok(());
        }

        // 与 `receiver()` 互斥：一旦设置 handler，channel 就永远收不到事件。
        MenuEvent::set_event_handler(Some(handle_menu_event));
        TrayIconEvent::set_event_handler(Some(handle_tray_event));

        let (rgba, width, height) = decode_tray_icon(TRAY_ICON_PNG)?;
        let icon = tray_icon::Icon::from_rgba(rgba, width, height)?;

        let show = MenuItem::with_id(MENU_ID_SHOW, "显示 Navop", true, None);
        let quit = MenuItem::with_id(MENU_ID_QUIT, "退出 Navop", true, None);
        let separator = PredefinedMenuItem::separator();
        let menu = Menu::new();
        menu.append(&show)?;
        menu.append(&separator)?;
        menu.append(&quit)?;

        let tray = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_icon(icon)
            .with_tooltip("Navop")
            // 左键留给「显示主窗口」，右键继续打开菜单。
            .with_menu_on_left_click(false)
            .with_menu_on_right_click(true)
            .build()?;

        TRAY_ICON.with(|slot| *slot.borrow_mut() = Some(tray));
        Ok(())
    }

    fn handle_menu_event(event: MenuEvent) {
        if let Some(command) = command_for_menu_id(event.id().as_ref()) {
            dispatch(command);
        }
    }

    fn handle_tray_event(event: TrayIconEvent) {
        let TrayIconEvent::Click {
            button,
            button_state,
            ..
        } = event
        else {
            return;
        };
        if let Some(command) = command_for_tray_click(button, button_state) {
            dispatch(command);
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
mod desktop {
    pub(super) fn install() -> anyhow::Result<()> {
        anyhow::bail!("当前平台未实现系统托盘")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_action_hides_only_when_the_tray_is_ready() {
        assert_eq!(
            MainWindowCloseAction::HideToTray,
            main_window_close_action(true)
        );
        assert_eq!(
            MainWindowCloseAction::RequestQuit,
            main_window_close_action(false)
        );
    }

    #[test]
    fn menu_ids_map_to_the_minimal_command_set() {
        assert_eq!(
            Some(TrayCommand::ShowMainWindow),
            command_for_menu_id(MENU_ID_SHOW)
        );
        assert_eq!(
            Some(TrayCommand::QuitApplication),
            command_for_menu_id(MENU_ID_QUIT)
        );
        assert_eq!(None, command_for_menu_id("navop.tray.unknown"));
        assert_eq!(None, command_for_menu_id(""));
    }

    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    #[test]
    fn tray_icon_click_shows_the_main_window_only_on_left_button_release() {
        use tray_icon::{MouseButton, MouseButtonState};

        assert_eq!(
            Some(TrayCommand::ShowMainWindow),
            command_for_tray_click(MouseButton::Left, MouseButtonState::Up)
        );
        assert_eq!(
            None,
            command_for_tray_click(MouseButton::Left, MouseButtonState::Down)
        );
        assert_eq!(
            None,
            command_for_tray_click(MouseButton::Right, MouseButtonState::Up)
        );
        assert_eq!(
            None,
            command_for_tray_click(MouseButton::Middle, MouseButtonState::Up)
        );
    }

    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    #[test]
    fn embedded_brand_icon_decodes_to_a_square_rgba_sized_for_the_tray() {
        let (rgba, width, height) = decode_tray_icon(TRAY_ICON_PNG).expect("品牌图标必须可解码");

        assert_eq!(TRAY_ICON_EDGE, width);
        assert_eq!(TRAY_ICON_EDGE, height);
        assert_eq!((width * height * 4) as usize, rgba.len());
        assert!(
            rgba.chunks_exact(4).any(|pixel| pixel[3] > 0),
            "缩放后的托盘图标不能是全透明"
        );
    }

    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    #[test]
    fn decode_rejects_payloads_that_are_not_images() {
        assert!(decode_tray_icon(b"not a png").is_err());
        assert!(decode_tray_icon(&[]).is_err());
    }

    #[test]
    fn dispatcher_is_the_only_place_that_touches_the_command_channel() {
        let source = include_str!("system_tray.rs");
        let dispatch_fn = source
            .split("fn dispatch(command: TrayCommand)")
            .nth(1)
            .and_then(|source| source.split("\nfn execute_command").next())
            .expect("dispatch source");

        // 平台回调只允许入队；真正的 GPUI 操作必须在 execute_command 里。
        assert!(dispatch_fn.contains("COMMAND_TX"));
        assert!(!dispatch_fn.contains("update_window"));
        assert!(!dispatch_fn.contains("Window"));
    }

    #[test]
    fn tray_quit_reuses_the_existing_quit_request_instead_of_quitting_directly() {
        let source = include_str!("system_tray.rs");
        let execute_fn = source
            .split("fn execute_command(command: TrayCommand")
            .nth(1)
            .and_then(|source| source.split("\n/// 恢复主窗口，拆成").next())
            .expect("execute_command source");
        let quit_arm = execute_fn
            .split("TrayCommand::QuitApplication => {")
            .nth(1)
            .and_then(|rest| rest.split("\n    }\n\n    Ok(())").next())
            .expect("QuitApplication arm");
        // needle 运行时拼接，避免断言字面量把 include_str! 守卫自己命中。
        let direct_quit = ["cx", ".quit()"].concat();

        assert!(quit_arm.contains("request_window_quit"));
        assert!(quit_arm.contains("restore_main_window"));
        assert!(
            !quit_arm.contains(&direct_quit),
            "托盘退出必须先恢复主窗口，再走既有确认与标签页关闭流程"
        );

        // 直接退出的唯一例外是「主窗口 handle 已失效」这条异常路径，
        // 正常托盘退出路径不得依赖它。
        assert_eq!(1, source.matches(&direct_quit).count());
    }

    /// 回归守卫：原生 order-front / activate 必须在 GPUI 窗口借用**之外**执行。
    ///
    /// 2026-09-17 真机实测：把 `makeKeyAndOrderFront:` + `NSApplication::activate` 放在
    /// `cx.update_window(...)` 的借用里，AppKit 会同步回调 `gpui/src/window.rs` 的
    /// `on_active_status_change` → `handle.update(...)`，抢同一个 App 借用失败，
    /// 日志只留下 `ERROR gpui::window: RefCell already borrowed`（窗口已经隐藏再恢复时才出现）。
    #[test]
    fn native_restore_runs_outside_the_gpui_window_borrow() {
        let source = include_str!("system_tray.rs");
        let restore_fn = source
            .split("fn restore_main_window(")
            .nth(1)
            .and_then(|rest| rest.split("/// 无人值守冒烟钩子").next())
            .expect("restore_main_window source");

        // 借用内只做两件安全的事：取原生句柄 + GPUI 自己的延迟激活。
        let borrowed = restore_fn
            .split(".update_window(handle,")
            .nth(1)
            .and_then(|rest| rest.split("})").next())
            .expect("update_window closure");
        assert!(borrowed.contains("main_window_target"));
        assert!(borrowed.contains("activate_window"));
        assert!(
            !borrowed.contains("show_main_window"),
            "原生恢复不能在 GPUI 窗口借用内调用（会触发 AppKit 回调重入 → RefCell already borrowed）"
        );

        // 借用释放之后才允许改原生状态。
        assert!(restore_fn.contains("show_main_window(target)"));
    }

    #[test]
    fn tray_icon_is_kept_alive_on_the_creating_thread() {
        let source = include_str!("system_tray.rs");

        // TrayIcon 是 Rc<RefCell<..>>（!Send），只能线程本地持有，且必须真的存住。
        assert!(source.contains("thread_local!"));
        assert!(source.contains("static TRAY_ICON: RefCell<Option<tray_icon::TrayIcon>>"));
        assert!(source.contains("*slot.borrow_mut() = Some(tray)"));
    }

    #[test]
    fn tray_backends_avoid_the_gtk_and_appindicator_linux_dependencies() {
        let manifest = include_str!("../Cargo.toml");
        let linux_deps = manifest
            .split("[target.'cfg(target_os = \"linux\")'.dependencies]")
            .nth(1)
            .and_then(|rest| rest.split("\n[").next())
            .expect("linux target dependencies");
        // needle 运行时拼接：Cargo.toml 的注释里就写着这两个 crate 名。
        let appindicator_feature = ["libapp", "indicator"].concat();
        let xdo_feature = ["lib", "xdo"].concat();

        assert!(linux_deps.contains("tray-icon"));
        assert!(linux_deps.contains("features = [\"ksni\"]"));
        assert!(linux_deps.contains("default-features = false"));
        assert!(!linux_deps.contains(&appindicator_feature));
        assert!(!linux_deps.contains(&xdo_feature));
    }
}
