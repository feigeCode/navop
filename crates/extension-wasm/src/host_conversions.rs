use extension_component::{
    ActionContext, ViewSpec,
    protocol::{Column, ConnectionInfo, DbError, DbValue, ExecOptions, RowBatch},
};

use crate::bindings::onet::extension::{db as Db, ui as Ui};

pub(crate) fn host_exec_options(options: Db::ExecOptions) -> ExecOptions {
    ExecOptions {
        max_rows: options.max_rows,
        timeout_ms: options.timeout_ms,
        stream: options.streaming,
    }
}

pub(crate) fn wit_action_context(context: ActionContext) -> Ui::ActionContext {
    Ui::ActionContext {
        extension_id: context.extension_id,
        command_id: context.command_id,
        node_id: context.node_id,
        node_name: context.node_name,
        node_type: context.node_type,
        database_type: context.database_type,
        connection_id: context.connection_id,
    }
}

pub(crate) fn host_view_spec(view: Ui::ViewSpec) -> ViewSpec {
    ViewSpec {
        id: view.id,
        title: view.title,
        mode: host_view_mode(view.mode),
        nodes: view.nodes.into_iter().map(host_ui_node).collect(),
        actions: view.actions.into_iter().map(host_ui_action).collect(),
        window: view.window.map(host_view_window_options),
    }
}

fn host_view_window_options(
    window: Ui::ViewWindowOptions,
) -> extension_component::ViewWindowOptions {
    extension_component::ViewWindowOptions {
        width: window.width,
        height: window.height,
        min_width: window.min_width,
        min_height: window.min_height,
    }
}

fn host_view_mode(mode: Ui::ViewMode) -> extension_component::ViewMode {
    match mode {
        Ui::ViewMode::Dialog => extension_component::ViewMode::Dialog,
        Ui::ViewMode::Window => extension_component::ViewMode::Window,
    }
}

fn host_ui_node(node: Ui::UiNode) -> extension_component::UiNode {
    match node {
        Ui::UiNode::Text(text) => extension_component::UiNode::Text { text },
        Ui::UiNode::Form(fields) => extension_component::UiNode::Form {
            fields: fields.into_iter().map(host_ui_field).collect(),
        },
    }
}

fn host_ui_field(field: Ui::UiField) -> extension_component::UiField {
    extension_component::UiField {
        id: field.id,
        label: field.label,
        kind: host_field_kind(field.kind),
        required: field.required,
        value: field.value,
        source: field.source.map(host_field_source),
    }
}

fn host_field_kind(kind: Ui::FieldKind) -> extension_component::UiFieldKind {
    match kind {
        Ui::FieldKind::Text => extension_component::UiFieldKind::Text,
        Ui::FieldKind::TextArea => extension_component::UiFieldKind::TextArea,
        Ui::FieldKind::Password => extension_component::UiFieldKind::Password,
        Ui::FieldKind::Checkbox => extension_component::UiFieldKind::Checkbox,
        Ui::FieldKind::Select => extension_component::UiFieldKind::Select,
    }
}

fn host_field_source(source: Ui::FieldSource) -> extension_component::FieldSource {
    match source {
        Ui::FieldSource::StaticOptions(options) => extension_component::FieldSource::StaticOptions(
            options
                .into_iter()
                .map(|option| extension_component::SelectOption {
                    value: option.value,
                    label: option.label,
                })
                .collect(),
        ),
        Ui::FieldSource::DbSelector(source) => {
            extension_component::FieldSource::DbSelector(extension_component::DbSelectorSource {
                kind: host_db_selector_kind(source.kind),
                query: extension_component::DbSelectorQuery {
                    connection_id: source.query.connection_id,
                    database: source.query.database,
                    schema: source.query.schema,
                    table: source.query.table,
                },
            })
        }
    }
}

fn host_db_selector_kind(kind: Ui::DbSelectorKind) -> extension_component::DbSelectorKind {
    match kind {
        Ui::DbSelectorKind::Connection => extension_component::DbSelectorKind::Connection,
        Ui::DbSelectorKind::Database => extension_component::DbSelectorKind::Database,
        Ui::DbSelectorKind::Schema => extension_component::DbSelectorKind::Schema,
        Ui::DbSelectorKind::Table => extension_component::DbSelectorKind::Table,
        Ui::DbSelectorKind::Column => extension_component::DbSelectorKind::Column,
    }
}

fn host_ui_action(action: Ui::UiAction) -> extension_component::UiAction {
    extension_component::UiAction {
        id: action.id,
        label: action.label,
        style: host_action_style(action.style),
    }
}

fn host_action_style(style: Ui::ActionStyle) -> extension_component::UiActionStyle {
    match style {
        Ui::ActionStyle::Primary => extension_component::UiActionStyle::Primary,
        Ui::ActionStyle::Secondary => extension_component::UiActionStyle::Secondary,
        Ui::ActionStyle::Danger => extension_component::UiActionStyle::Danger,
    }
}

pub(crate) fn wit_connection_info(connection: ConnectionInfo) -> Db::ConnectionInfo {
    Db::ConnectionInfo {
        id: connection.id,
        name: connection.name,
        driver: connection.driver,
        database: connection.database,
    }
}

pub(crate) fn wit_db_error(error: DbError) -> Db::DbError {
    Db::DbError {
        code: error.code,
        message: error.message,
    }
}

pub(crate) fn wit_row_batch(batch: RowBatch) -> Db::RowBatch {
    Db::RowBatch {
        columns: batch.columns.into_iter().map(wit_column).collect(),
        rows: batch
            .rows
            .into_iter()
            .map(|row| row.into_iter().map(wit_db_value).collect())
            .collect(),
        next_cursor: batch.next_cursor,
    }
}

fn wit_column(column: Column) -> Db::Column {
    Db::Column {
        name: column.name,
        type_name: column.type_name,
        nullable: column.nullable,
    }
}

fn wit_db_value(value: DbValue) -> Db::DbValue {
    match value {
        DbValue::Null => Db::DbValue::Null,
        DbValue::Bool(value) => Db::DbValue::Boolean(value),
        DbValue::Integer(value) => Db::DbValue::Integer(value),
        DbValue::Float(value) => Db::DbValue::Float(value),
        DbValue::Text(value) => Db::DbValue::Text(value),
        DbValue::Bytes(value) => Db::DbValue::Bytes(value),
    }
}
