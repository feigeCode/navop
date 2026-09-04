use db::connection::DbConnection;
use db::plugin::DatabasePlugin;
use db::postgresql::PostgresPlugin;
use db::types::{ColumnDefinition, ForeignKeyDefinition, IndexDefinition, TableDesign};

use crate::real_databases::common::env::{optional_database, postgres_config, skip_database};
use crate::real_databases::postgres::core_flow::{
    drop_schema, execute, reset_schema, unique_schema,
};

#[tokio::test]
async fn postgres_real_table_designer_create_alter_rename_and_export() {
    let Some(config) = postgres_config() else {
        skip_database("PostgreSQL", "ONETCLI_TEST_POSTGRES_PASSWORD (empty string is valid)");
        return;
    };
    let config = optional_database(
        &config,
        &std::env::var("ONETCLI_TEST_POSTGRES_DATABASE").unwrap_or_else(|_| "postgres".to_string()),
    );
    let schema = unique_schema("design");
    let plugin = PostgresPlugin::new();
    let mut connection = plugin
        .create_connection(config)
        .await
        .expect("PostgreSQL should connect");
    reset_schema(&plugin, connection.as_ref(), &schema).await;
    execute(
        &plugin,
        connection.as_ref(),
        &format!("CREATE TABLE \"{schema}\".parent (id INT PRIMARY KEY);"),
    )
    .await;

    let original = create_design();
    let create_sql = plugin.build_create_table_sql(&original).to_uppercase();
    assert!(create_sql.contains("CREATE TABLE"));
    assert!(create_sql.contains("SERIAL"));
    execute(
        &plugin,
        connection.as_ref(),
        &format!(
            "SET search_path TO \"{schema}\"; {}",
            plugin.build_create_table_sql(&original)
        ),
    )
    .await;
    assert_created_metadata(&plugin, connection.as_ref(), &schema).await;

    let altered = altered_design(&original);
    let sql = plugin.build_alter_table_sql_with_renames(
        &original,
        &altered,
        &[("label".to_string(), "title".to_string())],
    );
    let upper = sql.to_uppercase();
    assert!(upper.contains("DROP INDEX"));
    assert!(upper.contains("ADD COLUMN"));
    assert!(upper.contains("RENAME COLUMN"));
    assert!(upper.contains("ADD CONSTRAINT"));
    execute(&plugin, connection.as_ref(), &sql).await;
    assert_altered_metadata(&plugin, connection.as_ref(), &schema).await;
    assert_exports(&plugin, connection.as_ref(), &schema).await;

    drop_schema(&plugin, connection.as_ref(), &schema).await;
    connection.disconnect().await.expect("disconnect");
}

fn create_design() -> TableDesign {
    TableDesign {
        database_name: "postgres".into(),
        table_name: "designed".into(),
        columns: vec![
            ColumnDefinition::new("id")
                .data_type("SERIAL")
                .nullable(false)
                .primary_key(true),
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
        options: Default::default(),
    }
}

fn altered_design(original: &TableDesign) -> TableDesign {
    let mut design = original.clone();
    design.columns[1].name = "title".into();
    design.columns[1].length = Some(100);
    design.columns.push(
        ColumnDefinition::new("score")
            .data_type("DOUBLE PRECISION")
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
    plugin: &PostgresPlugin,
    connection: &(dyn DbConnection + Send + Sync),
    schema: &str,
) {
    let columns = plugin
        .list_columns(connection, "postgres", Some(schema.to_string()), "designed")
        .await
        .expect("created columns");
    assert!(columns.iter().any(|column| column.name == "label"));
    let indexes = plugin
        .list_indexes(connection, "postgres", Some(schema.to_string()), "designed")
        .await
        .expect("created indexes");
    assert!(
        indexes
            .iter()
            .any(|index| index.name == "idx_designed_label" && index.is_unique)
    );
}

async fn assert_altered_metadata(
    plugin: &PostgresPlugin,
    connection: &(dyn DbConnection + Send + Sync),
    schema: &str,
) {
    let columns = plugin
        .list_columns(connection, "postgres", Some(schema.to_string()), "designed")
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
            .any(|column| column.name == "score" && column.data_type == "float8")
    );
    assert!(!columns.iter().any(|column| column.name == "label"));
    let indexes = plugin
        .list_indexes(connection, "postgres", Some(schema.to_string()), "designed")
        .await
        .expect("altered indexes");
    assert!(
        indexes
            .iter()
            .any(|index| index.name == "idx_designed_title" && index.is_unique)
    );
    let foreign_keys = plugin
        .list_foreign_keys(connection, "postgres", Some(schema.to_string()), "designed")
        .await
        .expect("altered foreign keys");
    assert!(
        foreign_keys
            .iter()
            .any(|foreign_key| foreign_key.name == "fk_designed_parent")
    );
}

async fn assert_exports(
    plugin: &PostgresPlugin,
    connection: &(dyn DbConnection + Send + Sync),
    schema: &str,
) {
    let create = plugin
        .export_table_create_sql(connection, "postgres", Some(schema), "designed")
        .await
        .expect("PostgreSQL create export");
    assert!(create.to_uppercase().contains("CREATE TABLE"));
    let data = plugin
        .export_table_data_sql(
            connection,
            "postgres",
            Some(schema),
            "designed",
            None,
            Some(10),
        )
        .await
        .expect("PostgreSQL data export");
    assert!(
        data.trim().is_empty(),
        "empty table should export no rows: {data}"
    );
}
