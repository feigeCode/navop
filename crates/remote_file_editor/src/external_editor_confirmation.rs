use gpui::{Context, PromptLevel, Window};
use one_core::settings::{AppSettings, RemoteFileEditorOverride};
use rust_i18n::t;

use crate::external_editor::ExternalEditLaunch;

pub(crate) fn confirm_external_program<T: 'static>(
    launch_request: ExternalEditLaunch,
    window: &mut Window,
    cx: &mut Context<T>,
) {
    let message = t!(
        "RemoteFileEditor.prompt.launch_message",
        program = launch_request.program.clone()
    )
    .to_string();
    let launch = t!("RemoteFileEditor.action.launch").to_string();
    let cancel = t!("RemoteFileEditor.action.cancel").to_string();
    let answer = window.prompt(
        PromptLevel::Warning,
        &t!("RemoteFileEditor.prompt.launch_title"),
        Some(&message),
        &[launch.as_str(), cancel.as_str()],
        cx,
    );
    let entity = cx.entity().clone();
    window
        .spawn(cx, async move |cx| {
            if answer.await.ok() != Some(0) {
                return;
            }
            let _ = entity.update_in(cx, |_this, window, cx| {
                remember_program(&launch_request.editor_key, &launch_request.program, cx);
                launch_request.start(window, cx);
            });
        })
        .detach();
}

fn remember_program(editor_key: &str, program: &str, cx: &mut gpui::App) {
    AppSettings::update_and_save(cx, |settings| {
        if let Some(existing) = settings
            .remote_file_editor
            .overrides
            .iter_mut()
            .find(|value| value.editor_key == editor_key)
        {
            existing.program = program.to_string();
            return;
        }
        settings
            .remote_file_editor
            .overrides
            .push(RemoteFileEditorOverride {
                editor_key: editor_key.to_string(),
                program: program.to_string(),
                args: Vec::new(),
            });
    });
}
