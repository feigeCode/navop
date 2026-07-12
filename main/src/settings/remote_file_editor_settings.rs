use extension_runtime::{GlobalExtensionRuntimeCatalog, RegisteredRemoteFileEditorContribution};
use gpui::{App, ParentElement as _, PathPromptOptions, SharedString, Window};
use gpui_component::{
    Sizable as _,
    button::Button,
    h_flex,
    setting::{SettingField, SettingGroup, SettingItem},
};
use one_core::settings::{AppSettings, RemoteFileEditorOverride, RemoteFileEditorUserSettings};
use remote_file_editor::editor_supports_current_platform;
use rust_i18n::t;

pub fn remote_file_editor_setting_group(
    default_settings: &RemoteFileEditorUserSettings,
    cx: &App,
) -> SettingGroup {
    let editors = installed_editors(cx);
    let mut items = vec![
        default_editor_item(default_settings, &editors),
        auto_upload_item(default_settings),
        conflict_check_item(default_settings),
    ];
    items.extend(editors.into_iter().map(editor_override_item));
    SettingGroup::new()
        .title(t!("Settings.General.RemoteFileEditor.group_title"))
        .items(items)
}

fn installed_editors(cx: &App) -> Vec<RegisteredRemoteFileEditorContribution> {
    cx.try_global::<GlobalExtensionRuntimeCatalog>()
        .and_then(GlobalExtensionRuntimeCatalog::get)
        .map(|catalog| {
            catalog
                .remote_file_editors()
                .iter()
                .filter(|editor| editor_supports_current_platform(&editor.platforms))
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

fn default_editor_item(
    default_settings: &RemoteFileEditorUserSettings,
    editors: &[RegisteredRemoteFileEditorContribution],
) -> SettingItem {
    let mut options = vec![(
        SharedString::from(""),
        SharedString::from(t!("Settings.General.RemoteFileEditor.none").to_string()),
    )];
    options.extend(editors.iter().map(|editor| {
        (
            SharedString::from(editor.editor_key.clone()),
            SharedString::from(editor.display_name.clone()),
        )
    }));
    SettingItem::new(
        t!("Settings.General.RemoteFileEditor.default_editor"),
        SettingField::dropdown(
            options,
            |cx: &App| {
                AppSettings::global(cx)
                    .remote_file_editor
                    .default_external_editor
                    .clone()
                    .unwrap_or_default()
                    .into()
            },
            |value: SharedString, cx: &mut App| {
                let value = value.to_string();
                AppSettings::update_and_save(cx, |settings| {
                    settings.remote_file_editor.default_external_editor =
                        (!value.is_empty()).then_some(value);
                });
            },
        )
        .default_value(SharedString::from(
            default_settings
                .default_external_editor
                .clone()
                .unwrap_or_default(),
        )),
    )
    .description(t!("Settings.General.RemoteFileEditor.default_editor_desc").to_string())
}

fn auto_upload_item(default_settings: &RemoteFileEditorUserSettings) -> SettingItem {
    SettingItem::new(
        t!("Settings.General.RemoteFileEditor.auto_upload"),
        SettingField::checkbox(
            |cx: &App| {
                AppSettings::global(cx)
                    .remote_file_editor
                    .auto_upload_external_changes
            },
            |value, cx: &mut App| {
                AppSettings::update_and_save(cx, |settings| {
                    settings.remote_file_editor.auto_upload_external_changes = value;
                });
            },
        )
        .default_value(default_settings.auto_upload_external_changes),
    )
    .description(t!("Settings.General.RemoteFileEditor.auto_upload_desc").to_string())
}

fn conflict_check_item(default_settings: &RemoteFileEditorUserSettings) -> SettingItem {
    SettingItem::new(
        t!("Settings.General.RemoteFileEditor.conflict_check"),
        SettingField::checkbox(
            |cx: &App| {
                AppSettings::global(cx)
                    .remote_file_editor
                    .check_remote_modified_before_upload
            },
            |value, cx: &mut App| {
                AppSettings::update_and_save(cx, |settings| {
                    settings
                        .remote_file_editor
                        .check_remote_modified_before_upload = value;
                });
            },
        )
        .default_value(default_settings.check_remote_modified_before_upload),
    )
    .description(t!("Settings.General.RemoteFileEditor.conflict_check_desc").to_string())
}

fn editor_override_item(editor: RegisteredRemoteFileEditorContribution) -> SettingItem {
    let editor_key = editor.editor_key.clone();
    let editor_key_for_field = editor_key.clone();
    SettingItem::new(
        editor.display_name,
        SettingField::render(move |options, _window, cx| {
            let current = configured_program(&editor_key_for_field, cx);
            let button_label = current.unwrap_or_else(|| {
                t!("Settings.General.RemoteFileEditor.choose_program").to_string()
            });
            let editor_key = editor_key_for_field.clone();
            h_flex().child(
                Button::new(format!("remote-editor-program-{editor_key}"))
                    .label(button_label)
                    .with_size(options.size)
                    .on_click(move |_, window, cx| {
                        prompt_editor_program(editor_key.clone(), window, cx);
                    }),
            )
        }),
    )
    .description(t!("Settings.General.RemoteFileEditor.program_override_desc").to_string())
}

fn configured_program(editor_key: &str, cx: &App) -> Option<String> {
    AppSettings::global(cx)
        .remote_file_editor
        .overrides
        .iter()
        .find(|value| value.editor_key == editor_key)
        .map(|value| value.program.clone())
}

fn prompt_editor_program(editor_key: String, window: &mut Window, cx: &mut App) {
    let future = cx.prompt_for_paths(PathPromptOptions {
        files: true,
        directories: false,
        multiple: false,
        prompt: Some(
            t!("Settings.General.RemoteFileEditor.select_program")
                .to_string()
                .into(),
        ),
    });
    window
        .spawn(cx, async move |cx| {
            let Ok(Ok(Some(paths))) = future.await else {
                return;
            };
            let Some(path) = paths.first() else {
                return;
            };
            let program = path.to_string_lossy().into_owned();
            let _ = cx.update(|_, cx| {
                AppSettings::update_and_save(cx, |settings| {
                    set_program_override(
                        &mut settings.remote_file_editor.overrides,
                        &editor_key,
                        program,
                    );
                });
                cx.refresh_windows();
            });
        })
        .detach();
}

fn set_program_override(
    overrides: &mut Vec<RemoteFileEditorOverride>,
    editor_key: &str,
    program: String,
) {
    if let Some(existing) = overrides
        .iter_mut()
        .find(|value| value.editor_key == editor_key)
    {
        existing.program = program;
        return;
    }
    overrides.push(RemoteFileEditorOverride {
        editor_key: editor_key.to_string(),
        program,
        args: Vec::new(),
    });
}
