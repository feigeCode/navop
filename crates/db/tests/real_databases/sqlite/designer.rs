use std::path::Path;

use db::connection::DbConnection;
use db::executor::ExecOptions;
use db::plugin::DatabasePlugin;
use db::sqlite::{SqliteDbConnection, SqlitePlugin};
use db::types::{
    ColumnDefinition, ForeignKeyDefinition, IndexDefinition, TableDesign, TableOptions,
};
use one_core::storage::{DatabaseType, DbConnectionConfig};

use crate::real_databases::common::assertions::assert_no_sql_errors;
use crate::real_databases::common::fixture::file_config;

fn config(path: &Path) -> DbConnectionConfig {
    file_config("sqlite-real-designer", DatabaseType::SQLite, path)
}

#[tokio::test]
async fn sqlite_real_table_designer_create_alter_rename_and_export() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let path = temp_dir.path().join("designer.db");
    let plugin = SqlitePlugin::new();
    let mut connection = SqliteDbConnection::new(config(&path));
    connection.connect().await.expect("connect");

    execute(
        &plugin,
        &connection,
        "CREATE TABLE parent (id INTEGER PRIMARY KEY, label TEXT NOT NULL);",
    )
    .await;
    execute(
        &plugin,
        &connection,
        "INSERT INTO parent VALUES (1, 'parent');",
    )
    .await;
    let create = create_design();
    let create_sql = plugin
        .build_create_table_sql_async(&connection, &create)
        .await
        .expect("create SQL");
    execute(&plugin, &connection, &create_sql).await;
    assert_metadata(&plugin, &connection).await;

    let altered = altered_design(&create);
    let alter_sql = plugin
        .build_alter_table_sql_with_renames_async(
            &connection,
            &create,
            &altered,
            &[("label".to_string(), "title".to_string())],
        )
        .await
        .expect("alter SQL");
    execute(&plugin, &connection, &alter_sql).await;
    execute(
        &plugin,
        &connection,
        "INSERT INTO designed (id, title, score) VALUES (1, 'after alter', 91.5);",
    )
    .await;
    let columns = plugin
        .list_columns(&connection, "main", None, "designed")
        .await
        .expect("altered columns");
    assert!(
        columns
            .iter()
            .any(|column| column.name == "title" && column.data_type == "TEXT")
    );
    assert!(
        columns
            .iter()
            .any(|column| column.name == "score" && column.data_type == "REAL")
    );
    assert!(!columns.iter().any(|column| column.name == "label"));
    let indexes = plugin
        .list_indexes(&connection, "main", None, "designed")
        .await
        .expect("altered indexes");
    assert!(
        indexes
            .iter()
            .any(|index| index.name == "idx_designed_title" && index.is_unique)
    );

    let create_export = plugin
        .export_table_create_sql(&connection, "main", None, "designed")
        .await
        .expect("export create");
    assert!(create_export.to_uppercase().contains("CREATE TABLE"));
    let data_export = plugin
        .export_table_data_sql(
            &connection,
            "main",
            None,
            "designed",
            Some("title IS NOT NULL"),
            Some(10),
        )
        .await
        .expect("export data");
    assert!(data_export.contains("INSERT INTO"));
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
                .data_type("TEXT")
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
    design.columns[1].name = "title".into();
    design.columns.push(
        ColumnDefinition::new("score")
            .data_type("REAL")
            .nullable(true),
    );
    design.indexes[0].name = "idx_designed_title".into();
    design.indexes[0].columns = vec!["title".into()];
    design.foreign_keys.push(ForeignKeyDefinition {
        name: "fk_designed_parent".into(),
        columns: vec!["id".into()],
        ref_table: "parent".into(),
        ref_schema: None,
        ref_columns: vec!["id".into()],
        on_delete: "CASCADE".into(),
        on_update: "NO ACTION".into(),
    });
    design
}

async fn execute(plugin: &SqlitePlugin, connection: &SqliteDbConnection, sql: &str) {
    assert_no_sql_errors(
        &connection
            .execute(plugin, sql, ExecOptions::default())
            .await
            .expect("execute"),
        sql,
    );
}

async fn assert_metadata(plugin: &SqlitePlugin, connection: &SqliteDbConnection) {
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
