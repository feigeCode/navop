use crate::popup_window::PopupWindowContent;
use gpui::{
    AppContext as _, Bounds, Context, IntoElement, Render, TestAppContext, VisualTestContext,
    Window, WindowBounds, WindowOptions, prelude::*, px, size,
};
use gpui_component::{Root, TITLE_BAR_HEIGHT, Theme, v_flex};

fn popup_open_source() -> &'static str {
    let source = include_str!("popup_window.rs");
    let open_start = source
        .find("pub fn open_popup_window")
        .expect("popup window opener");
    let content_start = source[open_start..]
        .find("struct PopupWindowContent")
        .map(|offset| open_start + offset)
        .expect("popup window content");
    &source[open_start..content_start]
}

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
    assert!(!render.contains(".id(\"fullscreen-titlebar-content\")"));
    assert!(!render.contains("this.pt(px(4.0))"));
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
    let open = popup_open_source();

    assert!(source.contains("fullscreen_hint: Option<SharedString>"));
    assert!(open.contains("if let Some(fullscreen_hint)"));
    assert!(open.contains("cx.update_window(window.into()"));
    assert!(open.contains("Notification::info(fullscreen_hint)"));
    assert!(open.contains(".autohide(true)"));
    assert!(open.contains("window.push_notification"));
}

#[test]
fn every_popup_registers_with_the_shared_window_close_router() {
    let open = popup_open_source();

    assert!(open.contains("crate::window_close::register_window"));
    assert!(open.contains("window.window_handle()"));
}

#[test]
fn popup_uses_the_active_window_display() {
    let source = include_str!("popup_window.rs");
    let open = popup_open_source();

    // 弹出窗口必须出现在活动窗口所在屏幕，而不是恒落主屏幕。
    // 方案：解析活动窗口真实的 display_id（必要时先 bounds_changed 刷新过期缓存），
    // 用 GPUI 的 `Bounds::centered(display_id, ...)` 生成该屏幕下的居中 bounds，
    // 并把同一个 display_id 传给 WindowOptions，弹窗即落在活动窗口所在屏幕。
    assert!(open.contains("parent_window"));
    assert!(source.contains("window.bounds_changed(cx)"));
    assert!(source.contains("window.display(cx)"));
    assert!(open.contains("Bounds::centered(parent_display_id"));
    assert!(open.contains("display_id: parent_display_id"));
}

/// 模拟业务弹窗：根节点占满整窗；`min_height` 用来复现“内容最小高度
/// 超过标题栏下方可用高度”的小屏场景（Xshell 配置导入弹窗即属此类）。
struct FullHeightPopupView {
    min_height: Option<gpui::Pixels>,
}

impl Render for FullHeightPopupView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .when_some(self.min_height, |this, min_height| this.min_h(min_height))
            .debug_selector(|| "full-height-popup-view".to_owned())
    }
}

fn open_full_height_popup(
    cx: &mut TestAppContext,
    min_height: Option<gpui::Pixels>,
) -> VisualTestContext {
    cx.update(|cx| {
        gpui_component::init(cx);
        cx.set_global(Theme::default());
    });

    let window = cx.update(|cx| {
        let window_bounds = Bounds::centered(None, size(px(800.0), px(600.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(window_bounds)),
                ..Default::default()
            },
            |window, cx| {
                let view = cx.new(|_| FullHeightPopupView { min_height });
                let content =
                    cx.new(|_| PopupWindowContent::new(view.into(), "导入".to_owned(), false));
                cx.new(|cx| Root::new(content, window, cx))
            },
        )
        .expect("test window opens")
    });

    let mut visual = VisualTestContext::from_window(window.into(), cx);
    visual.update(|window, _| window.refresh());
    visual.run_until_parked();
    visual
}

#[gpui::test]
fn tall_full_height_popup_keeps_room_for_the_titlebar(cx: &mut TestAppContext) {
    let mut cx = open_full_height_popup(cx, Some(px(900.0)));

    let viewport = cx.update(|window, _| window.viewport_size());
    let content = cx
        .debug_bounds("full-height-popup-view")
        .expect("popup content is rendered");

    assert_eq!(
        content.origin.y, TITLE_BAR_HEIGHT,
        "业务内容必须从 34px 标题栏下方开始，不能把标题栏顶出窗口（Windows 上就是最大化/关闭按钮残缺）: content={content:?}, viewport={viewport:?}"
    );
    assert!(
        content.origin.y < viewport.height,
        "标题栏下方必须仍有可见内容区: content={content:?}, viewport={viewport:?}"
    );
}

#[gpui::test]
fn full_height_popup_fills_the_space_below_the_titlebar(cx: &mut TestAppContext) {
    let mut cx = open_full_height_popup(cx, None);

    let viewport = cx.update(|window, _| window.viewport_size());
    let content = cx
        .debug_bounds("full-height-popup-view")
        .expect("popup content is rendered");

    assert_eq!(
        content.origin.y, TITLE_BAR_HEIGHT,
        "业务内容必须紧贴标题栏下方: content={content:?}"
    );
    assert_eq!(
        content.bottom(),
        viewport.height,
        "业务内容必须占满标题栏下方的全部高度: content={content:?}, viewport={viewport:?}"
    );
}
