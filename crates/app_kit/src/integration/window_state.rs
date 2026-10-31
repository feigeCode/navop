//! 窗口尺寸与位置持久化。
//!
//! 复用 `one_core::settings::AppSettings::main_window_state`（每个产品独立的数据目录，
//! 因此三个产品互不影响），在窗口尺寸变化时防抖保存，并在下次启动时恢复。

use gpui::{App, Bounds, Context, Pixels, Size, Window, point, px, size};
use one_core::settings::{AppSettings, MainWindowState};

/// 尺寸变化后的防抖时长，避免拖动窗口时频繁写盘。
const SAVE_DELAY_MS: u64 = 150;

/// 计算窗口打开时的初始尺寸：优先使用上次保存的状态，其次默认尺寸居中。
pub fn initial_bounds(default_size: Size<Pixels>, cx: &App) -> Bounds<Pixels> {
    let display_bounds = cx.primary_display().map(|display| display.visible_bounds());
    match (
        saved_bounds(cx),
        display_bounds.map(|bounds| bounds.center()),
    ) {
        (Some(saved), Some(_center)) => saved,
        (Some(saved), None) => saved,
        (None, Some(center)) => Bounds::centered_at(center, default_size),
        (None, None) => Bounds::new(point(px(0.0), px(0.0)), default_size),
    }
}

fn saved_bounds(cx: &App) -> Option<Bounds<Pixels>> {
    let state = AppSettings::current(cx).main_window_state?;
    Some(Bounds::new(
        point(px(state.x), px(state.y)),
        size(px(state.width), px(state.height)),
    ))
}

/// 保存当前窗口尺寸与位置；无变化时跳过写盘。
pub fn save(window: &Window, cx: &mut App) {
    let bounds = window.window_bounds().get_bounds();
    let display_uuid = window
        .display(cx)
        .and_then(|display| display.uuid().ok())
        .map(|uuid| uuid.to_string());
    let Some(state) = MainWindowState::new(
        f32::from(bounds.origin.x),
        f32::from(bounds.origin.y),
        f32::from(bounds.size.width),
        f32::from(bounds.size.height),
        display_uuid,
    ) else {
        return;
    };
    if AppSettings::current(cx).main_window_state.as_ref() == Some(&state) {
        return;
    }
    AppSettings::update_and_save(cx, |settings| {
        settings.main_window_state = Some(state);
    });
}

/// 观察窗口尺寸变化并防抖保存，同时在关闭前保存一次。
pub fn observe<T: 'static>(cx: &mut Context<T>, window: &mut Window) {
    cx.observe_window_bounds(window, |_, window, cx| {
        cx.spawn_in(window, async move |_, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(SAVE_DELAY_MS))
                .await;
            let _ = cx.update(|window, cx| save(window, cx));
        })
        .detach();
    })
    .detach();
}

/// 定时保存间隔：窗口可能只被移动而不触发尺寸回调。
const AUTO_SAVE_SECS: u64 = 20;

/// 打开窗口后调用：关闭前保存一次，并周期保存位置变化。
pub fn attach(window: &mut Window, cx: &mut App) {
    window.on_window_should_close(cx, |window, cx| {
        save(window, cx);
        true
    });
    let handle = window.window_handle();
    cx.spawn(async move |cx: &mut gpui::AsyncApp| {
        loop {
            cx.background_executor()
                .timer(std::time::Duration::from_secs(AUTO_SAVE_SECS))
                .await;
            let alive = cx
                .update(|cx: &mut App| handle.update(cx, |_, window, cx| save(window, cx)).is_ok());
            if !alive {
                break;
            }
        }
    })
    .detach();
}
