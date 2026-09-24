use gpui::{
    AnyView, AnyWindowHandle, App, AppContext, AsyncApp, Bounds, Context, InteractiveElement,
    IntoElement, KeyBinding, ParentElement, Render, SharedString, Size, StatefulInteractiveElement,
    Styled, WeakEntity, Window, WindowBounds, WindowId, WindowKind, WindowOptions, actions, div,
    prelude::FluentBuilder, px, size,
};
use gpui_component::{
    ActiveTheme, Root, TITLE_BAR_HEIGHT, TitleBar, WindowExt, notification::Notification, v_flex,
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
/// 只有通过 [`open_reusable_popup_window`] 打开的弹窗才会登记；**没登记的弹窗行为完全
/// 不变**（仍是一次性窗口，关闭即销毁）。
static REUSABLE_POPUPS: OnceLock<Mutex<HashMap<&'static str, ReusablePopup>>> = OnceLock::new();

/// 注册表里的一项。
///
/// `content` 是复用能正确工作的关键：重新显示时可以**换掉窗口里的 view**。
/// 换 view 用的是**本次调用**传入的 factory，所以「同一个导出窗口换了一张表」「同一个
/// 更新窗口来了个新版本」这类情况不会残留上一次的内容 —— 复用窗口不等于复用旧数据。
struct ReusablePopup {
    handle: AnyWindowHandle,
    content: WeakEntity<PopupWindowContent>,
}

fn reusable_popups() -> &'static Mutex<HashMap<&'static str, ReusablePopup>> {
    REUSABLE_POPUPS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn record_reusable_popup(
    key: &'static str,
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
            crate::popup_lifecycle::record_window_registered();
            crate::popup_lifecycle::log_lifecycle("popup_window_registered");
        }
    }
}

fn forget_reusable_popup(key: &'static str) {
    if let Ok(mut slots) = reusable_popups().lock() {
        // 只有真的移除了条目才归还计数：窗口销毁与探活失败可能同时走到这里，
        // 重复扣减会让 `live_windows` 变成负数。
        let removed = slots.remove(key).is_some();
        drop(slots);
        if removed {
            crate::popup_lifecycle::record_window_unregistered();
            crate::popup_lifecycle::log_lifecycle("popup_window_unregistered");
        }
    }
}

/// 原生窗口被销毁时清掉它的登记（非 macOS、隐藏失败回落、应用退出）。
///
/// 不清理的话条目会变成永不失效的脏数据，`live_windows` 只增不减。
pub(crate) fn forget_reusable_popup_by_window(window_id: WindowId) {
    let key = reusable_popups().lock().ok().and_then(|slots| {
        slots
            .iter()
            .find(|(_, entry)| entry.handle.window_id() == window_id)
            .map(|(key, _)| *key)
    });

    if let Some(key) = key {
        forget_reusable_popup(key);
    }
}

/// 这个窗口的内容实体；不是登记过的复用弹窗时返回 `None`。
pub(crate) fn reusable_popup_content(
    window_id: WindowId,
) -> Option<WeakEntity<PopupWindowContent>> {
    reusable_popups().lock().ok().and_then(|slots| {
        slots
            .values()
            .find(|entry| entry.handle.window_id() == window_id)
            .map(|entry| entry.content.clone())
    })
}

/// 这个窗口是不是登记过的「关闭后复用」弹窗。
///
/// [`crate::window_close::close_window_for_reuse`] 用它做兜底：**没登记的窗口照旧销毁**。
/// 否则「隐藏了一个永远不会被重新显示、也没人会去复用它的窗口」就只是白白泄漏 ——
/// 一个视图层的关闭按钮改错了，不该变成内存泄漏。
pub(crate) fn is_reusable_popup(window_id: WindowId) -> bool {
    reusable_popup_content(window_id).is_some()
}

/// 结束这个复用弹窗的**业务会话**：卸载业务 view，清掉焦点与通知。
///
/// 原生窗口不受影响（它已经被隐藏，等着复用）。「复用的是窗口，不是上一轮的业务状态」——
/// 下一次打开会用新的 factory 重建 view，用的也是本次调用传入的入参。
///
/// 清理**同步**发生在关闭动作内部，不交给 `defer`：延后清理会有「刚重新打开的新会话
/// 被上一轮的清理任务删掉」的时序问题，同步卸载则根本不存在这个窗口期。
pub(crate) fn end_reusable_popup_session(window: &mut Window, cx: &mut App) {
    let Some(content) = reusable_popup_content(window.window_handle().window_id()) else {
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

/// 该复用键的弹窗只是被隐藏（还活着）时：用 `factory` 重建它里面的 view，然后重新显示，
/// 返回 `true`。
///
/// 用 `factory`（**本次调用**传进来的那个）而不是上次的，是为了保证内容跟着参数走：
/// factory 决定 view 的全部入参，重建就等于「按这次的要求重新做一遍内容」。
///
/// 探活失败的条目会顺手清掉 —— 窗口可能已经被真正销毁（隐藏失败回落、被系统关掉、
/// 或退出流程回收），此时返回 `false` 让调用方走新建路径。
/// 给一个「关闭后复用」的弹窗接上所有关闭入口，让它们统一走「隐藏」。
///
/// 光改视图内的取消/保存按钮是不够的：用户更常点的是**原生标题栏的红点**，那条路径由
/// AppKit 自己发起（`windowShouldClose:`），不经过应用代码；还有 Cmd-W 一类走
/// [`crate::window_close::request_close_window`] 的入口。漏掉任何一个，窗口还是会被销毁，
/// 复用键永远命中不到 —— 也就白改了。
fn install_reusable_popup_close_routes(window: &mut Window, cx: &mut App) {
    // ① 原生关闭（macOS 红点 / 平台层 close）。返回 false 表示「别关」，窗口已经被隐藏。
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

fn reshow_reusable_popup(
    key: &'static str,
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
/// [`end_reusable_popup_session`]），下次打开用本次 factory 重建。所以调用方不需要写
/// 任何复位逻辑，也不要把「关闭后还能读回上次的输入」当成契约。
pub fn open_reusable_popup_window<F, E>(
    options: PopupWindowOptions,
    reuse_key: &'static str,
    create_view_fn: F,
    parent_window: Option<&mut Window>,
    cx: &mut App,
) where
    E: Into<AnyView>,
    F: Fn(&mut Window, &mut App) -> E + 'static,
{
    let factory: Box<dyn Fn(&mut Window, &mut App) -> AnyView> =
        Box::new(move |window, cx| create_view_fn(window, cx).into());

    open_popup_window_inner(options, Some(reuse_key), factory, parent_window, cx);
}

/// [`open_popup_window`] 与 [`open_reusable_popup_window`] 的公共实现。
///
/// `reuse_key` 为 `None` 时是标准的一次性弹窗；为 `Some` 时走「关闭后隐藏 + 复用」路径。
fn open_popup_window_inner(
    options: PopupWindowOptions,
    reuse_key: Option<&'static str>,
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
        if let Some(key) = reuse_key
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
            if let Some(key) = reuse_key {
                record_reusable_popup(key, window.window_handle(), content.downgrade());
                install_reusable_popup_close_routes(window, cx);
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
        let sheet_layer = Root::render_sheet_layer(window, cx);
        let dialog_layer = Root::render_dialog_layer(window, cx);
        let notification_layer = Root::render_notification_layer(window, cx);
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
/// 所以把三条不变量钉在测试里。本机没有 Touch Bar，行为层面无法复现，这里是唯一能自动化的防线。
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

    /// 「隐藏」只对登记过复用键的窗口生效。
    ///
    /// 视图层的关闭按钮现在统一换成 `close_window_for_reuse`；万一某个窗口忘了（或不该）
    /// 登记复用键，隐藏它就意味着「有一个窗口永远留在虚无里」。所以判断必须排在实际隐藏之前。
    #[test]
    fn only_registered_windows_are_hidden() {
        let close = body(CLOSE_SOURCE, "pub fn close_window_for_reuse");

        let guard = close
            .find("is_reusable_popup(")
            .expect("close_window_for_reuse must ask whether the window is registered");
        let hide = close
            .find("hide_for_reuse(")
            .expect("close_window_for_reuse must hide registered windows");
        assert!(
            guard < hide,
            "the registration check must come *before* hiding, otherwise unregistered windows leak"
        );
        assert!(
            close.contains("window.remove_window();"),
            "close_window_for_reuse must still destroy unregistered windows"
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
            .expect("close_window_for_reuse must hide registered windows");
        let end = close
            .find("end_reusable_popup_session(")
            .expect("closing must also end the business session, not just hide the window");
        assert!(
            hide < end,
            "the session must only end after the window was actually hidden: a failed hide \
             destroys the window, and its content entity returns the count on drop"
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

        let release = body(POPUP_SOURCE, "fn end_reusable_popup_session");
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

    /// 复用窗口要把**所有**关闭入口都接上。
    ///
    /// 只改视图里的取消/保存按钮是不够的：用户更常点的是原生标题栏的红点，那条路径由 AppKit
    /// 的 `windowShouldClose:` 发起，压根不经过应用代码。漏掉它，窗口还是会被销毁 ——
    /// 也就等于这次修复对真实用户无效。
    #[test]
    fn reusable_windows_hide_on_every_close_route() {
        let routes = body(POPUP_SOURCE, "fn install_reusable_popup_close_routes");
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
            "every intercepted route must end up hiding the window"
        );

        let opener = body(POPUP_SOURCE, "fn open_popup_window_inner");
        assert!(
            opener.contains("install_reusable_popup_close_routes"),
            "the reusable opener must install those routes when it creates the window"
        );
    }
}
