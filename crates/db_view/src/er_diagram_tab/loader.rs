use db::{ColumnInfo, ForeignKeyDefinition, GlobalDbState, TableInfo};
use gpui::AsyncApp;

use super::{ErColumnModel, ErForeignKeyModel, ErTableModel};

pub(crate) async fn load_er_tables(
    global_state: GlobalDbState,
    cx: &mut AsyncApp,
    connection_id: String,
    database: String,
    schema: Option<String>,
) -> anyhow::Result<Vec<ErTableModel>> {
    let tables = global_state
        .list_tables(cx, connection_id.clone(), database.clone(), schema.clone())
        .await?;
    let mut models = Vec::with_capacity(tables.len());

    for table in tables {
        let model = load_table_model(
            &global_state,
            cx,
            &connection_id,
            &database,
            schema.clone(),
            table,
        )
        .await?;
        models.push(model);
    }

    Ok(models)
}

async fn load_table_model(
    global_state: &GlobalDbState,
    cx: &mut AsyncApp,
    connection_id: &str,
    database: &str,
    selected_schema: Option<String>,
    table: TableInfo,
) -> anyhow::Result<ErTableModel> {
    let schema = table.schema.clone().or(selected_schema);
    let columns = global_state
        .list_columns(
            cx,
            connection_id.to_string(),
            database.to_string(),
            schema.clone(),
            table.name.clone(),
        )
        .await?;
    let foreign_keys = global_state
        .list_foreign_keys(
            cx,
            connection_id.to_string(),
            database.to_string(),
            schema.clone(),
            table.name.clone(),
        )
        .await
        .unwrap_or_default();

    Ok(ErTableModel {
        name: table.name,
        schema,
        columns: columns.into_iter().map(column_model).collect(),
        foreign_keys: foreign_keys.into_iter().map(foreign_key_model).collect(),
    })
}

fn column_model(column: ColumnInfo) -> ErColumnModel {
    ErColumnModel {
        name: column.name,
        data_type: column.data_type,
        is_primary_key: column.is_primary_key,
        is_nullable: column.is_nullable,
    }
}

fn foreign_key_model(fk: ForeignKeyDefinition) -> ErForeignKeyModel {
    ErForeignKeyModel {
        name: fk.name,
        columns: fk.columns,
        ref_table: fk.ref_table,
        ref_columns: fk.ref_columns,
    }
}
