use crate::notes_notifications::{notify_error_message, notify_operation_error};
use crate::notes_view::NotesLoadState;
use crate::{NotebookMetadata, NotesStorage, NotesView, TreeState};
use anyhow::Result;
use gpui::{Context, Entity, IntoElement, ParentElement, PathPromptOptions, Styled, Window, px};
use gpui_component::{
    IconName, WindowExt,
    button::Button,
    h_flex,
    input::{Input, InputState},
    v_flex,
};
use rust_i18n::t;
use std::path::PathBuf;

const DEFAULT_NOTEBOOK_NAME: &str = "Notes";

struct OpenedNotes {
    storage: NotesStorage,
    metadata: NotebookMetadata,
}

struct LocationDialogState {
    view: Entity<NotesView>,
    input: Entity<InputState>,
}

impl NotesView {
    pub(crate) fn initialize_configured_notes(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<bool> {
        if !NotesStorage::has_configured_root()? {
            return Ok(false);
        }
        let root = NotesStorage::configured_root()?;
        let opened = open_or_initialize_notes(root)?;
        self.finish_location_setup(opened, window, cx)?;
        Ok(true)
    }

    pub(crate) fn show_location_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        open_location_dialog(
            LocationDialogState {
                view: cx.entity(),
                input: self.setup_path.clone(),
            },
            window,
            cx,
        );
    }

    fn confirm_location(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let path = self.setup_path.read(cx).value().trim().to_owned();
        if path.is_empty() {
            notify_error_message(window, cx, t!("Notes.notebook_path_required").to_string());
            return false;
        }
        let result = self.activate_location(PathBuf::from(path), window, cx);
        if let Err(error) = result {
            notify_operation_error(window, cx, error);
            return false;
        }
        true
    }

    fn activate_location(
        &mut self,
        root: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        let opened = open_or_initialize_notes(root.clone())?;
        NotesStorage::save_configured_root(&root)?;
        self.finish_location_setup(opened, window, cx)
    }

    fn finish_location_setup(
        &mut self,
        opened: OpenedNotes,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        self.tree = TreeState::from_ui_state(opened.storage.load_state()?);
        self.storage = Some(opened.storage);
        self.notebook_name = opened.metadata.name.into();
        self.load_state = NotesLoadState::Ready;
        self.refresh_tree(window, cx)?;
        cx.notify();
        Ok(())
    }
}

pub(crate) fn defer_location_dialog(
    input: Entity<InputState>,
    window: &mut Window,
    cx: &mut Context<NotesView>,
) {
    let state = LocationDialogState {
        view: cx.entity(),
        input,
    };
    window.defer(cx, move |window, cx| {
        open_location_dialog(state, window, cx);
    });
}

fn open_location_dialog(state: LocationDialogState, window: &mut Window, cx: &mut gpui::App) {
    window.open_dialog(cx, move |dialog, _window, _cx| {
        let view_for_ok = state.view.clone();
        dialog
            .title(t!("Notes.location_dialog_title").to_string())
            .w(px(560.0))
            .confirm()
            .on_ok(move |_, window, cx| {
                view_for_ok.update(cx, |view, cx| view.confirm_location(window, cx))
            })
            .child(location_dialog_body(state.input.clone()))
    });
}

fn location_dialog_body(input: Entity<InputState>) -> impl IntoElement {
    let input_for_select = input.clone();
    v_flex()
        .gap_3()
        .child(t!("Notes.location_dialog_description").to_string())
        .child(
            h_flex()
                .w_full()
                .gap_2()
                .child(Input::new(&input).flex_1().disabled(true))
                .child(
                    Button::new("select_notes_location")
                        .icon(IconName::FolderOpenColor.color())
                        .label(t!("Notes.select_path").to_string())
                        .on_click(move |_, window, cx| {
                            prompt_for_location(input_for_select.clone(), window, cx);
                        }),
                ),
        )
}

fn prompt_for_location(input: Entity<InputState>, window: &mut Window, cx: &mut gpui::App) {
    let future = cx.prompt_for_paths(PathPromptOptions {
        files: false,
        directories: true,
        multiple: false,
        prompt: Some(t!("Notes.select_notebook_directory").to_string().into()),
    });
    window
        .spawn(cx, async move |cx| {
            let Ok(Ok(Some(paths))) = future.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let value = path.to_string_lossy().into_owned();
            let _ = cx.update(|window, cx| {
                input.update(cx, |input, cx| input.set_value(value, window, cx));
            });
        })
        .detach();
}

fn open_or_initialize_notes(root: PathBuf) -> Result<OpenedNotes> {
    let storage = NotesStorage::open(root)?;
    let metadata = match storage.load_notebook()? {
        Some(metadata) => metadata,
        None => storage.create_notebook(DEFAULT_NOTEBOOK_NAME, "")?,
    };
    Ok(OpenedNotes { storage, metadata })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opening_location_initializes_once_and_preserves_metadata() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let root = temp.path().join("notes");

        let created = open_or_initialize_notes(root.clone())?;
        assert_eq!(DEFAULT_NOTEBOOK_NAME, created.metadata.name);
        assert_eq!(
            Some(created.metadata.clone()),
            created.storage.load_notebook()?
        );

        let reopened = open_or_initialize_notes(root)?;
        assert_eq!(created.metadata, reopened.metadata);
        Ok(())
    }
}
