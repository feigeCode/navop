use gpui::{App, AppContext, PathPromptOptions, Window};
use gpui_component::WindowExt;
use one_core::themes;
use rust_i18n::t;

pub fn prompt_import_theme(window: &mut Window, cx: &mut App) {
    let target_window = window.window_handle();
    let future = cx.prompt_for_paths(PathPromptOptions {
        files: true,
        directories: false,
        multiple: true,
        prompt: Some(
            t!("Settings.General.Appearance.select_theme_files")
                .to_string()
                .into(),
        ),
    });
    window
        .spawn(cx, async move |cx| {
            if let Ok(Ok(Some(paths))) = future.await {
                let _ = cx.update(|_view, cx: &mut App| {
                    let message = import_message(&paths, cx);
                    let _ = cx.update_window(target_window, |_, window, cx| {
                        window.push_notification(message, cx);
                    });
                });
            }
            Ok::<(), anyhow::Error>(())
        })
        .detach();
}

fn import_message(paths: &[std::path::PathBuf], cx: &mut App) -> String {
    match themes::import_theme_files(paths, cx) {
        Ok(count) => t!(
            "Settings.General.Appearance.import_theme_success",
            count = count
        )
        .to_string(),
        Err(error) => t!(
            "Settings.General.Appearance.import_theme_failed",
            error = error
        )
        .to_string(),
    }
}
