use crate::{DocumentFormat, NotesView, TreeRow};
use gpui::{Context, Window};
use gpui_component::{
    IconName, Sizable,
    button::{Button, ButtonVariants},
};

impl NotesView {
    pub(crate) fn render_convert_button(&self, row: &TreeRow, cx: &mut Context<Self>) -> Button {
        let row = row.clone();
        Button::new(format!("convert-note-{}", row.relative_path.display()))
            .icon(IconName::Copy)
            .ghost()
            .xsmall()
            .tooltip("转换为 Markdown（保留原富文本文档）")
            .on_click(
                cx.listener(move |view, _, window, cx| view.convert_to_markdown(&row, window, cx)),
            )
    }

    fn convert_to_markdown(&mut self, row: &TreeRow, window: &mut Window, cx: &mut Context<Self>) {
        if row.format != Some(DocumentFormat::RichText) {
            return;
        }
        if self
            .editors
            .values()
            .any(|cached| cached.relative_path == row.relative_path && cached.handle.is_dirty(cx))
        {
            self.set_error("富文本文档尚未保存，请等待自动保存完成后再转换");
            cx.notify();
            return;
        }
        let result = self
            .storage()
            .and_then(|storage| storage.convert_rich_text_to_markdown(&row.relative_path));
        match result {
            Ok(descriptor) => {
                self.tree.selected_document = Some(descriptor.relative_path);
                if let Err(error) = self.refresh_tree(window, cx) {
                    self.set_error(error);
                } else {
                    self.error = None;
                }
            }
            Err(error) => self.set_error(error),
        }
        cx.notify();
    }
}
