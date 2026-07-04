mod database_editor_view;
pub mod db_connection_form;
mod generic_database_form;
mod generic_schema_form;
mod generic_user_form;
pub mod manifest_bridge;
mod schema_editor_view;
mod user_editor_view;

pub use database_editor_view::DatabaseEditorView;
pub use generic_database_form::GenericDatabaseForm;
pub use generic_schema_form::GenericSchemaForm;
pub use generic_user_form::GenericUserForm;
pub use schema_editor_view::SchemaEditorView;
pub use user_editor_view::UserEditorView;

use db::plugin::{DatabaseOperationRequest, DatabaseUserOperationRequest};

/// 数据库表单通用事件
/// 所有数据库类型的表单都应该发出这些事件
pub enum DatabaseFormEvent {
    FormChanged(DatabaseOperationRequest),
}

/// 数据库用户表单事件
pub enum DatabaseUserFormEvent {
    FormChanged(DatabaseUserOperationRequest),
}

/// Schema 编辑器请求
#[derive(Clone, Debug)]
pub struct SchemaOperationRequest {
    pub schema_name: String,
    pub comment: Option<String>,
}

/// Schema 表单事件
pub enum SchemaFormEvent {
    FormChanged(SchemaOperationRequest),
}
