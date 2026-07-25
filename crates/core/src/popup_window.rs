use gpui::{
    AnyView, App, AppContext, Bounds, Context, InteractiveElement, IntoElement, KeyBinding,
    ParentElement, Render, SharedString, Size, StatefulInteractiveElement, Styled, Window,
    WindowBounds, WindowKind, WindowOptions, actions, div, prelude::FluentBuilder, px, size,
};
use gpui_component::{
    ActiveTheme, Root, TITLE_BAR_HEIGHT, TitleBar, WindowExt, notification::Notification, v_flex,
};

const FULLSCREEN_POPUP_CONTEXT: &str = "FullscreenPopupWindow";

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
/// # 参数
/// - `options`: 窗口配置选项
/// - `create_view_fn`: 创建窗口内容的闭包
/// - `cx`: App 上下文
///
/// # 示例
/// ```ignore
/// open_popup_window(
///     PopupWindowOptions::new("My Window").size(600.0, 400.0),
///     |window, cx| {
///         cx.new(|cx| MyView::new(window, cx))
///     },
///     cx,
/// );
/// ```
pub fn open_popup_window<F, E>(options: PopupWindowOptions, create_view_fn: F, cx: &mut App)
where
    E: Into<AnyView>,
    F: FnOnce(&mut Window, &mut App) -> E + Send + 'static,
{
    let mut window_size = size(px(options.width), px(options.height));
    if let Some(display) = cx.primary_display() {
        let display_size = display.bounds().size;
        window_size.width = window_size.width.min(display_size.width * 0.85);
        window_size.height = window_size.height.min(display_size.height * 0.85);
    }
    let window_bounds = Bounds::centered(None, window_size, cx);
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
            kind: WindowKind::Normal,
            window_background: gpui::WindowBackgroundAppearance::Transparent,
            #[cfg(target_os = "linux")]
            window_decorations: Some(gpui::WindowDecorations::Client),
            ..Default::default()
        };

        let window = cx.open_window(window_opts, |window, cx| {
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
                        .on_hover(cx.listener(|this, hovered, _, cx| {
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

#[cfg(test)]
mod tests {
    #[test]
    fn fullscreen_hidden_titlebar_is_revealed_from_a_top_edge_hover_zone() {
        let source = include_str!("popup_window.rs");
        let render_start = source
            .find("impl Render for PopupWindowContent")
            .expect("popup content renderer");
        let render = &source[render_start..];

        assert!(render.contains("let auto_hide_titlebar"));
        assert!(render.contains(".id(\"fullscreen-titlebar-reveal-zone\")"));
        assert!(render.contains(".absolute()"));
        assert!(render.contains(".top_0()"));
        assert!(render.contains(".on_hover(cx.listener"));
        assert!(render.contains("this.titlebar_revealed = *hovered"));
        assert!(render.contains("TitleBar::new()"));
    }

    #[test]
    fn escape_exits_auto_hidden_popup_fullscreen() {
        let source = include_str!("popup_window.rs");

        assert!(source.contains("KeyBinding::new("));
        assert!(source.contains("\"escape\","));
        assert!(source.contains("ExitPopupFullscreen,"));
        assert!(source.contains(".when(auto_hide_titlebar"));
        assert!(source.contains(".key_context(FULLSCREEN_POPUP_CONTEXT)"));
        assert!(source.contains(".on_action(cx.listener"));
        assert!(source.contains("window.toggle_fullscreen()"));
        assert!(source.contains("cx.stop_propagation()"));
    }

    #[test]
    fn popup_fullscreen_hint_uses_an_auto_hiding_notification() {
        let source = include_str!("popup_window.rs");
        let open_start = source
            .find("pub fn open_popup_window")
            .expect("popup window opener");
        let content_start = source[open_start..]
            .find("\nstruct PopupWindowContent")
            .map(|offset| open_start + offset)
            .expect("popup window content");
        let open = &source[open_start..content_start];

        assert!(source.contains("fullscreen_hint: Option<SharedString>"));
        assert!(open.contains("if let Some(fullscreen_hint)"));
        assert!(open.contains("cx.update_window(window.into()"));
        assert!(!open.contains("window.update(cx"));
        assert!(open.contains("Notification::info(fullscreen_hint)"));
        assert!(open.contains(".autohide(true)"));
        assert!(open.contains("window.push_notification"));
    }
}
