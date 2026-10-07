use gpui::{
    AnyView, AnyWindowHandle, App, AppContext, AsyncApp, Bounds, Context, InteractiveElement,
    IntoElement, KeyBinding, ParentElement, Render, SharedString, Size, StatefulInteractiveElement,
    Styled, WeakEntity, Window, WindowBounds, WindowId, WindowKind, WindowOptions, actions, div,
    prelude::FluentBuilder, px, size,
};
use gpui_component::{
    ActiveTheme, Root, TITLE_BAR_HEIGHT, TitleBar, WindowExt, notification::Notification,
    render_dialog_layer, render_notification_layer, render_sheet_layer, v_flex,
};
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

const FULLSCREEN_POPUP_CONTEXT: &str = "FullscreenPopupWindow";

/// 主窗口 handle 注册表。
///
/// 部分弹出窗口的调用方只持有 `cx: &mut App`、没有 `&mut Window`，而 GPUI 在
/// 这类上下文里 `cx.active_window()` 会返回 `None`。此时回退到主窗口 handle，
/// 让弹窗落在用户实际所在的屏幕（主窗口所在显示器）。
static MAIN_WINDOW_HANDLE: OnceLock<AnyWindowHandle> = OnceLock::new();

/// 由主程序在创建主窗口后注册主窗口 handle。
pub fn set_main_window_handle(handle: AnyWindowHandle) {
    let _ = MAIN_WINDOW_HANDLE.set(handle);
}

/// 读取已注册的主窗口 handle（未注册时返回 `None`）。
fn main_window_handle() -> Option<AnyWindowHandle> {
    MAIN_WINDOW_HANDLE.get().copied()
}

/// 「关闭后复用」的弹窗注册表：复用键 → 那个窗口。
///
/// 只有通过 [`open_reusable_popup_window`] 打开的弹窗才会登记。没有复用键的一次性弹窗
/// 登记在 [`PARKED_POPUPS`] 里，两者的关闭动作一致：**隐藏原生窗口，不销毁**。
///
/// 键是**可携带目标身份**的 `String`（如 `connection-form:ssh:42`），不是 `&'static str`：
/// 同类弹窗常常同时为不同目标各开一个（同时编辑两个连接、同时连两个远程桌面），
/// 只按「弹窗种类」复用会把后开的窗口顶掉先开的那个。键与窗口一一对应，所以复用的粒度
/// 是「同一类弹窗的同一个目标」。
static REUSABLE_POPUPS: OnceLock<Mutex<HashMap<String, ReusablePopup>>> = OnceLock::new();

/// 一次性弹窗的登记表：窗口 id → 它的内容实体。
///
/// 一次性弹窗没有复用键，但**同样要「关闭即隐藏」**（否则 AppKit 会在自己的关闭流程里
/// 销毁窗口，红点那条路还是会闪退，见 navop#308 / navop#314）。要隐藏它就得在关闭时按窗口
/// 找到内容实体、把业务 view 卸掉，所以也得登记。
///
/// 代价说清楚：这类窗口隐藏后**没人会重新显示它**，下一次打开同一个对话框是**新建**一个
/// 窗口。也就是说停放的窗口数会随打开次数增长、直到应用退出。它换来的是「绝不销毁原生
/// 窗口」，而每个停放的窗口只留一个空壳（业务 view 在关闭时已经卸载）。要把数量真正压下去，
/// 得把热点弹窗改成 [`open_reusable_popup_window`]，见
/// `docs/macos-memory-investigation.md` §10.7。
static PARKED_POPUPS: OnceLock<Mutex<HashMap<WindowId, WeakEntity<PopupWindowContent>>>> =
    OnceLock::new();

/// 注册表里的一项。
///
/// `content` 是复用能正确工作的关键：重新显示时可以**换掉窗口里的 view**。
/// 换 view 用的是**本次调用**传入的 factory，所以「同一个导出窗口换了一张表」「同一个
/// 更新窗口来了个新版本」这类情况不会残留上一次的内容 —— 复用窗口不等于复用旧数据。
struct ReusablePopup {
    handle: AnyWindowHandle,
    content: WeakEntity<PopupWindowContent>,
}

fn reusable_popups() -> &'static Mutex<HashMap<String, ReusablePopup>> {
    REUSABLE_POPUPS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn parked_popups() -> &'static Mutex<HashMap<WindowId, WeakEntity<PopupWindowContent>>> {
    PARKED_POPUPS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn record_reusable_popup(
    key: String,
    handle: AnyWindowHandle,
    content: WeakEntity<PopupWindowContent>,
) {
    if let Ok(mut slots) = reusable_popups().lock() {
        // 同一个键重新登记只是刷新条目，不是新增窗口 —— 计数只在真的插入时增长，
        // 否则「反复开关是否在新建窗口」就无法从计数上判断。
        let is_new_window = slots
            .insert(key, ReusablePopup { handle, content })
            .is_none();
        drop(slots);
        if is_new_window {
            crate::popup_lifecycle::record_window_registered(
                crate::popup_lifecycle::PopupWindowKind::Reusable,
            );
            crate::popup_lifecycle::log_lifecycle("popup_window_registered");
        }
    }
}

fn forget_reusable_popup(key: &str) {
    if let Ok(mut slots) = reusable_popups().lock() {
        // 只有真的移除了条目才归还计数：窗口销毁与探活失败可能同时走到这里，
        // 重复扣减会让 `live_windows` 变成负数。
        let removed = slots.remove(key).is_some();
        drop(slots);
        if removed {
            crate::popup_lifecycle::record_window_unregistered(
                crate::popup_lifecycle::PopupWindowKind::Reusable,
            );
            crate::popup_lifecycle::log_lifecycle("popup_window_unregistered");
        }
    }
}

/// 登记一个没有复用键的一次性弹窗（关闭时同样只隐藏、不销毁）。
fn record_parked_popup(window_id: WindowId, content: WeakEntity<PopupWindowContent>) {
    if let Ok(mut slots) = parked_popups().lock() {
        let is_new_window = slots.insert(window_id, content).is_none();
        drop(slots);
        if is_new_window {
            crate::popup_lifecycle::record_window_registered(
                crate::popup_lifecycle::PopupWindowKind::Parked,
            );
            crate::popup_lifecycle::log_lifecycle("popup_window_registered");
        }
    }
}

fn forget_parked_popup(window_id: WindowId) {
    if let Ok(mut slots) = parked_popups().lock() {
        let removed = slots.remove(&window_id).is_some();
        drop(slots);
        if removed {
            crate::popup_lifecycle::record_window_unregistered(
                crate::popup_lifecycle::PopupWindowKind::Parked,
            );
            crate::popup_lifecycle::log_lifecycle("popup_window_unregistered");
        }
    }
}

/// 原生窗口被销毁时清掉它的登记（非 macOS、隐藏失败回落、应用退出）。
///
/// 不清理的话条目会变成永不失效的脏数据，`live_windows` 只增不减。
pub(crate) fn forget_popup_by_window(window_id: WindowId) {
    let key = reusable_popups().lock().ok().and_then(|slots| {
        slots
            .iter()
            .find(|(_, entry)| entry.handle.window_id() == window_id)
            .map(|(key, _)| key.clone())
    });

    if let Some(key) = key {
        forget_reusable_popup(&key);
    }
    forget_parked_popup(window_id);
}

/// 这个窗口的内容实体；不是弹窗（或刚被销毁）时返回 `None`。
pub(crate) fn popup_content(window_id: WindowId) -> Option<WeakEntity<PopupWindowContent>> {
    let reusable = reusable_popups().lock().ok().and_then(|slots| {
        slots
            .values()
            .find(|entry| entry.handle.window_id() == window_id)
            .map(|entry| entry.content.clone())
    });
    if reusable.is_some() {
        return reusable;
    }

    parked_popups().lock().ok()?.get(&window_id).cloned()
}

/// 这个窗口是不是弹窗（复用登记的，或一次性停放的）。
///
/// [`crate::window_close::close_window_for_reuse`] 用它区分两类窗口：
///
/// - **弹窗**：关闭 = 隐藏原生窗口（留着复用，或就此停放），**绝不销毁**；
/// - **其他窗口**（设置窗口、编辑器窗口……）：照旧销毁 —— 它们不在这条复用链上，也不存在
///   「AppKit 在自己的关闭流程里销毁窗口」那个崩溃点。
///
/// 这条兜底让视图层可以放心用这个函数替换 `window.remove_window()`：写错了也只是回到原
/// 行为，而不是把某个窗口莫名其妙地藏起来。
pub(crate) fn is_popup_window(window_id: WindowId) -> bool {
    popup_content(window_id).is_some()
}

/// 结束这个弹窗的**业务会话**：卸载业务 view，清掉焦点与通知。
///
/// 原生窗口不受影响（它已经被隐藏，等着复用或就此停放）。「复用的是窗口，不是上一轮的
/// 业务状态」—— 复用窗口下一次打开会用新的 factory 重建 view，用的也是本次调用传入的入参。
///
/// 清理**同步**发生在关闭动作内部，不交给 `defer`：延后清理会有「刚重新打开的新会话
/// 被上一轮的清理任务删掉」的时序问题，同步卸载则根本不存在这个窗口期。
pub(crate) fn end_popup_session(window: &mut Window, cx: &mut App) {
    let Some(content) = popup_content(window.window_handle().window_id()) else {
        return;
    };

    // 先清窗口级状态：隐藏的窗口不再重绘，但焦点与通知层仍然持有旧会话的实体
    // （输入状态、弹层闭包）。顺序上先清它们、再卸载业务 view 更安全。
    window.blur(cx);
    window.clear_notifications(cx);

    if content
        .update(cx, |content, cx| content.end_session(cx))
        .is_err()
    {
        // 内容实体已经没了：会话随窗口一起销毁，计数由 `Drop for PopupWindowContent` 归还。
        return;
    }

    crate::popup_lifecycle::log_lifecycle("popup_session_ended");
}

/// 给弹窗接上所有关闭入口，让它们统一走 [`crate::window_close::close_window_for_reuse`]。
///
/// 光改视图内的取消/保存按钮是不够的：用户更常点的是**原生标题栏的红点**，那条路径由
/// AppKit 自己发起（`windowShouldClose:`），不经过应用代码；还有 Cmd-W 一类走
/// [`crate::window_close::request_close_window`] 的入口。
///
/// 漏掉红点不只是「白改」，而是会崩：那条路上窗口是由 AppKit 在**它自己的关闭流程里**
/// 销毁的，此刻窗口的响应者（字段编辑器等）可能已经被 AppKit 释放，而 Touch Bar 查找器的
/// 观察却是延迟到下一个显示周期才注销的 —— `-[_NSTouchBarFinderObservation invalidate]`
/// 于是对一个已经不在的对象调用 `removeObserver:forKeyPath:context:`，异常没人接住，
/// AppKit 的 `-[NSApplication _crashOnException:]` 直接 abort（现场见 navop#308 / navop#314，
/// 上游记为 zed#64819）。
///
/// 我们这一侧发起的关闭（保存 / 取消按钮、Cmd-W）曾经走 `remove_window()` + GPUI 的延迟
/// 回收，现场反馈「确定」那条路已经不崩了、只有红点还在崩 —— 差别正是**谁发起的销毁**。
/// 现在两条路统一：先取消 AppKit 的关闭，再交给 `close_window_for_reuse`，而它**只隐藏、
/// 不销毁**（见 [`crate::window_close::close_window_for_reuse`]）。于是「销毁原生窗口」这件事
/// 在应用运行期间彻底消失，AppKit 的延迟注销也就永远踩不到已释放的对象。
///
/// **两类弹窗都要装**：一次性弹窗（没登记复用键）同样不能让 AppKit 自己销毁窗口，
/// 否则「保存连接」这种最常见的一步还是闪退。它们没有复用键，关闭后只是停放在那里。
///
/// 未启用 [`crate::window_close::HIDE_WINDOWS_ON_CLOSE`] 的构建（该开关当前恒为 `false`）
/// 什么都不装：关闭交回 AppKit 与 GPUI 自己的销毁路径（`remove_window()`）。
fn install_popup_close_routes(window: &mut Window, cx: &mut App) {
    if !crate::window_close::HIDE_WINDOWS_ON_CLOSE {
        return;
    }

    // ① 原生关闭（macOS 红点 / 平台层 close）。返回 false 表示「别关」：AppKit 不参与销毁，
    //    窗口交给 `close_window_for_reuse` 隐藏。
    window.on_window_should_close(cx, |window, cx| {
        let _ = crate::window_close::close_window_for_reuse(window, cx);
        false
    });

    // ② 走 `request_close_window` 的关闭（Cmd-W 等）。装了 handler 就不再落到默认的
    //    `remove_window()` 上。
    crate::window_close::set_window_close_handler(
        window.window_handle(),
        |handle, cx| {
            cx.defer(move |cx| {
                let _ = handle.update(cx, |_, window, cx| {
                    let _ = crate::window_close::close_window_for_reuse(window, cx);
                });
            });
        },
        cx,
    );
}

/// 该复用键的弹窗只是被隐藏（还活着）时：用 `factory` 重建它里面的 view，然后重新显示，
/// 返回 `true`。
///
/// 用 `factory`（**本次调用**传进来的那个）而不是上次的，是为了保证内容跟着参数走：
/// factory 决定 view 的全部入参，重建就等于「按这次的要求重新做一遍内容」。
///
/// 探活失败的条目会顺手清掉 —— 窗口可能已经被真正销毁（隐藏失败回落、被系统关掉、
/// 或退出流程回收），此时返回 `false` 让调用方走新建路径。
fn reshow_reusable_popup(
    key: &str,
    options: &PopupWindowOptions,
    factory: &dyn Fn(&mut Window, &mut App) -> AnyView,
    cx: &mut AsyncApp,
) -> bool {
    let Some((handle, content)) = reusable_popups().lock().ok().and_then(|slots| {
        slots
            .get(key)
            .map(|entry| (entry.handle, entry.content.clone()))
    }) else {
        return false;
    };

    let title = options.title.to_string();
    let window_size = size(px(options.width), px(options.height));

    let result = cx.update_window(handle, |_, window, cx| {
        let view = factory(window, cx);
        if content
            .update(cx, |content, cx| {
                content.replace_session(view, title.clone(), options.hide_titlebar_when_fullscreen);
                cx.notify();
            })
            .is_err()
        {
            // 内容实体已经没了（窗口正在销毁），当作没命中。
            return false;
        }
        crate::popup_lifecycle::log_lifecycle("popup_session_reopened");

        // 尺寸和标题都是调用方按本次参数给的，重新显示时要跟着回到本次的取值 ——
        // 窗口在上一轮里可能被 view 自己 resize 过（更新弹窗下载中就会）。
        window.resize(window_size);
        window.set_window_title(&title);
        window.activate_window();
        true
    });

    if matches!(result, Ok(true)) {
        return true;
    }

    forget_reusable_popup(key);
    false
}

actions!(popup_window, [ExitPopupFullscreen]);

struct FullscreenHintNotification;

pub fn init(cx: &mut App) {
    cx.bind_keys([KeyBinding::new(
        "escape",
        ExitPopupFullscreen,
        Some(FULLSCREEN_POPUP_CONTEXT),
    )]);
}

/// 弹出窗口的配置选项
pub struct PopupWindowOptions {
    pub title: SharedString,
    pub width: f32,
    pub height: f32,
    pub min_width: f32,
    pub min_height: f32,
    pub fullscreen: bool,
    pub hide_titlebar_when_fullscreen: bool,
    pub fullscreen_hint: Option<SharedString>,
}

impl Default for PopupWindowOptions {
    fn default() -> Self {
        Self {
            title: "".into(),
            width: 600.0,
            height: 550.0,
            min_width: 400.0,
            min_height: 300.0,
            fullscreen: false,
            hide_titlebar_when_fullscreen: false,
            fullscreen_hint: None,
        }
    }
}

impl PopupWindowOptions {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
            ..Default::default()
        }
    }

    pub fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    pub fn height(mut self, height: f32) -> Self {
        self.height = height;
        self
    }

    pub fn min_width(mut self, min_width: f32) -> Self {
        self.min_width = min_width;
        self
    }

    pub fn min_height(mut self, min_height: f32) -> Self {
        self.min_height = min_height;
        self
    }

    pub fn size(mut self, width: f32, height: f32) -> Self {
        self.width = width;
        self.height = height;
        self
    }

    pub fn fullscreen(mut self, fullscreen: bool) -> Self {
        self.fullscreen = fullscreen;
        self
    }

    pub fn hide_titlebar_when_fullscreen(mut self, hide: bool) -> Self {
        self.hide_titlebar_when_fullscreen = hide;
        self
    }

    pub fn fullscreen_hint(mut self, hint: impl Into<SharedString>) -> Self {
        self.fullscreen_hint = Some(hint.into());
        self
    }
}

/// 创建弹出窗口
///
/// 异步创建一个独立的弹出窗口，窗口内容由 `create_view_fn` 提供。
/// 窗口会自动包含 Root 组件以支持 notification 等功能。
///
/// 弹出窗口应出现在「父窗口 / 当前激活窗口」所在的屏幕，而不是恒落主屏幕。
/// 优先使用 `parent_window`（调用方透传的真实窗口，最可靠）；没有时回退到当前激活窗口；
/// 再没有则回退主屏幕。`parent_window` 的 display_id 在读取前先 `bounds_changed` 一次，
/// 刷新「窗口被拖到另一屏但未触发 resize」时 GPUI 未刷新的缓存 `display_id`，
/// 从而保证弹窗落在真实所在屏幕。
///
/// # 关闭行为
///
/// 开了 `macos-touchbar-window-hide`（当前发布流水线只给 x86_64 macOS 打开）的构建里，
/// 这个窗口关闭时
/// **只隐藏、不销毁**（见 [`crate::window_close::close_window_for_reuse`]）：它是 macOS
/// Touch Bar 机型闪退（navop#308 / navop#314）的修法。没有复用键，所以关闭后不会有人
/// 重新显示它，下一次打开是新建一个窗口 —— 代价与后续收敛方向见 [`PARKED_POPUPS`]。
/// 其他构建（ARM macOS / Windows / Linux）保持原来的「关闭即销毁」。
///
/// # 参数
/// - `options`: 窗口配置选项
/// - `create_view_fn`: 创建窗口内容的闭包
/// - `parent_window`: 触发弹窗的父窗口；为 `None` 时回退到激活窗口 / 主屏幕
/// - `cx`: App 上下文
///
/// # 示例
/// ```ignore
/// open_popup_window(
///     PopupWindowOptions::new("My Window").size(600.0, 400.0),
///     |window, cx| {
///         cx.new(|cx| MyView::new(window, cx))
///     },
///     Some(window),
///     cx,
/// );
/// ```
pub fn open_popup_window<F, E>(
    options: PopupWindowOptions,
    create_view_fn: F,
    parent_window: Option<&mut Window>,
    cx: &mut App,
) where
    E: Into<AnyView>,
    F: FnOnce(&mut Window, &mut App) -> E + Send + 'static,
{
    // 一次性窗口：这个 factory 只会被调用一次。用 `RefCell` 把它兜成一个可调用的 `Fn`，
    // 万一将来有人把一次性窗口接到复用路径上，会在第二次调用时立刻 panic，
    // 而不是静默把上一次的内容重新显示出来。
    let create_view_fn = RefCell::new(Some(create_view_fn));
    let factory: Box<dyn Fn(&mut Window, &mut App) -> AnyView> = Box::new(move |window, cx| {
        let create_view_fn = create_view_fn
            .borrow_mut()
            .take()
            .expect("one-shot popup window view factory was called more than once");
        create_view_fn(window, cx).into()
    });

    open_popup_window_inner(options, None, factory, parent_window, cx);
}

/// 创建**关闭后复用**的弹出窗口。
///
/// 与 [`open_popup_window`] 的唯一区别在关闭之后：这个窗口在 macOS 上关闭时只是被隐藏
/// （配合 [`crate::window_close::close_window_for_reuse`]），下次用同一个 `reuse_key`
/// 再打开就直接重新显示那个窗口，不再新建，因此原生 NSWindow 从不销毁。
/// 未启用 `macos-touchbar-window-hide` 的构建里两者行为一致（关闭即销毁）。
///
/// # `reuse_key` 怎么取
///
/// 复用粒度是「同一类弹窗的**同一个目标**」，所以键要把目标身份带上，别只写弹窗种类：
///
/// - `"settings.global-proxy"` —— 全局只有一个的窗口，用常量就够了；
/// - `format!("connection-form:ssh:{}", id_or_new)` —— 同一类弹窗会为不同连接各开一个，
///   键里必须带连接身份，否则给 B 打开表单会把 A 的窗口顶掉（同时编辑两个连接是很常见的）。
///
/// 键一旦确定就不要再变（同一个键必须始终指同一个目标），否则旧窗口会永远停放在那里。
///
/// # 为什么需要它
///
/// macOS 上销毁原生窗口会让 AppKit 的 Touch Bar 观察者去注销一个已经 dealloc 的对象，
/// 抛出的 ObjC 异常无人接住 ⇒ 闪退（`EXC_CRASH (SIGABRT)`）。把「关闭」换成「隐藏」从根上
/// 避开这个竞态。完整机制见 [`crate::window_close::hide_for_reuse`] 与 skill
/// `navop-macos-selector-availability-crash` §9.11。
///
/// # `create_view_fn` 为什么是 `Fn` 而不是 `FnOnce`
///
/// 复用窗口时里面的 view 会被**重建**，所以 factory 必须能被调用多次。重建用的是
/// **本次调用**传入的 factory（不是记住上次那个），view 的入参因此跟着每次调用走 ——
/// 复用窗口不会复用旧数据。代价是 factory 捕获的入参要在调用点 `clone()` 一次。
///
/// # 什么样的窗口能用
///
/// - 内容完全由入参决定、可以随时重建的：表单、对话框、预览、比较窗口等。
/// - **不要**用于持有长生命周期会话状态的窗口（例如一条活的远程桌面连接）。
///
/// # 配套动作
///
/// 关闭路径必须用 [`crate::window_close::close_window_for_reuse`] 代替
/// `window.remove_window()`；否则窗口照样被销毁，复用键永远命中不到。
///
/// # 复用的是什么
///
/// 复用**原生窗口**，不复用业务会话：窗口关闭时会卸载业务 view（见
/// [`end_popup_session`]），下次打开用本次 factory 重建。所以调用方不需要写
/// 任何复位逻辑，也不要把「关闭后还能读回上次的输入」当成契约。
pub fn open_reusable_popup_window<F, E>(
    options: PopupWindowOptions,
    reuse_key: impl Into<String>,
    create_view_fn: F,
    parent_window: Option<&mut Window>,
    cx: &mut App,
) where
    E: Into<AnyView>,
    F: Fn(&mut Window, &mut App) -> E + 'static,
{
    let factory: Box<dyn Fn(&mut Window, &mut App) -> AnyView> =
        Box::new(move |window, cx| create_view_fn(window, cx).into());

    open_popup_window_inner(options, Some(reuse_key.into()), factory, parent_window, cx);
}

/// [`open_popup_window`] 与 [`open_reusable_popup_window`] 的公共实现。
///
/// `reuse_key` 为 `None` 时是一次性弹窗：关闭时同样只隐藏、不销毁，但没有复用键，
/// 所以窗口只是**停放**在那里（详见 [`PARKED_POPUPS`]）。
fn open_popup_window_inner(
    options: PopupWindowOptions,
    reuse_key: Option<String>,
    factory: Box<dyn Fn(&mut Window, &mut App) -> AnyView + 'static>,
    parent_window: Option<&mut Window>,
    cx: &mut App,
) {
    // 解析父窗口 / 激活窗口所在显示器的 id，并保留父窗口 bounds 用于相对居中。
    let (parent_display_id, parent_window_bounds) = match parent_window {
        Some(window) => {
            window.bounds_changed(cx);
            let display_id = window.display(cx).map(|display| display.id());
            (display_id, Some(window.bounds()))
        }
        None => {
            // 没有父窗口时，优先活动窗口；活动窗口取不到（仅持有 App 上下文的调用方）
            // 则回退到注册的主窗口 handle，保证弹窗落在用户实际所在屏幕。
            let from_active: Option<(Option<gpui::DisplayId>, Option<Bounds<gpui::Pixels>>)> =
                cx.active_window().and_then(|handle| {
                    handle
                        .update(cx, |_, window, cx| {
                            window.bounds_changed(cx);
                            window
                                .display(cx)
                                .map(|display| (Some(display.id()), Some(window.bounds())))
                        })
                        .ok()
                        .flatten()
                });
            from_active
                .or_else(|| {
                    let handle = main_window_handle()?;
                    cx.update_window(handle, |_, window, cx| {
                        window.bounds_changed(cx);
                        window
                            .display(cx)
                            .map(|display| (Some(display.id()), Some(window.bounds())))
                    })
                    .ok()
                    .flatten()
                })
                .unwrap_or((None, None::<Bounds<gpui::Pixels>>))
        }
    };

    let mut window_size = size(px(options.width), px(options.height));
    let display = parent_display_id
        .and_then(|id| cx.find_display(id))
        .or_else(|| cx.primary_display());
    if let Some(d) = display.as_ref() {
        let display_size = d.bounds().size;
        window_size.width = window_size.width.min(display_size.width * 0.85);
        window_size.height = window_size.height.min(display_size.height * 0.85);
    }
    // 相对父窗口居中：以父窗口中心为锚点，弹窗尺寸不超出显示器可视范围。
    // 没有父窗口 bounds 时回退到显示器居中（`Bounds::centered` 生成目标显示器
    // 局部坐标的居中 bounds，平台层会再叠加该显示器 frame 原点）。
    let window_bounds = match parent_window_bounds {
        Some(parent_bounds) => {
            let mut bounds = Bounds::centered_at(parent_bounds.center(), window_size);
            // 钳制到所在显示器内，避免大弹窗溢出屏幕边缘。
            if let Some(d) = display.as_ref() {
                let visible = d.bounds();
                bounds.origin.x = bounds
                    .origin
                    .x
                    .max(visible.origin.x)
                    .min(visible.right() - window_size.width);
                bounds.origin.y = bounds
                    .origin
                    .y
                    .max(visible.origin.y)
                    .min(visible.bottom() - window_size.height);
            }
            bounds
        }
        None => Bounds::centered(parent_display_id, window_size, cx),
    };
    let title = options.title.clone();
    let fullscreen_hint = options.fullscreen_hint.clone();

    cx.spawn(async move |cx| {
        // 复用它：同一个键的弹窗如果只是被隐藏（还活着），用本次的 factory 重建里面的 view
        // 再重新显示，绝不新建 ——「销毁再重建」正是要避开的那条路径
        // （见 `crate::window_close::hide_for_reuse`）。
        if let Some(key) = reuse_key.as_deref()
            && reshow_reusable_popup(key, &options, factory.as_ref(), cx)
        {
            return Ok(());
        }

        let window_bounds = if options.fullscreen {
            WindowBounds::Fullscreen(window_bounds)
        } else {
            WindowBounds::Windowed(window_bounds)
        };
        let window_opts = WindowOptions {
            window_bounds: Some(window_bounds),
            titlebar: Some(TitleBar::title_bar_options()),
            window_min_size: Some(Size {
                width: px(options.min_width),
                height: px(options.min_height),
            }),
            display_id: parent_display_id,
            kind: WindowKind::Normal,
            window_background: gpui::WindowBackgroundAppearance::Transparent,
            #[cfg(target_os = "linux")]
            window_decorations: Some(gpui::WindowDecorations::Client),
            ..Default::default()
        };

        let window = cx.open_window(window_opts, |window, cx| {
            crate::window_close::register_window(window.window_handle(), cx);
            let view: AnyView = factory(window, cx);
            let title = title.to_string();
            let content = cx.new(|_| {
                PopupWindowContent::new(view, title, options.hide_titlebar_when_fullscreen)
            });
            if crate::window_close::HIDE_WINDOWS_ON_CLOSE {
                // 登记只在这套机制生效的构建里做：未启用时窗口关闭即销毁，登记表既没用，
                // 还会让 `forget_popup_by_window` 这类清理逻辑去碰不存在的条目。
                if let Some(key) = reuse_key {
                    record_reusable_popup(key, window.window_handle(), content.downgrade());
                } else {
                    record_parked_popup(window.window_handle().window_id(), content.downgrade());
                }
                // 关闭路线对两类弹窗都要装：一次性弹窗也必须先取消 AppKit 的关闭、再走统一的
                // 隐藏路线，否则「保存连接」这类最常见的关闭动作在红点路径上还是会闪退。
                install_popup_close_routes(window, cx);
            }
            cx.new(|cx| Root::new(content, window, cx))
        })?;

        // Updating through the typed WindowHandle<Root> leases Root for the whole
        // callback. push_notification updates Root again, so use the untyped
        // window path to avoid a re-entrant Root lease.
        cx.update_window(window.into(), |_, window, cx| {
            window.activate_window();
            window.set_window_title(&title);
            if let Some(fullscreen_hint) = fullscreen_hint {
                window.push_notification(
                    Notification::info(fullscreen_hint)
                        .id::<FullscreenHintNotification>()
                        .autohide(true),
                    cx,
                );
            }
        })?;

        Ok::<_, anyhow::Error>(())
    })
    .detach();
}

/// 一个复用弹窗的**内容实体**：它只做两件事 —— 渲染本次业务会话，以及在关闭时把它卸掉。
///
/// `view` 是 `Option` 而不是不可空字段：这是「关闭即结束会话」的落点。
/// 关闭时把它置为 `None`，业务 view 连同它持有的数据、连接引用与任务句柄一起释放；
/// 下次打开再用新的 factory 重建。窗口复用**不等于**会话复用。
pub(crate) struct PopupWindowContent {
    view: Option<AnyView>,
    title: String,
    hide_titlebar_when_fullscreen: bool,
    titlebar_revealed: bool,
}

impl PopupWindowContent {
    fn new(view: AnyView, title: String, hide_titlebar_when_fullscreen: bool) -> Self {
        crate::popup_lifecycle::record_session_opened();
        Self {
            view: Some(view),
            title,
            hide_titlebar_when_fullscreen,
            titlebar_revealed: false,
        }
    }

    /// 换上一次**新打开**的业务会话（复用窗口重新显示时传入本次 factory 重建的 view）。
    ///
    /// 旧会话还活着时它就在这里被替换掉，计数不变；否则记为一次新会话。
    fn replace_session(
        &mut self,
        view: AnyView,
        title: String,
        hide_titlebar_when_fullscreen: bool,
    ) {
        if self.view.replace(view).is_none() {
            crate::popup_lifecycle::record_session_opened();
        }
        self.title = title;
        self.hide_titlebar_when_fullscreen = hide_titlebar_when_fullscreen;
        self.titlebar_revealed = false;
    }

    /// 结束本次业务会话：卸载业务 view，回到「空闲窗口」状态。重复调用是安全的。
    fn end_session(&mut self, cx: &mut Context<Self>) {
        if self.view.take().is_none() {
            return;
        }
        self.titlebar_revealed = false;
        crate::popup_lifecycle::record_session_ended();
        cx.notify();
    }
}

impl Drop for PopupWindowContent {
    fn drop(&mut self) {
        // 窗口被真正销毁（非 macOS、隐藏失败回落、应用退出）时会话不经过关闭入口，
        // 计数也要在这里归还，否则 `live_sessions` 只增不减。
        if self.view.is_some() {
            crate::popup_lifecycle::record_session_ended();
        }
    }
}

impl Render for PopupWindowContent {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let sheet_layer = render_sheet_layer(window, cx);
        let dialog_layer = render_dialog_layer(window, cx);
        let notification_layer = render_notification_layer(window, cx);
        let auto_hide_titlebar = self.hide_titlebar_when_fullscreen && window.is_fullscreen();

        v_flex()
            .relative()
            .when(auto_hide_titlebar, |this| {
                this.key_context(FULLSCREEN_POPUP_CONTEXT)
                    .on_action(cx.listener(|this, _: &ExitPopupFullscreen, window, cx| {
                        this.titlebar_revealed = false;
                        window.toggle_fullscreen();
                        cx.stop_propagation();
                        cx.notify();
                    }))
            })
            .justify_center()
            .size_full()
            .bg(cx.theme().background)
            .opacity(crate::settings::AppSettings::global(cx).window_opacity)
            .when(!auto_hide_titlebar, |this| {
                this.child(render_popup_titlebar(self.title.clone()))
            })
            .children(self.view.clone())
            .children(sheet_layer)
            .children(dialog_layer)
            .children(notification_layer)
            .when(auto_hide_titlebar, |this| {
                this.child(
                    div()
                        .id("fullscreen-titlebar-reveal-zone")
                        .absolute()
                        .top_0()
                        .left_0()
                        .w_full()
                        .h(if self.titlebar_revealed {
                            TITLE_BAR_HEIGHT
                        } else {
                            px(4.0)
                        })
                        .overflow_hidden()
                        .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                            if this.titlebar_revealed != *hovered {
                                this.titlebar_revealed = *hovered;
                                cx.notify();
                            }
                        }))
                        .when(self.titlebar_revealed, |this| {
                            this.child(render_popup_titlebar(self.title.clone()))
                        }),
                )
            })
    }
}

fn render_popup_titlebar(title: String) -> TitleBar {
    TitleBar::new().child(
        div()
            .flex()
            .items_center()
            .justify_center()
            .flex_1()
            .text_sm()
            .font_weight(gpui::FontWeight::MEDIUM)
            .child(title),
    )
}

/// 复用机制的源码契约。
///
/// 「关闭即隐藏」的修复有一个特点：**改坏了不会报错，只会悄悄失去作用** ——
/// 窗口还是会销毁（于是 Touch Bar 机型还是会崩），或者更糟：藏进虚无再也没人打开（泄漏）。
/// 所以把下面几条不变量钉在测试里。本机没有 Touch Bar，行为层面无法复现，这里是唯一能自动化的防线。
#[cfg(test)]
mod reuse_contract_tests {
    const POPUP_SOURCE: &str = include_str!("popup_window.rs");
    const CLOSE_SOURCE: &str = include_str!("window_close.rs");

    /// 截出 `signature` 开头、到下一个顶层 `}` 为止的源码（不含签名上方的文档注释）。
    fn body<'a>(source: &'a str, signature: &str) -> &'a str {
        let start = source
            .find(signature)
            .unwrap_or_else(|| panic!("`{signature}` is gone; the reuse mechanism changed shape"));
        let rest = &source[start..];
        let end = rest
            .find("\n}\n")
            .map(|offset| offset + 1)
            .unwrap_or(rest.len());
        &rest[..end]
    }

    /// **弹窗只隐藏、不销毁**；销毁只剩「不是弹窗」和「当前构建没开保护」两条路。
    ///
    /// 0.3.118 的现场把不变量抬到了这一步：只要窗口是在**某条路线**上被销毁的，那条路线
    /// 就会被 AppKit 的延迟注销踩到（#308 「确定」、#314 「保存」、以及红点）。所以成功
    /// 隐藏的分支里绝不能出现销毁动作 —— 否则修了三条路、还会漏第四条。
    ///
    /// 「隐藏失败」曾经是第三条销毁路线（`warn!` + `remove_window()`），现在不是了：受保护
    /// 模式下的销毁同样会经过 AppKit 的关闭流程，把「关不掉」降级成「偷偷销毁」等于把崩溃
    /// 挪回原位。失败要**保留窗口**、记 error 日志，并把 `Retained` 交回调用方。
    #[test]
    fn popup_windows_are_hidden_and_never_destroyed() {
        let close = body(CLOSE_SOURCE, "pub fn close_window_for_reuse");

        let guard = close
            .find("is_popup_window(")
            .expect("close_window_for_reuse must ask whether the window is a popup window");
        let hide = close
            .find("hide_for_reuse(")
            .expect("close_window_for_reuse must hide popup windows");
        assert!(
            guard < hide,
            "the popup check must come *before* hiding: non-popup windows (settings, editors) \
             are still destroyed on purpose and must not be hidden by accident"
        );

        let destroys = close.matches("window.remove_window();").count();
        assert_eq!(
            destroys, 1,
            "destroying is only allowed when the window is not a popup (`Ok(false)`, i.e. a build \
             that did not opt in); neither a successful hide nor a failed one may destroy the \
             window, otherwise the Touch Bar finder can retract an observation of a dead object again"
        );

        // 决策本身抽成了纯函数，「隐藏失败必须保留窗口」才能在没有真窗口的情况下回归。
        let plan = body(CLOSE_SOURCE, "fn close_plan(is_popup: bool");
        assert!(
            plan.contains("Err(_) => ClosePlan::Retain"),
            "a failed hide must keep the window (`Retain`) instead of silently degrading to a \
             destroy"
        );
        assert!(
            !plan.contains("remove_window"),
            "close_plan is the pure decision only: destroying belongs to the funnel, so that a \
             regression cannot hide behind `Ok(false)`"
        );

        let failure = close
            .split("ClosePlan::Retain")
            .nth(1)
            .expect("the funnel must still handle a failed hide explicitly");
        assert!(
            failure.contains("WindowCloseOutcome::Retained"),
            "a failed hide must report `Retained` to the caller instead of silently degrading \
             to a destroy"
        );
        assert!(
            !failure.contains("remove_window"),
            "a failed hide must keep the window and its session: destroying it here is exactly \
             the AppKit close path this switch exists to avoid"
        );

        // 保存类流程的收尾：窗口没关掉时必须告诉用户，而不是让表单静默留在屏幕上（留在屏幕上
        // 的表单还能再点一次「保存」，那正是「已保存但没关掉」以外最容易漏的一条）。
        let after_save = body(CLOSE_SOURCE, "pub fn close_window_after_save");
        assert!(
            after_save.contains("WindowCloseOutcome::Retained"),
            "close_window_after_save must branch on `Retained`: that is the only case where the \
             form stays on screen and the user has to be told the save landed but the window did not"
        );
        assert!(
            after_save.contains("push_notification"),
            "close_window_after_save must tell the user when the window could not be closed"
        );
    }

    /// 重新显示时用**本次调用**的 factory 重建 view（经由 `replace_session`）。
    ///
    /// 如果注册表把第一次的 factory（或 view）留下来了，第二次打开就会显示第一次的内容 ——
    /// 典型症状是「同一个导出窗口换了一张表，列还是旧表的」。这是复用最危险的退化方向，
    /// 所以显式禁止注册表里出现 factory。
    #[test]
    fn reshow_rebuilds_the_view_with_the_current_factory() {
        let entry = body(POPUP_SOURCE, "struct ReusablePopup");
        assert!(
            !entry.contains("factory"),
            "ReusablePopup must not remember a view factory: reusing the first call's factory \
             would show stale content on the second open"
        );

        let reshow = body(POPUP_SOURCE, "fn reshow_reusable_popup");
        assert!(
            reshow.contains("factory: &dyn Fn"),
            "reshow_reusable_popup must take the factory of the *current* call"
        );
        assert!(
            reshow.contains("content.replace_session("),
            "reshow_reusable_popup must swap in the rebuilt view"
        );
    }

    /// 「隐藏窗口」必须同时**结束业务会话**：卸载 view，并归还计数器。
    ///
    /// 只隐藏不卸载的话，上一次打开留下的 view（连同它持有的数据与任务句柄）会被内容树
    /// 一直强引用着 —— 用户不再打开那类窗口时就是纯泄漏，而且注册表里的 `WeakEntity` 管不到它。
    /// 这条链路断在任何一环都会静默退化：关闭动作不再调用卸载、卸载不再真的放开 view、
    /// 或者计数器只加不减。
    #[test]
    fn closing_ends_the_business_session() {
        let close = body(CLOSE_SOURCE, "pub fn close_window_for_reuse");
        let hide = close
            .find("hide_for_reuse(")
            .expect("close_window_for_reuse must hide popup windows");
        let end = close
            .find("end_popup_session(")
            .expect("closing must also end the business session, not just hide the window");
        assert!(
            hide < end,
            "the session must only end after the window was actually hidden: a failed hide \
             keeps the window and its session (nothing is destroyed, so no `Drop` returns the \
             count)"
        );

        let content = body(POPUP_SOURCE, "struct PopupWindowContent");
        assert!(
            content.contains("view: Option<AnyView>"),
            "PopupWindowContent.view must be Option: closing has to be able to unload it"
        );

        let end_session = body(POPUP_SOURCE, "fn end_session(&mut self");
        assert!(
            end_session.contains("self.view.take()"),
            "end_session must release the business view instead of keeping it alive"
        );

        let release = body(POPUP_SOURCE, "fn end_popup_session");
        assert!(
            release.contains("content.end_session"),
            "the close route must end the popup's business session"
        );
        assert!(
            release.contains("window.blur(") && release.contains("clear_notifications("),
            "the hidden window must also drop focus and notification state"
        );

        let drop_impl = body(POPUP_SOURCE, "impl Drop for PopupWindowContent");
        assert!(
            drop_impl.contains("record_session_ended"),
            "destroying a window with a live session must return the count on drop"
        );
    }

    /// 去掉行注释和所有空白，用来做「A 紧跟在 B 之后」这类结构断言（不受缩进 / 换行影响）。
    ///
    /// 注释必须一起去掉：注释文字会被压进字符串里，让「上一个有效 token 是什么」判断失真。
    fn squash_code(source: &str) -> String {
        source
            .lines()
            .map(|line| line.split_once("//").map_or(line, |(code, _)| code))
            .flat_map(str::chars)
            .filter(|ch| !ch.is_whitespace())
            .collect()
    }

    /// **每一类**弹窗都要接上原生关闭路线，而且这条路线必须在 `reuse_key` 分支**之外**。
    ///
    /// 只改视图里的取消/保存按钮是不够的：用户更常点的是原生标题栏的红点，那条路径由
    /// AppKit 自己发起（`windowShouldClose:`），不经过应用代码，而且窗口是由 AppKit 在
    /// **它自己的关闭流程里**销毁的 —— 那正是 #308 / #314 的崩溃点。一次性弹窗
    /// （保存连接那类）没接住红点的话，最常见的关闭动作还是闪退。
    #[test]
    fn every_popup_intercepts_the_native_close_route() {
        let routes = body(POPUP_SOURCE, "fn install_popup_close_routes");
        assert!(
            routes.contains("on_window_should_close"),
            "the native close route (macOS traffic light / windowShouldClose:) must be intercepted"
        );
        assert!(
            routes.contains("set_window_close_handler"),
            "the request_close_window route (Cmd-W and friends) must be intercepted"
        );
        assert!(
            routes.contains("close_window_for_reuse"),
            "every intercepted route must go through the single close funnel"
        );

        let opener = squash_code(body(POPUP_SOURCE, "fn open_popup_window_inner"));
        let (before_install, _) = opener
            .split_once("install_popup_close_routes(window,cx);")
            .expect("the opener must install the close routes on every window it creates");
        assert!(
            before_install.ends_with('}'),
            "install_popup_close_routes must be a sibling of the `reuse_key` branch: inside it, \
             one-shot popups (the save-connection dialog) would keep letting AppKit destroy the \
             window and still crash"
        );
    }

    /// 一次性弹窗也必须登记（停放到 `PARKED_POPUPS`）。
    ///
    /// 关闭时靠登记表按窗口找到内容实体、把业务 view 卸掉；没登记的话窗口是隐藏了，
    /// 但上一次打开的数据还挂在那里。而且它必须在 `else` 分支里：一次性弹窗的 factory
    /// 用 `RefCell` 兜成了只能调用一次（[`open_popup_window`]），接到复用路径上会在第二次
    /// 调用时 panic。
    #[test]
    fn one_shot_popups_are_registered_as_parked() {
        let opener = squash_code(body(POPUP_SOURCE, "fn open_popup_window_inner"));
        assert!(
            opener.contains("record_reusable_popup("),
            "reuse-keyed popups must keep their registration: it is what makes re-showing work"
        );
        assert!(
            opener.contains("}else{record_parked_popup("),
            "one-shot popups must be parked in the else branch of the `reuse_key` check: \
             without an entry the close route cannot find their content entity, and reusing a \
             one-shot factory would panic on the second call"
        );
    }

    /// **开关只有一个来源**，而且它把整条链路上的每一环都门控了。
    ///
    /// 这套机制只在打包时传了 `macos-touchbar-window-hide` 的 macOS 包里启用（见
    /// `crates/core/Cargo.toml` 的 `macos-touchbar-window-hide`；该开关当前恒为 `false`，
    /// 上游 zed#65186 已修掉根因），其他构建必须逐字退回原行为。漏掉任何一环都会变成
    /// 半开半关的状态，而且都不报错、只静默退化：
    /// 登记了却不隐藏（窗口照样销毁，条目永远探活失败，白占内存）、隐藏了却不登记
    /// （窗口藏起来但业务 view 不卸载，纯泄漏）、装了关闭路线却不隐藏（点红点没反应）。
    #[test]
    fn the_hide_switch_gates_every_link_of_the_chain() {
        assert!(
            CLOSE_SOURCE.matches("const HIDE_WINDOWS_ON_CLOSE").count() == 1,
            "the switch must be defined exactly once: two definitions is how builds end up \
             half-enabled"
        );
        assert!(
            squash_code(CLOSE_SOURCE).contains(
                "pubconstHIDE_WINDOWS_ON_CLOSE:bool=cfg!(all(target_os=\"macos\",feature=\"macos-touchbar-window-hide\"));"
            ),
            "the switch must stay derived from `target_os = \"macos\"` **and** the cargo \
             feature: a build that forgets the target check would turn the workaround on for \
             Windows and Linux too (the feature itself carries no architecture condition — \
             Apple Silicon Touch Bar Macs need the same protection on demand)"
        );

        let hide = body(CLOSE_SOURCE, "pub fn hide_for_reuse");
        let opt_in_guard = hide.find("if !HIDE_WINDOWS_ON_CLOSE").expect(
            "hide_for_reuse must refuse to hide in builds that do not opt in, so that the \
                 single close funnel degrades to `remove_window()` as before",
        );
        let first_step_that_can_fail = hide
            .find("MainThreadMarker::new()")
            .expect("hide_for_reuse must check that it runs on AppKit's main thread");
        assert!(
            opt_in_guard < first_step_that_can_fail,
            "the opt-in guard must come *before* every step that can fail: that ordering is \
             what makes `Err` reachable only in builds that opted in, and the close funnel \
             keeps the window on `Err` instead of destroying it"
        );

        let routes = body(POPUP_SOURCE, "fn install_popup_close_routes");
        assert!(
            routes.contains("if !crate::window_close::HIDE_WINDOWS_ON_CLOSE"),
            "install_popup_close_routes must install nothing in builds that do not opt in: \
             intercepting the native close route without hiding leaves the window un-closable"
        );

        let opener = squash_code(body(POPUP_SOURCE, "fn open_popup_window_inner"));
        assert!(
            opener.contains("ifcrate::window_close::HIDE_WINDOWS_ON_CLOSE{"),
            "the opener must only register windows in builds that opt in — otherwise the \
             registries fill up with entries whose windows are destroyed on close"
        );
    }
}
