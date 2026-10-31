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
//! 宿主差异通过 [`TrayHost`] 注入：全量产品注入自己的原生显隐与退出确认实现，
//! 独立产品使用本模块的通用默认实现。
//!
//! 五个已经踩实、不能再踩的坑（都有源码守卫测试盯着）：
//!
//! 1. `MenuEvent::receiver()` / `TrayIconEvent::receiver()` 与
//!    `set_event_handler(Some(_))` **互斥**——设了 handler 之后就再也不会往 channel
//!    发事件，只能二选一，不能混用。
//! 2. `TrayIcon` 内部是 `Rc<RefCell<..>>`，**全平台 `!Send`**，只能放 `thread_local!`；
//!    而且 drop 掉最后一个实例图标会立刻消失（「图标闪一下」就是这个）。
//! 3. GPUI 前台任务就是主线程上的 runnable ⇒ 绝不能在 `cx.spawn` 里阻塞等待。
//!    `recv().await` 是让出线程的异步等待，可以；`MenuEvent::receiver().recv()`
//!    那种阻塞写法会把主线程连同「必须由它派发菜单事件」的那条线程一起堵死。
//! 4. 原生可见性/激活修改必须留在 GPUI 窗口借用**之外**（见 [`restore_main_window`]）：
//!    AppKit 会同步回调进 GPUI，在借用内改原生状态必定二次借用失败。
//! 5. `TRAY_ICON` 这个 `thread_local!` 必须在退出前**主动清空**（见 [`shutdown`]），
//!    不能留给进程退出的 TLS 析构链：Linux(ksni) 的 `TrayIcon::drop` 会回到 async-io，
//!    而 async-io 的线程本地是在托盘安装过程中才注册的，退出时反而先被销毁 ——
//!    析构器撞上已销毁的 TLS 只能 panic → abort（issue #336）。

use gpui::{AnyWindowHandle, App, AsyncApp, Window};
use one_core::tab_container::GlobalTabContainer;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

mod commands;
mod desktop;
mod menu;
#[cfg(debug_assertions)]
mod smoke;
#[cfg(test)]
mod tests;

use commands::execute_command;
pub use menu::main_window_close_action;

/// 托盘回调允许产出的命令。刻意保持最小集合。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrayCommand {
    ShowMainWindow,
    /// 激活指定 id 的已打开标签（托盘菜单里的「会话」项）。
    ActivateSession {
        id: String,
    },
    /// 打开新建连接向导（只有 `TrayConfig::show_quick_actions` 时才会出现在菜单里）。
    NewConnection,
    /// 打开设置页（只有 `TrayConfig::show_quick_actions` 时才会出现在菜单里）。
    OpenSettings,
    QuitApplication,
}

/// 主窗口关闭按钮的行为。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MainWindowCloseAction {
    /// 托盘可用但用户还没固化偏好：弹窗让用户当场选「最小化到托盘 / 退出应用」。
    AskUser,
    /// 用户已选择最小化到托盘：隐藏窗口，保留进程、标签页和后台任务。
    HideToTray,
    /// 托盘不可用（或用户已选择退出）：走既有退出确认。
    RequestQuit,
}

/// 宿主注入的平台相关回调。没注入的回调走本模块的通用实现。
///
/// 全量产品的「显示主窗口」必须自己实现原生 order-front（见
/// `crates/navop_app/src/window_visibility.rs`），所以这里给的是一个**完整接管**的
/// 异步回调，而不是一个可选装饰步骤。
#[derive(Clone, Copy, Default)]
pub struct TrayHost {
    /// 主窗口句柄。默认取窗口栈里的第一个窗口。
    pub main_window: Option<fn() -> Option<AnyWindowHandle>>,
    /// 把主窗口带回前台。默认只做 GPUI 自己的 `activate_window`。
    pub show_main_window: Option<fn(AnyWindowHandle, &mut AsyncApp) -> anyhow::Result<()>>,
    /// 把主窗口藏起来（冒烟钩子用）。默认不支持。
    pub hide_main_window: Option<fn(&mut Window) -> anyhow::Result<()>>,
    /// 按 id 激活标签。默认读 [`GlobalTabContainer`] 并 `set_active_index`。
    pub activate_tab: Option<fn(&str, &mut Window, &mut App)>,
    /// 退出确认入口。默认走 `one_core::window_close::request_close_window`。
    pub request_quit: Option<fn(AnyWindowHandle, &mut Window, &mut App)>,
}

/// 产品自己的托盘外观与菜单开关。
pub struct TrayConfig {
    /// 菜单文案里的产品名（「显示 Navop」「退出 Navop」）。
    pub display_name: String,
    /// 托盘悬停提示。
    pub tooltip: String,
    /// 托盘图标 PNG 字节。
    pub icon_png: &'static [u8],
    /// 是否在菜单里放「新建连接 / 打开设置」。全量产品关掉以保持原有菜单。
    pub show_quick_actions: bool,
    /// 右下角角标颜色（RGBA）。独立产品用它和全量产品区分托盘图标。
    pub badge: Option<[u8; 4]>,
}

impl TrayConfig {
    /// 用产品展示名与内置品牌图标构造配置。
    pub fn new(display_name: impl Into<String>) -> Self {
        let display_name = display_name.into();
        Self {
            tooltip: display_name.clone(),
            display_name,
            icon_png: brand_icon_png(),
            show_quick_actions: true,
            badge: None,
        }
    }

    pub fn with_tooltip(mut self, tooltip: impl Into<String>) -> Self {
        self.tooltip = tooltip.into();
        self
    }

    pub fn with_icon_png(mut self, icon_png: &'static [u8]) -> Self {
        self.icon_png = icon_png;
        self
    }

    pub fn show_quick_actions(mut self, show: bool) -> Self {
        self.show_quick_actions = show;
        self
    }

    /// 给托盘图标加一个右下角角标，用于区分同机运行的多个产品。
    pub fn with_badge(mut self, badge: Option<[u8; 4]>) -> Self {
        self.badge = badge;
        self
    }
}

/// 内嵌品牌图标：不依赖运行目录或安装包里的相对路径。
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
const fn brand_icon_png() -> &'static [u8] {
    include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../resources/navop-icon.png"
    ))
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
const fn brand_icon_png() -> &'static [u8] {
    &[]
}

static COMMAND_TX: OnceLock<smol::channel::Sender<TrayCommand>> = OnceLock::new();
static TRAY_READY: AtomicBool = AtomicBool::new(false);
static INITIALIZED: OnceLock<()> = OnceLock::new();
static HOST: OnceLock<TrayHost> = OnceLock::new();
static CONFIG: OnceLock<TrayConfig> = OnceLock::new();

fn host() -> &'static TrayHost {
    HOST.get_or_init(TrayHost::default)
}

pub(super) fn config() -> &'static TrayConfig {
    CONFIG.get_or_init(|| TrayConfig::new("Navop"))
}

/// 初始化系统托盘，返回托盘是否可用。重复调用返回已有状态。
///
/// 必须在「平台事件循环已经在跑」的主线程上调用（macOS 的 `NSStatusItem` 与
/// ksni 的 D-Bus service 都要求如此）。GPUI 的 `Application::run` 闭包满足该前提：
/// `on_finish_launching` 已经执行过 `did_finish_launching`。
pub fn install(host: TrayHost, config: TrayConfig, cx: &mut App) -> bool {
    if INITIALIZED.set(()).is_err() {
        return is_available();
    }

    let _ = HOST.set(host);
    let _ = CONFIG.set(config);
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
        smoke::install(cx);
    }

    is_available()
}

/// 托盘是否可用。只在初始化完成后为真；初始化失败或平台不支持时恒为假。
pub fn is_available() -> bool {
    TRAY_READY.load(Ordering::SeqCst)
}

/// 把当前打开的标签同步进托盘菜单的「会话」段。
///
/// 只该在 GPUI 主线程上调用：托盘句柄是线程本地的，菜单操作必须留在创建它的
/// 线程（macOS 的 AppKit 硬要求，Windows 的 `set_menu` 也只是给自己 hwnd 发消息）。
pub fn sync_sessions_from(cx: &mut App) {
    if !is_available() {
        return;
    }
    let Some(global) = cx.try_global::<GlobalTabContainer>() else {
        return;
    };
    let container = global.tab_container.clone();
    let sessions: Vec<(String, String)> = container
        .read(cx)
        .tabs()
        .iter()
        .map(|tab| (tab.id().to_string(), tab.title(cx).to_string()))
        .collect();

    desktop::sync_sessions(&sessions);
}

/// 退出前释放托盘句柄，让进程退出的 TLS 析构链上不再留下托盘析构器。
///
/// # 为什么必须显式释放
///
/// `TRAY_ICON` 是包着 `RefCell<Option<TrayIcon>>` 的 `thread_local!`，不主动清空，
/// 它的析构器就会由 libc 在 `exit()` 里沿 `__call_tls_dtors` 执行。问题出在 Linux 的
/// ksni 后端：`TrayIcon::drop` 要跑一次 `Handle::shutdown().wait()`，也就是在调用线程
/// 上跑 `async_io::block_on()`；而 `async_io::driver::block_on` 一进门就无条件访问
/// async-io 自己的线程本地 `driver::CACHE`。
///
/// 两个线程本地的注册顺序是确定的，而且是错的那一个：`TRAY_ICON` 在 [`install`] →
/// `desktop::install()` 的第一句 `TRAY_ICON.with(..)` 上注册，而 `CACHE` 要等到同一函数
/// 里的 `TrayIconBuilder::build()` 走 ksni `spawn()` 才注册 —— **`TRAY_ICON` 先、`CACHE` 后**。
/// TLS 析构逆序执行，于是退出时 `CACHE` 先被销毁，之后 `TRAY_ICON` 的析构器再去碰它，
/// 拿到的就是 `cannot access a Thread Local Storage value during or after destruction:
/// AccessError`；在 TLS 析构器里 panic 无法展开，只能 abort。外部表现正是 issue #336。
///
/// 这里阻塞主线程一次是可接受的：`sync_sessions` 早就在主线程上跑同一个 `block_on`，
/// 本函数只是把同样的调用提前到退出之前。重复调用是幂等的（句柄取走即为空）。
pub fn shutdown() {
    // 先掐掉可用性标记：句柄交出去之后，`sync_sessions_from` 之类的菜单操作必须立刻
    // 变成空操作，不能再去碰一个已经 drop 掉的句柄。
    TRAY_READY.store(false, Ordering::SeqCst);
    desktop::shutdown();
}

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
            let attempted = command.clone();
            if let Err(error) = execute_command(command, &mut cx) {
                tracing::warn!(%error, ?attempted, "托盘命令执行失败");
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
    // `TrayCommand` 不是 `Copy`（会话 id 是 `String`），失败日志要留一份副本。
    let attempted = command.clone();
    if let Err(error) = tx.try_send(command) {
        tracing::warn!(%error, ?attempted, "托盘命令入队失败");
    }
}
