//! 连接打开动作：数据库类连接交给领域注册表，未接入时给出可见提示。

use domain_api::{HomeContext as _, NotifyLevel};
use gpui::{App, Window};
use one_core::storage::{ConnectionType, StoredConnection};
use one_core::tab_container::TabOpenMode;

impl super::DataHome {
    /// 打开连接页签；数据库驱动缺失等失败原因用提示区分。
    pub(super) fn open_connection(
        &mut self,
        connection: StoredConnection,
        window: &mut Window,
        cx: &mut App,
    ) {
        let kind = connection.connection_type;
        if !app_kit::registry().open_connection(
            &connection,
            TabOpenMode::Activate,
            self,
            window,
            cx,
        ) {
            let message = match kind {
                ConnectionType::Database => "该数据库连接暂不可用：可能缺少驱动，或连接参数无效",
                _ => "该连接类型尚未接入 navop-data",
            };
            self.notify(NotifyLevel::Warning, message, window, cx);
        }
    }
}
