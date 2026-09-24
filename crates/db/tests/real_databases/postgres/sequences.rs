use db::connection::DbConnection;
use db::plugin::DatabasePlugin;
use db::postgresql::PostgresPlugin;
use db::types::DbNodeType;

use crate::real_databases::common::env::{optional_database, postgres_config, skip_database};
use crate::real_databases::postgres::core_flow::{
    drop_schema, execute, reset_schema, unique_schema,
};

/// 覆盖三种取值形态：显式 min/max、NO MINVALUE/NO MAXVALUE、以及 smallint 序列的上界。
const FIXTURE_SQL: &str = r#"
CREATE SEQUENCE plain;
CREATE SEQUENCE big AS bigint START 5 INCREMENT 3 MINVALUE 2 MAXVALUE 999 CYCLE;
CREATE SEQUENCE no_minmax NO MINVALUE NO MAXVALUE;
CREATE SEQUENCE small AS smallint;
"#;

#[tokio::test]
async fn postgres_real_sequences_list_values_and_schema_scope() {
    let Some(config) = postgres_config() else {
        skip_database(
            "PostgreSQL",
            "ONETCLI_TEST_POSTGRES_PASSWORD (empty string is valid)",
        );
        return;
    };
    let config = optional_database(
        &config,
        &std::env::var("ONETCLI_TEST_POSTGRES_DATABASE").unwrap_or_else(|_| "postgres".to_string()),
    );
    // 非 public schema：序列列表必须按 schema 取，不能固定查 public。
    let schema = unique_schema("seq");
    let plugin = PostgresPlugin::new();
    let mut connection = plugin
        .create_connection(config)
        .await
        .expect("PostgreSQL should connect");
    let conn: &(dyn DbConnection + Send + Sync) = connection.as_ref();

    reset_schema(&plugin, conn, &schema).await;
    conn.switch_schema(&schema)
        .await
        .expect("switch to the test schema");
    execute(&plugin, conn, FIXTURE_SQL).await;

    assert_listed_values(&plugin, conn, &schema).await;
    assert_object_view_uses_schema(&plugin, conn, &schema).await;
    assert_tree_folder_lists_sequences(&plugin, conn, &schema).await;

    drop_schema(&plugin, conn, &schema).await;
    connection.disconnect().await.expect("disconnect");
}

async fn assert_listed_values(
    plugin: &PostgresPlugin,
    connection: &(dyn DbConnection + Send + Sync),
    schema: &str,
) {
    let sequences = plugin
        .list_sequences(connection, "postgres", Some(schema.to_string()))
        .await
        .expect("sequences should list");
    let names: Vec<&str> = sequences.iter().map(|seq| seq.name.as_str()).collect();
    assert_eq!(vec!["big", "no_minmax", "plain", "small"], names);

    let sequence = |name: &str| {
        sequences
            .iter()
            .find(|seq| seq.name == name)
            .unwrap_or_else(|| panic!("{name} should be listed, got {names:?}"))
    };

    let big = sequence("big");
    assert_eq!(Some(5), big.start_value);
    assert_eq!(Some(3), big.increment);
    assert_eq!(Some(2), big.min_value);
    assert_eq!(Some(999), big.max_value);

    // NO MINVALUE 不代表取不到值：information_schema 会给实际生效的边界。
    let no_minmax = sequence("no_minmax");
    assert_eq!(Some(1), no_minmax.min_value);
    assert_eq!(Some(i64::MAX), no_minmax.max_value);

    // smallint 序列的上界不是 bigint 上界，说明取的是序列自身定义而非类型默认值。
    assert_eq!(Some(i64::from(i16::MAX)), sequence("small").max_value);
    assert_eq!(Some(1), sequence("plain").min_value);
}

async fn assert_object_view_uses_schema(
    plugin: &PostgresPlugin,
    connection: &(dyn DbConnection + Send + Sync),
    schema: &str,
) {
    let view = plugin
        .list_sequences_view_in_schema(connection, "postgres", Some(schema.to_string()))
        .await
        .expect("sequence object view should load");
    assert_eq!(DbNodeType::Sequence, view.db_node_type);
    let rows: Vec<&Vec<String>> = view
        .rows
        .iter()
        .filter(|row| row.first().map(String::as_str) == Some("big"))
        .collect();
    assert_eq!(1, rows.len(), "big should appear exactly once in {view:?}");
    assert_eq!(
        vec!["big", "5", "3", "2", "999"],
        rows[0].iter().map(String::as_str).collect::<Vec<_>>()
    );

    // 不带 schema 的实现必须仍然查 public：非 public schema 的序列不该在这里露头。
    let public_view = plugin
        .list_sequences_view(connection, "postgres")
        .await
        .expect("public sequence object view should load");
    assert!(
        !public_view
            .rows
            .iter()
            .any(|row| row.first().map(String::as_str) == Some("big")),
        "public view must not leak sequences from another schema"
    );
    assert!(
        !public_view
            .rows
            .iter()
            .any(|row| row.first().map(String::as_str) == Some("plain")),
        "public view must not leak sequences from another schema"
    );
}

async fn assert_tree_folder_lists_sequences(
    plugin: &PostgresPlugin,
    connection: &(dyn DbConnection + Send + Sync),
    schema: &str,
) {
    let schema_node = db::types::DbNode::new(
        format!("conn:{schema}"),
        schema,
        DbNodeType::Schema,
        "conn".to_string(),
        one_core::storage::DatabaseType::PostgreSQL,
    )
    .with_metadata(std::collections::HashMap::from([(
        "database".to_string(),
        "postgres".to_string(),
    )]));

    let children = plugin
        .build_database_or_schema_children(connection, &schema_node, Some(schema.to_string()))
        .await
        .expect("schema children should build");
    let folder = children
        .iter()
        .find(|node| node.node_type == DbNodeType::SequencesFolder)
        .expect("sequences folder should exist");
    let names: Vec<&str> = folder
        .children
        .iter()
        .map(|node| node.name.as_str())
        .collect();
    assert_eq!(vec!["big", "no_minmax", "plain", "small"], names);
    assert_eq!(
        Some("5"),
        folder
            .children
            .iter()
            .find(|node| node.name == "big")
            .and_then(|node| node.metadata.get("start_value"))
            .map(String::as_str)
    );
}
