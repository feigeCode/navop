use gpui::{
    AnyView, AnyWindowHandle, App, AppContext, Bounds, Context, InteractiveElement, IntoElement,
    KeyBinding, ParentElement, Render, SharedString, Size, StatefulInteractiveElement, Styled,
    Window, WindowBounds, WindowKind, WindowOptions, actions, div, prelude::FluentBuilder, px,
    size,
};
use std::sync::OnceLock;
use gpui_component::{
    ActiveTheme, Root, TITLE_BAR_HEIGHT, TitleBar, WindowExt, notification::Notification, v_flex,
};

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
    // 解析父窗口 / 激活窗口所在显示器的 id。
    let parent_display_id = match parent_window {
        Some(window) => {
            window.bounds_changed(cx);
            window.display(cx).map(|display| display.id())
        }
        None => {
            // 没有父窗口时，优先活动窗口；活动窗口取不到（仅持有 App 上下文的调用方）
            // 则回退到注册的主窗口 handle，保证弹窗落在用户实际所在屏幕。
            let from_active = cx.active_window().and_then(|handle| {
                handle
                    .update(cx, |_, window, cx| {
                        window.bounds_changed(cx);
                        window.display(cx)
                    })
                    .ok()
                    .flatten()
                    .map(|display| display.id())
            });
            from_active.or_else(|| {
                main_window_handle().and_then(|handle| {
                    cx.update_window(handle, |_, window, cx| {
                        window.bounds_changed(cx);
                        window.display(cx).map(|display| display.id())
                    })
                    .ok()
                    .flatten()
                })
            })
        }
    };

    let mut window_size = size(px(options.width), px(options.height));
    let display = parent_display_id
        .and_then(|id| cx.find_display(id))
        .or_else(|| cx.primary_display());
    if let Some(display) = display {
        let display_size = display.bounds().size;
        window_size.width = window_size.width.min(display_size.width * 0.85);
        window_size.height = window_size.height.min(display_size.height * 0.85);
    }
    // `Bounds::centered` 生成目标显示器局部坐标的居中 bounds，平台层会再叠加
    // 该显示器 frame 原点；必须同时指定 display_id。
    let window_bounds = Bounds::centered(parent_display_id, window_size, cx);
    let title = options.title.clone();
    let fullscreen_hint = options.fullscreen_hint.clone();

    cx.spawn(async move |cx| {
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
            let view = create_view_fn(window, cx);
            let title = title.to_string();
            let content = cx.new(|_| PopupWindowContent {
                view: view.into(),
                title,
                hide_titlebar_when_fullscreen: options.hide_titlebar_when_fullscreen,
                titlebar_revealed: false,
            });
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

struct PopupWindowContent {
    view: AnyView,
    title: String,
    hide_titlebar_when_fullscreen: bool,
    titlebar_revealed: bool,
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
            .child(self.view.clone())
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
