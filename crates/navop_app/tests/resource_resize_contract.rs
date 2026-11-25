//! 资源栏（连接列表面板）拖动必须跟手：拖动中只写会话内覆盖值，松手才写设置。
//!
//! 外壳每帧都从设置求解面板宽度，而设置的落盘有 180ms 防抖；若拖动直接把宽度交给
//! 防抖保存，面板在拖动期间就不会跟手，只在松手后跳一下。

#[test]
fn resource_resize_follows_the_pointer_before_the_settings_debounce() {
    let source = include_str!("../src/navop_app.rs");

    // 拖动中：更新拖动会话、写会话内覆盖值并立刻重渲染，不等设置保存。
    let drag = source
        .split("fn resize_application_resource(")
        .nth(1)
        .unwrap()
        .split("fn finish_application_resource_resize(")
        .next()
        .unwrap();
    assert!(drag.contains("let Some(width) = crate::resource_mount::update_resource_resize_drag("));
    assert!(drag.contains("self.resource_width_drag = Some((key, width));"));
    assert!(drag.contains("cx.notify();"));
    assert!(!drag.contains("persist_application_resource_width("));

    // 松手：先把宽度写进设置内存，再交给防抖保存，最后撤掉覆盖值。
    let finish = source
        .split("fn finish_application_resource_resize(")
        .nth(1)
        .unwrap()
        .split("fn persist_application_resource_width(")
        .next()
        .unwrap();
    assert!(finish.contains("AppSettings::update(cx, |settings| {"));
    assert!(finish.contains("settings.resource_widths.insert(key, width);"));
    assert!(finish.contains("self.persist_application_resource_width(kind, width, cx);"));

    // 外壳求解布局宽度时，拖动中的覆盖值优先于设置里的持久宽度。
    assert!(source.contains(".filter(|(key, _)| *key == resource_width_key)"));
    assert!(source.contains(".map(|(_, width)| *width)"));

    // 两种布局都要记住宽度，不能只在图标布局下保存。
    let persist = source
        .split("fn persist_application_resource_width(")
        .nth(1)
        .unwrap()
        .split("fn sync_connection_sidebar_theme(")
        .next()
        .unwrap();
    assert!(!persist.contains("AppLayout::GlobalNavigation"));
    assert!(persist.contains("stable_width_key(AppSettings::global(cx).app_layout, kind)"));
}
