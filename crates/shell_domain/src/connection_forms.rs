//! 独立产品的连接表单提供方。
//!
//! 提供终端与文件、远程桌面与端口转发两组共 7 种连接的新建/编辑表单，
//! 让独立产品不再只能浏览已有连接。

mod forms;
mod kinds;

use domain_api::{ConnectionFormProvider, ConnectionFormRequest, ConnectionKindDescriptor};
use gpui::{AnyView, App, SharedString, Window};
use one_core::storage::StoredConnection;

/// Shell 产品注册到领域注册表的连接表单入口。
pub(crate) struct ShellConnectionFormProvider;

impl ConnectionFormProvider for ShellConnectionFormProvider {
    fn kinds(&self, _cx: &App) -> Vec<ConnectionKindDescriptor> {
        kinds::descriptors()
    }

    fn kind_for(&self, connection: &StoredConnection, _cx: &App) -> Option<SharedString> {
        kinds::kind_id_for_type(connection.connection_type).map(SharedString::from)
    }

    fn build(
        &self,
        request: ConnectionFormRequest,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<AnyView> {
        forms::build(request, window, cx)
    }
}
