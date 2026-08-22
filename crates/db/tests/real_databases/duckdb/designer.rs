use std::path::Path;

use db::connection::DbConnection;
use db::duckdb::{DuckDbConnection, DuckDbPlugin};
use db::executor::ExecOptions;
use db::plugin::DatabasePlugin;
use db::types::{ColumnDefinition, IndexDefinition, TableDesign, TableOptions};
use one_core::storage::{DatabaseType, DbConnectionConfig};

use crate::real_databases::common::assertions::assert_no_sql_errors;
use crate::real_databases::common::fixture::file_config;

fn config(path: &Path) -> DbConnectionConfig {
    file_config("duckdb-real-designer", DatabaseType::DuckDB, path)
}

#[tokio::test]
async fn duckdb_real_table_designer_create_alter_rename_and_export() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let plugin = DuckDbPlugin::new();
    let mut connection = DuckDbConnection::new(config(&temp_dir.path().join("designer.duckdb")));
    connection.connect().await.expect("connect");

    let create = create_design();
    let create_sql = plugin
        .build_create_table_sql_async(&connection, &create)
        .await
        .expect("create");
    execute(&plugin, &connection, &create_sql).await;
    assert_metadata(&plugin, &connection).await;

    let altered = altered_design(&create);
    let alter_sql = plugin.build_alter_table_sql(&create, &altered);
    execute(&plugin, &connection, &alter_sql).await;
    let columns = plugin
        .list_columns(&connection, "main", None, "designed")
        .await
        .expect("columns");
    assert!(columns.iter().any(|column| column.name == "label"));
    assert!(columns.iter().any(|column| column.name == "score"));

    execute(
        &plugin,
        &connection,
        "INSERT INTO designed VALUES (1, 'after alter', 91.5);",
    )
    .await;
    let create_export = plugin
        .export_table_create_sql(&connection, "main", None, "designed")
        .await
        .expect("create export");
    let data_export = plugin
        .export_table_data_sql(
            &connection,
            "main",
            None,
            "designed",
            Some("label IS NOT NULL"),
            Some(10),
        )
        .await
        .expect("data export");
    assert!(create_export.to_uppercase().contains("CREATE TABLE"));
    assert!(data_export.contains("INSERT INTO"));
    assert_column_rename(&plugin, &connection).await;
}

fn create_design() -> TableDesign {
    TableDesign {
        database_name: "main".into(),
        table_name: "designed".into(),
        columns: vec![
            ColumnDefinition::new("id")
                .data_type("INTEGER")
                .nullable(false),
            ColumnDefinition::new("label")
                .data_type("VARCHAR")
                .nullable(false),
        ],
        indexes: vec![IndexDefinition {
            name: "idx_designed_label".into(),
            columns: vec!["label".into()],
            is_unique: true,
            ..Default::default()
        }],
        foreign_keys: vec![],
        options: TableOptions::default(),
    }
}

fn altered_design(original: &TableDesign) -> TableDesign {
    let mut design = original.clone();
    design.columns.push(
        ColumnDefinition::new("score")
            .data_type("DOUBLE")
            .nullable(true),
    );
    design.indexes[0].name = "idx_designed_title".into();
    design
}

async fn execute(plugin: &DuckDbPlugin, connection: &DuckDbConnection, sql: &str) {
    assert_no_sql_errors(
        &connection
            .execute(plugin, sql, ExecOptions::default())
            .await
            .expect("execute"),
        sql,
    );
}

async fn assert_metadata(plugin: &DuckDbPlugin, connection: &DuckDbConnection) {
    let columns = plugin
        .list_columns(connection, "main", None, "designed")
        .await
        .expect("columns");
    assert!(columns.iter().any(|column| column.name == "label"));
    let indexes = plugin
        .list_indexes(connection, "main", None, "designed")
        .await
        .expect("indexes");
    assert!(
        indexes
            .iter()
            .any(|index| index.name == "idx_designed_label" && index.is_unique)
    );
}

async fn assert_column_rename(plugin: &DuckDbPlugin, connection: &DuckDbConnection) {
    execute(plugin, connection, "DROP INDEX idx_designed_title;").await;
    let sql = plugin.build_column_rename_sql("designed", "label", "title", None);
    execute(plugin, connection, &sql).await;
    let columns = plugin
        .list_columns(connection, "main", None, "designed")
        .await
        .expect("renamed columns");
    assert!(columns.iter().any(|column| column.name == "title"));
    assert!(!columns.iter().any(|column| column.name == "label"));
}
