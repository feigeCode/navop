use gpui::{App, AppContext, IntoElement, ParentElement, PathPromptOptions, Styled, Window, div};
use gpui_component::{
    IconName, Sizable, WindowExt,
    button::Button,
    h_flex,
    notification::Notification,
    setting::{RenderOptions, SettingField, SettingGroup, SettingItem},
};
use notes::NotesStorage;
use rust_i18n::t;

pub fn notes_setting_group() -> SettingGroup {
    SettingGroup::new()
        .title(t!("Settings.General.Notes.group_title"))
        .item(notes_path_item())
}

fn notes_path_item() -> SettingItem {
    SettingItem::new(
        t!("Settings.General.Notes.path"),
        SettingField::render(render_notes_path_field),
    )
    .description(t!("Settings.General.Notes.path_desc").to_string())
}

fn render_notes_path_field(
    options: &RenderOptions,
    _window: &mut Window,
    _cx: &mut App,
) -> gpui::AnyElement {
    let path = configured_path_label();
    h_flex()
        .w_full()
        .min_w_0()
        .gap_2()
        .child(
            div()
                .min_w_0()
                .flex_1()
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .child(path),
        )
        .child(
            Button::new("notes-select-directory")
                .icon(IconName::FolderOpenColor)
                .label(t!("Settings.General.Notes.select_directory").to_string())
                .with_size(options.size)
                .on_click(prompt_for_notes_directory),
        )
        .into_any_element()
}

fn configured_path_label() -> String {
    NotesStorage::configured_root()
        .or_else(|_| NotesStorage::default_root())
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn prompt_for_notes_directory(_: &gpui::ClickEvent, window: &mut Window, cx: &mut App) {
    let future = cx.prompt_for_paths(PathPromptOptions {
        files: false,
        directories: true,
        multiple: false,
        prompt: Some(
            t!("Settings.General.Notes.select_directory")
                .to_string()
                .into(),
        ),
    });
    let window_handle = window.window_handle();
    window
        .spawn(cx, async move |cx| {
            let Ok(Ok(Some(paths))) = future.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let result = NotesStorage::save_configured_root(&path);
            let _ = cx.update_window(window_handle, |_, window, cx| {
                notify_location_result(result, window, cx);
                window.refresh();
            });
        })
        .detach();
}

fn notify_location_result(result: anyhow::Result<()>, window: &mut Window, cx: &mut App) {
    let notification = match result {
        Ok(()) => Notification::success(t!("Settings.General.Notes.path_saved").to_string()),
        Err(error) => Notification::error(
            t!(
                "Settings.General.Notes.path_save_failed",
                error = error.to_string()
            )
            .to_string(),
        )
        .autohide(false),
    };
    window.push_notification(notification, cx);
}
