use domain_api::{ConnectionFormRequest, ConnectionSavedCallback};
use gpui::{Context, Window};
use gpui_component::WindowExt as _;
use one_core::storage::StoredConnection;
use std::sync::Arc;

use super::ConnectionImportWindow;

impl ConnectionImportWindow {
    pub(crate) fn edit_row(
        &mut self,
        record_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(draft) = self.model.draft(&record_id) else {
            return;
        };
        let connection = match draft.to_editor_connection() {
            Ok(connection) => connection,
            Err(error) => {
                self.model.mark_failed(&record_id, error);
                cx.notify();
                return;
            }
        };
        self.open_editor_for_connection(record_id, connection, window, cx);
    }

    /// 草稿编辑复用产品统一的连接表单：表单按连接类型由领域注册表提供，
    /// 保存后回调把草稿标记为已保存。
    fn open_editor_for_connection(
        &self,
        record_id: String,
        connection: StoredConnection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(kind_id) = app_kit::registry().kind_for_connection(&connection, cx) else {
            window.push_notification(
                gpui_component::notification::Notification::error(format!(
                    "当前产品不支持编辑该连接：{}（{}）",
                    connection.name, connection.connection_type
                )),
                cx,
            );
            return;
        };
        let import_window = cx.entity();
        let saved_record_id = record_id.clone();
        let on_saved: ConnectionSavedCallback = Arc::new(move |saved, _window, cx| {
            let record_id = saved_record_id.clone();
            let connection_id = saved.id;
            let import_window = import_window.clone();
            let _ = import_window.update(cx, |this, cx| {
                this.handle_editor_saved(record_id, connection_id, false, cx);
            });
        });
        let request = ConnectionFormRequest {
            initial_connection: Some(connection),
            ..ConnectionFormRequest::new(kind_id).with_saved_callback(on_saved)
        };
        if !app_kit::connection_actions::open_form(request, window, cx) {
            cx.notify();
            return;
        }
        cx.notify();
    }

    fn handle_editor_saved(
        &mut self,
        record_id: String,
        connection_id: Option<i64>,
        continue_to_next: bool,
        cx: &mut Context<Self>,
    ) {
        self.model.mark_saved(&record_id, connection_id);
        if continue_to_next
            && let Some(next_record_id) = self.model.next_save_candidate_row_id_after(&record_id)
        {
            // 保存并继续需要窗口句柄；导入编辑器暂不提供该入口。
            let _ = next_record_id;
        }
        cx.notify();
    }
}
