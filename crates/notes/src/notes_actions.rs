use crate::{NotesView, TreeRow};
use gpui::{AppContext, Context, Entity, ParentElement, SharedString, Styled, Window, div, px};
use gpui_component::{
    WindowExt, h_flex,
    input::{Input, InputEvent, InputState},
    v_flex,
};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy)]
pub(crate) enum CreateKind {
    Directory,
    Document,
}

impl NotesView {
    pub(crate) fn start_create(
        &mut self,
        kind: CreateKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let placeholder = match kind {
            CreateKind::Directory => "目录名称",
            CreateKind::Document => "文档名称",
        };
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
        self.dialog_subscription = Some(cx.subscribe_in(
            &input,
            window,
            move |view, input, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    let name = input.read(cx).value().trim().to_owned();
                    window.close_dialog(cx);
                    view.apply_create(kind, &name, cx);
                }
            },
        ));
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
                .placeholder("新名称")
        });
        let input_for_focus = input.clone();
        let row_for_enter = row.clone();
        self.dialog_subscription = Some(cx.subscribe_in(
            &input,
            window,
            move |view, input, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    let name = input.read(cx).value().trim().to_owned();
                    window.close_dialog(cx);
                    view.apply_rename(&row_for_enter, &name, cx);
                }
            },
        ));
        let view = cx.entity();
        window.open_dialog(cx, move |dialog, _window, _cx| {
            let input_for_ok = input.clone();
            let view_for_ok = view.clone();
            let row_for_ok = row.clone();
            dialog
                .title("重命名")
                .w(px(380.0))
                .confirm()
                .on_ok(move |_, _, cx| {
                    let name = input_for_ok.read(cx).value().trim().to_owned();
                    view_for_ok.update(cx, |view, cx| view.apply_rename(&row_for_ok, &name, cx));
                    true
                })
                .child(name_dialog_body("输入新的名称", &input))
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
                .title("删除")
                .w(px(380.0))
                .confirm()
                .on_ok(move |_, _, cx| {
                    view_for_ok.update(cx, |view, cx| view.apply_delete(&row_for_ok, cx));
                    true
                })
                .child(div().text_sm().child(format!(
                    "确定删除「{}」？目录会被递归删除，此操作不可撤销。",
                    row.display_name
                )))
        });
    }

    fn apply_create(&mut self, kind: CreateKind, name: &str, cx: &mut Context<Self>) {
        let result = match kind {
            CreateKind::Directory => self
                .storage()
                .and_then(|storage| storage.create_directory(&self.current_directory, name))
                .map(|path| {
                    self.tree.expanded_directories.insert(path);
                }),
            CreateKind::Document => self
                .storage()
                .and_then(|storage| storage.create_document(&self.current_directory, name))
                .map(|descriptor| {
                    self.tree.selected_document = Some(descriptor.relative_path);
                }),
        };
        self.finish_file_operation(result, cx);
    }

    fn apply_rename(&mut self, row: &TreeRow, name: &str, cx: &mut Context<Self>) {
        let old_path = row.relative_path.clone();
        let result = self
            .storage()
            .and_then(|storage| storage.rename_node(&old_path, name));
        let Ok(new_path) = result else {
            self.set_error(result.unwrap_err());
            cx.notify();
            return;
        };
        self.remap_tree_paths(&old_path, &new_path);
        if let Err(error) = self.remap_cached_editors(&old_path, &new_path) {
            self.set_error(error);
        }
        self.finish_file_operation(Ok(()), cx);
    }

    fn apply_delete(&mut self, row: &TreeRow, cx: &mut Context<Self>) {
        let path = &row.relative_path;
        if self
            .editors
            .values()
            .any(|cached| cached.relative_path.starts_with(path) && cached.handle.is_dirty(cx))
        {
            self.set_error("包含未保存文档，请等待自动保存完成后再删除");
            cx.notify();
            return;
        }
        let result = self.storage().and_then(|storage| storage.delete_node(path));
        if result.is_ok() {
            self.editors
                .retain(|_, cached| !cached.relative_path.starts_with(path));
            if self
                .tree
                .selected_document
                .as_ref()
                .is_some_and(|selected| selected.starts_with(path))
            {
                self.tree.selected_document = None;
            }
            self.tree
                .expanded_directories
                .retain(|expanded| !expanded.starts_with(path));
            if self.current_directory.starts_with(path) {
                self.current_directory = path.parent().unwrap_or(Path::new("")).to_path_buf();
            }
        }
        self.finish_file_operation(result.map(|_| ()), cx);
    }

    fn finish_file_operation(&mut self, result: anyhow::Result<()>, cx: &mut Context<Self>) {
        match result.and_then(|_| self.refresh_tree(cx)) {
            Ok(()) => self.error = None,
            Err(error) => self.set_error(error),
        }
        cx.notify();
    }

    fn remap_tree_paths(&mut self, old: &Path, new: &Path) {
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

    fn remap_cached_editors(&mut self, old: &Path, new: &Path) -> anyhow::Result<()> {
        let updates = self
            .editors
            .iter()
            .filter(|(_, cached)| cached.relative_path.starts_with(old))
            .map(|(id, cached)| (id.clone(), remap_path(&cached.relative_path, old, new)))
            .collect::<Vec<_>>();
        for (id, relative_path) in updates {
            let absolute_path = self.storage()?.descriptor(&relative_path)?.absolute_path;
            if let Some(cached) = self.editors.get_mut(&id) {
                cached.persistence.set_path(absolute_path)?;
                cached.relative_path = relative_path;
            }
        }
        Ok(())
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
        CreateKind::Directory => "新建目录",
        CreateKind::Document => "新建文档",
    };
    let input_for_focus = input.clone();
    window.open_dialog(cx, move |dialog, _window, _cx| {
        let input_for_ok = input.clone();
        let view_for_ok = view.clone();
        dialog
            .title(title)
            .w(px(380.0))
            .confirm()
            .on_ok(move |_, _, cx| {
                let name = input_for_ok.read(cx).value().trim().to_owned();
                view_for_ok.update(cx, |view, cx| view.apply_create(kind, &name, cx));
                true
            })
            .child(name_dialog_body("请输入名称", &input))
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

fn remap_path(path: &Path, old: &Path, new: &Path) -> PathBuf {
    path.strip_prefix(old)
        .map(|suffix| new.join(suffix))
        .unwrap_or_else(|_| path.to_path_buf())
}
