use gpui::{
    AnyView, App, AppContext, Bounds, Context, IntoElement, ParentElement, Render, SharedString,
    Size, Styled, Window, WindowBounds, WindowKind, WindowOptions, div, px, size,
};
use gpui_component::{ActiveTheme, Root, TitleBar, v_flex};

/// 弹出窗口的配置选项
pub struct PopupWindowOptions {
    pub title: SharedString,
    pub width: f32,
    pub height: f32,
    pub min_width: f32,
    pub min_height: f32,
}

impl Default for PopupWindowOptions {
    fn default() -> Self {
        Self {
            title: "".into(),
            width: 600.0,
            height: 550.0,
            min_width: 400.0,
            min_height: 300.0,
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

    cx.spawn(async move |cx| {
        let window_opts = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(window_bounds)),
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
            });
            cx.new(|cx| Root::new(content, window, cx))
        })?;

        window.update(cx, |_, window, _| {
            window.activate_window();
            window.set_window_title(&title);
        })?;

        Ok::<_, anyhow::Error>(())
    })
    .detach();
}

struct PopupWindowContent {
    view: AnyView,
    title: String,
}

impl Render for PopupWindowContent {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let sheet_layer = Root::render_sheet_layer(window, cx);
        let dialog_layer = Root::render_dialog_layer(window, cx);
        let notification_layer = Root::render_notification_layer(window, cx);

        v_flex()
            .justify_center()
            .size_full()
            .bg(cx.theme().background)
            .opacity(crate::settings::AppSettings::global(cx).window_opacity)
            .child(
                TitleBar::new().child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .flex_1()
                        .text_sm()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .child(self.title.clone()),
                ),
            )
            .child(self.view.clone())
            .children(sheet_layer)
            .children(dialog_layer)
            .children(notification_layer)
    }
}
