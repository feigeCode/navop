use db::connection::DbConnection;
use db::mysql::MySqlPlugin;
use db::plugin::DatabasePlugin;
use db::types::{
    ColumnDefinition, ForeignKeyDefinition, IndexDefinition, TableDesign, TableOptions,
};

use crate::real_databases::common::env::{mysql_config, skip_database};
use crate::real_databases::mysql::core_flow::{drop_database, execute, unique_database};

#[tokio::test]
async fn mysql_real_table_designer_create_alter_rename_and_export() {
    let Some(config) = mysql_config() else {
        skip_database("MySQL", "ONETCLI_TEST_MYSQL_PASSWORD");
        return;
    };
    let database = unique_database("design");
    let plugin = MySqlPlugin::new();
    let mut connection = plugin
        .create_connection(config)
        .await
        .expect("MySQL should connect");
    create_database(&plugin, connection.as_ref(), &database).await;

    let create_sql = plugin
        .build_create_table_sql(&create_design(&database))
        .to_uppercase();
    assert!(create_sql.contains("CREATE TABLE"));
    assert!(create_sql.contains("AUTO_INCREMENT"));
    execute(
        &plugin,
        connection.as_ref(),
        &format!(
            "USE `{database}`; {}",
            plugin.build_create_table_sql(&create_design(&database))
        ),
    )
    .await;
    assert_created_metadata(&plugin, connection.as_ref(), &database).await;

    let original = create_design(&database);
    let altered = altered_design(&original);
    let alter_sql = plugin
        .build_alter_table_sql_with_renames(
            &original,
            &altered,
            &[("label".to_string(), "title".to_string())],
        )
        .to_uppercase();
    assert!(alter_sql.contains("DROP INDEX"));
    assert!(alter_sql.contains("ADD"));
    assert!(alter_sql.contains("CHANGE"));
    assert!(alter_sql.contains("FOREIGN KEY"));
    execute(
        &plugin,
        connection.as_ref(),
        &plugin.build_alter_table_sql_with_renames(
            &original,
            &altered,
            &[("label".to_string(), "title".to_string())],
        ),
    )
    .await;
    assert_altered_metadata(&plugin, connection.as_ref(), &database).await;
    assert_exports(&plugin, connection.as_ref(), &database).await;

    drop_database(&plugin, connection.as_ref(), &database).await;
    connection.disconnect().await.expect("disconnect");
}

async fn create_database(
    plugin: &MySqlPlugin,
    connection: &(dyn DbConnection + Send + Sync),
    database: &str,
) {
    execute(
        plugin,
        connection,
        &format!(
            "CREATE DATABASE `{database}` CHARACTER SET utf8mb4; \
             CREATE TABLE `{database}`.parent (id INT PRIMARY KEY);"
        ),
    )
    .await;
}

fn create_design(database: &str) -> TableDesign {
    TableDesign {
        database_name: database.to_string(),
        table_name: "designed".into(),
        columns: vec![
            ColumnDefinition::new("id")
                .data_type("INT")
                .nullable(false)
                .primary_key(true)
                .auto_increment(true),
            ColumnDefinition::new("label")
                .data_type("VARCHAR")
                .length(64)
                .nullable(false),
        ],
        indexes: vec![
            IndexDefinition::new("idx_designed_label")
                .columns(vec!["label".into()])
                .unique(true),
        ],
        foreign_keys: vec![],
        options: TableOptions {
            engine: Some("InnoDB".into()),
            charset: Some("utf8mb4".into()),
            ..Default::default()
        },
    }
}

fn altered_design(original: &TableDesign) -> TableDesign {
    let mut design = original.clone();
    design.columns[1].name = "title".into();
    design.columns[1].length = Some(100);
    design.columns.push(
        ColumnDefinition::new("score")
            .data_type("DOUBLE")
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

async fn assert_created_metadata(
    plugin: &MySqlPlugin,
    connection: &(dyn DbConnection + Send + Sync),
    database: &str,
) {
    let columns = plugin
        .list_columns(connection, database, None, "designed")
        .await
        .expect("created columns");
    assert!(columns.iter().any(|column| column.name == "label"));
    let indexes = plugin
        .list_indexes(connection, database, None, "designed")
        .await
        .expect("created indexes");
    assert!(
        indexes
            .iter()
            .any(|index| index.name == "idx_designed_label" && index.is_unique)
    );
}

async fn assert_altered_metadata(
    plugin: &MySqlPlugin,
    connection: &(dyn DbConnection + Send + Sync),
    database: &str,
) {
    let columns = plugin
        .list_columns(connection, database, None, "designed")
        .await
        .expect("altered columns");
    assert!(
        columns
            .iter()
            .any(|column| column.name == "title" && column.data_type.contains("char"))
    );
    assert!(
        columns
            .iter()
            .any(|column| column.name == "score" && column.data_type == "double")
    );
    assert!(!columns.iter().any(|column| column.name == "label"));
    let indexes = plugin
        .list_indexes(connection, database, None, "designed")
        .await
        .expect("altered indexes");
    assert!(
        indexes
            .iter()
            .any(|index| index.name == "idx_designed_title" && index.is_unique)
    );
    let foreign_keys = plugin
        .list_foreign_keys(connection, database, None, "designed")
        .await
        .expect("altered foreign keys");
    assert!(
        foreign_keys
            .iter()
            .any(|foreign_key| foreign_key.name == "fk_designed_parent")
    );
}

async fn assert_exports(
    plugin: &MySqlPlugin,
    connection: &(dyn DbConnection + Send + Sync),
    database: &str,
) {
    let create = plugin
        .export_table_create_sql(connection, database, None, "designed")
        .await
        .expect("MySQL create export");
    assert!(create.to_uppercase().contains("CREATE TABLE"));
    let data = plugin
        .export_table_data_sql(connection, database, None, "designed", None, Some(10))
        .await
        .expect("MySQL data export");
    assert!(
        data.trim().is_empty(),
        "empty table should export no rows: {data}"
    );
}
