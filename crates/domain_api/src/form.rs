use crate::kind::ConnectionKindDescriptor;
use gpui::{AnyView, App, SharedString, Window};
use one_core::storage::StoredConnection;
use std::sync::Arc;

/// 表单保存后的回调。表单自身已经写入连接仓库，宿主只需要刷新列表。
pub type ConnectionSavedCallback =
    Arc<dyn Fn(&StoredConnection, &mut Window, &mut App) + Send + Sync>;

/// 构造连接表单需要的上下文。
#[derive(Clone, Default)]
pub struct ConnectionFormRequest {
    pub kind_id: SharedString,
    /// 编辑已有连接时提供。
    pub editing_connection: Option<StoredConnection>,
    /// 复制已有连接时提供（不写入 id，保存后是新连接）。
    pub initial_connection: Option<StoredConnection>,
    pub initial_workspace_id: Option<i64>,
    pub on_saved: Option<ConnectionSavedCallback>,
}

impl ConnectionFormRequest {
    pub fn new(kind_id: impl Into<SharedString>) -> Self {
        Self {
            kind_id: kind_id.into(),
            ..Self::default()
        }
    }

    pub fn editing(kind_id: impl Into<SharedString>, connection: StoredConnection) -> Self {
        Self {
            editing_connection: Some(connection),
            ..Self::new(kind_id)
        }
    }

    pub fn duplicate(kind_id: impl Into<SharedString>, connection: StoredConnection) -> Self {
        Self {
            initial_connection: Some(connection),
            ..Self::new(kind_id)
        }
    }

    pub fn with_workspace(mut self, workspace_id: Option<i64>) -> Self {
        self.initial_workspace_id = workspace_id;
        self
    }

    pub fn with_saved_callback(mut self, callback: ConnectionSavedCallback) -> Self {
        self.on_saved = Some(callback);
        self
    }

    pub fn is_new(&self) -> bool {
        self.editing_connection.is_none()
    }
}

/// 领域提供的连接表单：既能声明支持哪些连接种类，也能按请求构造表单视图。
pub trait ConnectionFormProvider: Send + Sync {
    /// 本领域支持的连接种类。
    fn kinds(&self, cx: &App) -> Vec<ConnectionKindDescriptor>;

    /// 已有连接对应的种类 ID；编辑/复制时用来预选表单。
    fn kind_for(&self, _connection: &StoredConnection, _cx: &App) -> Option<SharedString> {
        None
    }

    /// 构造表单视图；`request.kind_id` 不属于本领域时返回 `None`。
    fn build(
        &self,
        request: ConnectionFormRequest,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<AnyView>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_request_is_new() {
        let request = ConnectionFormRequest::new("ssh");
        assert!(request.is_new());
        assert_eq!("ssh", request.kind_id.as_ref());
        assert_eq!(None, request.initial_workspace_id);
    }

    #[test]
    fn workspace_and_callback_are_kept() {
        let request = ConnectionFormRequest::new("redis")
            .with_workspace(Some(7))
            .with_saved_callback(Arc::new(|_, _, _| {}));
        assert_eq!(Some(7), request.initial_workspace_id);
        assert!(request.on_saved.is_some());
    }

    #[test]
    fn request_clone_keeps_kind() {
        let request = ConnectionFormRequest::new("mongodb");
        let cloned = request.clone();
        assert_eq!("mongodb", cloned.kind_id.as_ref());
    }
}
