use crate::NotesView;
use crate::notes_notifications::notify_operation_error;
use gpui::{Context, Window};
use std::path::PathBuf;

pub(crate) fn menu_label() -> &'static str {
    if cfg!(target_os = "macos") {
        "在 Finder 中显示"
    } else if cfg!(target_os = "windows") {
        "在资源管理器中显示"
    } else {
        "在文件管理器中显示"
    }
}

impl NotesView {
    pub(crate) fn reveal_in_file_manager(
        &mut self,
        relative_path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self
            .storage()
            .and_then(|storage| storage.absolute_path(&relative_path))
        {
            Ok(path) => cx.reveal_path(&path),
            Err(error) => notify_operation_error(window, cx, error),
        }
    }
}
