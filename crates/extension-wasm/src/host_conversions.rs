use extension_component::{
    ActionContext,
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
