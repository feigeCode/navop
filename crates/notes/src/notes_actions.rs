use crate::notes_notifications::{notify_error_message, notify_operation_error};
use crate::path_policy::remap_path;
use crate::{DocumentFormat, NotesView, TreeRow};
use gpui::{AppContext, Context, Entity, ParentElement, SharedString, Styled, Window, div, px};
use gpui_component::{
    WindowExt, h_flex,
    input::{Input, InputState},
    v_flex,
};
use rust_i18n::t;
use std::path::Path;
use std::path::PathBuf;

#[derive(Clone, Copy)]
pub(crate) enum CreateKind {
    Directory,
    Document(DocumentFormat),
}

impl NotesView {
    pub(crate) fn start_create_in(
        &mut self,
        directory: PathBuf,
        kind: CreateKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !directory.as_os_str().is_empty() {
            self.tree.expanded_directories.insert(directory.clone());
        }
        self.current_directory = directory;
        self.start_create(kind, window, cx);
    }

    pub(crate) fn start_create(
        &mut self,
        kind: CreateKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let placeholder = match kind {
            CreateKind::Directory => t!("Notes.directory_name").to_string(),
            CreateKind::Document(_) => t!("Notes.document_name").to_string(),
        };
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
        let view = cx.entity();
        open_name_dialog(view, input, kind, window, cx);
    }

    pub(crate) fn start_rename(
        &mut self,
        row: TreeRow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(&row.display_name)
                .placeholder(t!("Notes.new_name").to_string())
        });
        let input_for_focus = input.clone();
        let view = cx.entity();
        window.open_dialog(cx, move |dialog, _window, _cx| {
            let input_for_ok = input.clone();
            let view_for_ok = view.clone();
            let row_for_ok = row.clone();
            dialog
                .title(t!("Notes.rename").to_string())
                .w(px(380.0))
                .confirm()
                .on_ok(move |_, window, cx| {
                    let name = input_for_ok.read(cx).value().trim().to_owned();
                    view_for_ok.update(cx, |view, cx| {
                        view.apply_rename(&row_for_ok, &name, window, cx)
                    });
                    true
                })
                .child(name_dialog_body(
                    t!("Notes.enter_new_name").as_ref(),
                    &input,
                ))
        });
        defer_input_focus(input_for_focus, window, cx);
    }

    pub(crate) fn confirm_delete(
        &mut self,
        row: TreeRow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let view = cx.entity();
        window.open_dialog(cx, move |dialog, _window, _cx| {
            let view_for_ok = view.clone();
            let row_for_ok = row.clone();
            dialog
                .title(t!("Notes.delete").to_string())
                .w(px(380.0))
                .confirm()
                .on_ok(move |_, window, cx| {
                    view_for_ok.update(cx, |view, cx| view.apply_delete(&row_for_ok, window, cx));
                    true
                })
                .child(
                    div().text_sm().child(
                        t!("Notes.delete_confirmation", name = row.display_name).to_string(),
                    ),
                )
        });
    }

    fn apply_create(
        &mut self,
        kind: CreateKind,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let result = match kind {
            CreateKind::Directory => self
                .storage()
                .and_then(|storage| storage.create_directory(&self.current_directory, name))
                .map(|path| {
                    self.tree.expanded_directories.insert(path.clone());
                    self.selected_sidebar_path = Some(path);
                }),
            CreateKind::Document(format) => self
                .storage()
                .and_then(|storage| {
                    storage.create_document_with_format(&self.current_directory, name, format)
                })
                .map(|descriptor| {
                    self.tree.last_created_format = format;
                    self.tree.selected_document = Some(descriptor.relative_path.clone());
                    self.selected_sidebar_path = Some(descriptor.relative_path);
                }),
        };
        self.finish_file_operation(result, window, cx);
    }

    fn apply_rename(
        &mut self,
        row: &TreeRow,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let old_path = row.relative_path.clone();
        let result = self
            .storage()
            .and_then(|storage| storage.rename_node(&old_path, name));
        let Ok(new_path) = result else {
            notify_operation_error(window, cx, result.unwrap_err());
            cx.notify();
            return;
        };
        self.remap_tree_paths(&old_path, &new_path);
        if let Err(error) = self.remap_markdown_sessions(&old_path, &new_path) {
            notify_operation_error(window, cx, error);
        }
        self.finish_file_operation(Ok(()), window, cx);
    }

    fn apply_delete(&mut self, row: &TreeRow, window: &mut Window, cx: &mut Context<Self>) {
        let path = &row.relative_path;
        if self.markdown_sessions.values().any(|session| {
            session.relative_path.starts_with(path)
                && (session.preview.read(cx).is_dirty()
                    || !matches!(
                        session.state.sync_state,
                        crate::markdown_session::MarkdownSyncState::Clean
                    ))
        }) {
            notify_error_message(
                window,
                cx,
                rust_i18n::t!("Notes.unsaved_documents_delete").to_string(),
            );
            cx.notify();
            return;
        }
        let result = self.storage().and_then(|storage| storage.delete_node(path));
        if result.is_ok() {
            self.remove_markdown_sessions_under(path);
            if self
                .tree
                .selected_document
                .as_ref()
                .is_some_and(|selected| selected.starts_with(path))
            {
                self.tree.selected_document = None;
            }
            if self
                .selected_sidebar_path
                .as_ref()
                .is_some_and(|selected| selected.starts_with(path))
            {
                self.selected_sidebar_path = None;
            }
            if self
                .context_menu_path
                .as_ref()
                .is_some_and(|selected| selected.starts_with(path))
            {
                self.context_menu_path = None;
            }
            self.tree
                .expanded_directories
                .retain(|expanded| !expanded.starts_with(path));
            if self.current_directory.starts_with(path) {
                self.current_directory = path.parent().unwrap_or(Path::new("")).to_path_buf();
            }
        }
        self.finish_file_operation(result.map(|_| ()), window, cx);
    }

    fn finish_file_operation(
        &mut self,
        result: anyhow::Result<()>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match result.and_then(|_| self.refresh_tree(window, cx)) {
            Ok(()) => {}
            Err(error) => notify_operation_error(window, cx, error),
        }
        cx.notify();
    }
    fn remap_tree_paths(&mut self, old: &Path, new: &Path) {
        self.selected_sidebar_path = self
            .selected_sidebar_path
            .as_ref()
            .map(|path| remap_path(path, old, new));
        self.context_menu_path = self
            .context_menu_path
            .as_ref()
            .map(|path| remap_path(path, old, new));
        self.tree.selected_document = self
            .tree
            .selected_document
            .as_ref()
            .map(|path| remap_path(path, old, new));
        self.tree.expanded_directories = self
            .tree
            .expanded_directories
            .iter()
            .map(|path| remap_path(path, old, new))
            .collect();
        self.current_directory = remap_path(&self.current_directory, old, new);
    }
}

fn open_name_dialog(
    view: Entity<NotesView>,
    input: Entity<InputState>,
    kind: CreateKind,
    window: &mut Window,
    cx: &mut Context<NotesView>,
) {
    let title = match kind {
        CreateKind::Directory => t!("Notes.new_directory").to_string(),
        CreateKind::Document(DocumentFormat::Markdown) => t!("Notes.new_markdown").to_string(),
    };
    let input_for_focus = input.clone();
    window.open_dialog(cx, move |dialog, _window, _cx| {
        let input_for_ok = input.clone();
        let view_for_ok = view.clone();
        dialog
            .title(title.clone())
            .w(px(380.0))
            .confirm()
            .on_ok(move |_, window, cx| {
                let name = input_for_ok.read(cx).value().trim().to_owned();
                view_for_ok.update(cx, |view, cx| view.apply_create(kind, &name, window, cx));
                true
            })
            .child(name_dialog_body(t!("Notes.enter_name").as_ref(), &input))
    });
    defer_input_focus(input_for_focus, window, cx);
}

fn defer_input_focus(input: Entity<InputState>, window: &mut Window, cx: &mut Context<NotesView>) {
    window.defer(cx, move |window, cx| {
        input.update(cx, |input, cx| input.focus(window, cx));
    });
}

fn name_dialog_body(label: &str, input: &Entity<InputState>) -> impl gpui::IntoElement {
    v_flex()
        .gap_3()
        .child(h_flex().child(SharedString::from(label.to_owned())))
        .child(Input::new(input).w_full())
}
