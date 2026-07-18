use crate::notes_notifications::{notify_error_message, notify_operation_error};
use crate::{DocumentFormat, NotesView, TreeRow};
use gpui::{Context, Window};

impl NotesView {
    pub(crate) fn convert_to_markdown(
        &mut self,
        row: &TreeRow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if row.format != Some(DocumentFormat::RichText) {
            return;
        }
        if self
            .editors
            .values()
            .any(|cached| cached.relative_path == row.relative_path && cached.handle.is_dirty(cx))
        {
            notify_error_message(
                window,
                cx,
                rust_i18n::t!("Notes.unsaved_rich_text_conversion").to_string(),
            );
            cx.notify();
            return;
        }
        let result = self
            .storage()
            .and_then(|storage| storage.convert_rich_text_to_markdown(&row.relative_path));
        match result {
            Ok(descriptor) => {
                self.tree.selected_document = Some(descriptor.relative_path.clone());
                self.selected_sidebar_path = Some(descriptor.relative_path);
                if let Err(error) = self.refresh_tree(window, cx) {
                    notify_operation_error(window, cx, error);
                }
            }
            Err(error) => notify_operation_error(window, cx, error),
        }
        cx.notify();
    }
}
