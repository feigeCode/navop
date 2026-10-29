//! 新建 / 编辑 / 复制 / 删除连接的统一入口。
//!
//! 三个产品共用同一套表单与确认弹窗：表单由领域通过
//! [`domain_api::ConnectionFormProvider`] 提供，本模块只负责选择种类、
//! 打开可复用弹窗，并在保存后广播连接变更事件让宿主列表刷新。

use gpui::{App, ParentElement as _, Window};
use gpui_component::WindowExt as _;
use gpui_component::notification::Notification;
use one_core::connection_notifier::{ConnectionDataEvent, emit_connection_event_from_app};
use one_core::storage::traits::Repository as _;
use one_core::storage::{ConnectionRepository, GlobalStorageState, StoredConnection};

pub use crate::new_connection::{
    ConnectionFormDialog, ConnectionFormMode, duplicate_connection, edit_connection,
    open_new_connection, open_new_connection_in_workspace,
};

/// 删除连接（带确认弹窗）。
///
/// 只处理本地仓库：独立产品默认不接云同步，云端副本由全量产品维护。
pub fn delete_connection(connection: StoredConnection, window: &mut Window, cx: &mut App) {
    let name = connection.name.clone();
    window.open_dialog(cx, move |dialog, _window, _cx| {
        let confirm = connection.clone();
        dialog
            .title("删除连接")
            .child(format!("确定删除「{name}」吗？此操作不可撤销。"))
            .confirm()
            .on_ok(
                move |_, window, cx| match remove_local_connection(&confirm, cx) {
                    Ok(()) => {
                        emit_connection_event_from_app(
                            ConnectionDataEvent::ConnectionDeleted {
                                connection_id: confirm.id.unwrap_or_default(),
                                cloud_id: confirm.cloud_id.clone(),
                                last_synced_at: confirm.last_synced_at,
                            },
                            cx,
                        );
                        window.push_notification(
                            Notification::success(format!("已删除「{}」", confirm.name)),
                            cx,
                        );
                        true
                    }
                    Err(error) => {
                        window.push_notification(
                            Notification::error(format!("删除失败：{error:#}")),
                            cx,
                        );
                        false
                    }
                },
            )
    });
}

fn remove_local_connection(connection: &StoredConnection, cx: &mut App) -> anyhow::Result<()> {
    let id = connection
        .id
        .ok_or_else(|| anyhow::anyhow!("连接尚未保存，无法删除"))?;
    let storage = cx.global::<GlobalStorageState>().storage.clone();
    let repository = storage
        .get::<ConnectionRepository>()
        .ok_or_else(|| anyhow::anyhow!("连接仓库未初始化"))?;
    repository
        .delete(id)
        .map_err(|error| anyhow::anyhow!("{error}"))
}
