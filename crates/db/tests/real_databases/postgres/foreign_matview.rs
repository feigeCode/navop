use db::connection::DbConnection;
use db::plugin::DatabasePlugin;
use db::postgresql::PostgresPlugin;
use db::types::{DbNode, DbNodeType, TableObjectType};
use one_core::storage::DatabaseType;

use crate::real_databases::common::env::{optional_database, postgres_config, skip_database};
use crate::real_databases::postgres::core_flow::{
    drop_schema, execute, reset_schema, unique_schema,
};

/// 外部表依赖 FDW：没有外部服务器就建不出外部表。本地测试用 postgres_fdw 指回
/// 同一个库，行为与真实外部表一致（可 SELECT / TRUNCATE，DDL 用 FOREIGN TABLE 语法）。
const FIXTURE_SQL: &str = r#"
CREATE TABLE orders (
    id INTEGER PRIMARY KEY,
    amount NUMERIC(10, 2) NOT NULL
);
INSERT INTO orders VALUES (1, 10.50), (2, 20.25);

CREATE TABLE orders_part (
    id INTEGER NOT NULL,
    amount NUMERIC(10, 2) NOT NULL
) PARTITION BY RANGE (id);
CREATE TABLE orders_part_low PARTITION OF orders_part FOR VALUES FROM (0) TO (100);
INSERT INTO orders_part VALUES (1, 1.00);

CREATE MATERIALIZED VIEW mv_orders AS
    SELECT id, amount * 2 AS doubled FROM orders;
COMMENT ON MATERIALIZED VIEW mv_orders IS 'doubled orders';
"#;

#[tokio::test]
async fn postgres_real_foreign_table_and_materialized_view_flow() {
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
    let password = config.password.clone();
    // 用非 public schema：DDL 必须自带 schema 限定，不能依赖会话 search_path。
    let schema = unique_schema("foreign");
    let server = format!("{schema}_srv");
    let plugin = PostgresPlugin::new();
    let mut connection = plugin
        .create_connection(config.clone())
        .await
        .expect("PostgreSQL should connect");
    let conn: &(dyn DbConnection + Send + Sync) = connection.as_ref();
    // 这个库本来没装 postgres_fdw 的话，测完把扩展也清掉，不给本地库留残留。
    let extension_existed = postgres_fdw_installed(conn).await;

    reset_schema(&plugin, conn, &schema).await;
    conn.switch_schema(&schema)
        .await
        .expect("switch to the test schema");
    create_foreign_table(&plugin, conn, &schema, &server, &password).await;
    execute(&plugin, conn, FIXTURE_SQL).await;

    assert_listed_objects(&plugin, conn, &schema).await;
    assert_object_view_types(&plugin, conn, &schema).await;
    assert_tree_children(&plugin, conn, &schema).await;
    assert_foreign_table_lifecycle(&plugin, conn, &schema, &server).await;
    assert_materialized_view_lifecycle(&plugin, conn, &schema).await;

    drop_schema(&plugin, conn, &schema).await;
    execute(
        &plugin,
        conn,
        &format!("DROP SERVER IF EXISTS \"{server}\" CASCADE;"),
    )
    .await;
    if !extension_existed {
        execute(&plugin, conn, "DROP EXTENSION IF EXISTS postgres_fdw;").await;
    }
    connection
        .disconnect()
        .await
        .expect("PostgreSQL should disconnect");
}

async fn create_foreign_table(
    plugin: &PostgresPlugin,
    connection: &(dyn DbConnection + Send + Sync),
    schema: &str,
    server: &str,
    password: &str,
) {
    let sql = foreign_table_setup_sql(schema, server, password);
    execute(plugin, connection, &sql).await;
}

async fn postgres_fdw_installed(connection: &(dyn DbConnection + Send + Sync)) -> bool {
    let result = connection
        .query("SELECT extname FROM pg_extension WHERE extname = 'postgres_fdw'")
        .await
        .expect("extension lookup should run");
    matches!(result, db::executor::SqlResult::Query(query) if !query.rows.is_empty())
}

/// 建立外部表所需的 SQL：FDW 扩展、外部服务器、用户映射、外部表本体。
fn foreign_table_setup_sql(schema: &str, server: &str, password: &str) -> String {
    // 密码来自环境变量（ONETCLI_TEST_POSTGRES_PASSWORD），不硬编码。
    let escaped_password = password.replace('\'', "''");
    // 显式限定外部表与远端表的 schema，不靠会话 search_path。
    format!(
        "CREATE EXTENSION IF NOT EXISTS postgres_fdw;\n\
         CREATE SERVER \"{server}\" FOREIGN DATA WRAPPER postgres_fdw \
             OPTIONS (host '127.0.0.1', dbname 'postgres');\n\
         CREATE USER MAPPING FOR CURRENT_USER SERVER \"{server}\" \
             OPTIONS (user 'postgres', password '{escaped_password}');\n\
         CREATE FOREIGN TABLE \"{schema}\".remote_orders (id INTEGER, amount NUMERIC(10, 2)) \
             SERVER \"{server}\" OPTIONS (schema_name '{schema}', table_name 'orders');"
    )
}

async fn assert_listed_objects(
    plugin: &PostgresPlugin,
    connection: &(dyn DbConnection + Send + Sync),
    schema: &str,
) {
    let tables = plugin
        .list_tables(connection, "postgres", Some(schema.to_string()))
        .await
        .expect("tables should list");
    let table = |name: &str| {
        tables
            .iter()
            .find(|table| table.name == name)
            .unwrap_or_else(|| panic!("{name} should be listed, got {:?}", names(&tables)))
    };

    // 普通表 / 分区表 / 外部表都在「表」目录，只有外部表需要区分类型。
    assert_eq!(TableObjectType::Table, table("orders").object_type);
    assert_eq!(TableObjectType::Table, table("orders_part").object_type);
    assert_eq!(
        TableObjectType::ForeignTable,
        table("remote_orders").object_type
    );
    assert!(!table("remote_orders").object_type.is_ddl_comparable());
    assert!(table("orders").object_type.is_ddl_comparable());

    // 物化视图不在 information_schema.views 里，必须走 pg_class。
    let views = plugin
        .list_views(connection, "postgres", Some(schema.to_string()))
        .await
        .expect("views should list");
    assert!(
        !views.iter().any(|view| view.name == "mv_orders"),
        "materialized views must not leak into the plain views list"
    );
    let matviews = plugin
        .list_materialized_views(connection, "postgres", Some(schema.to_string()))
        .await
        .expect("materialized views should list");
    let matview = matviews
        .iter()
        .find(|view| view.name == "mv_orders")
        .expect("mv_orders should be listed as a materialized view");
    assert_eq!(Some(schema), matview.schema.as_deref());
    assert_eq!(Some("doubled orders"), matview.comment.as_deref());
    assert!(
        matview
            .definition
            .as_deref()
            .unwrap_or_default()
            .contains("orders"),
        "materialized view definition should come from pg_get_viewdef: {:?}",
        matview.definition
    );
}

async fn assert_object_view_types(
    plugin: &PostgresPlugin,
    connection: &(dyn DbConnection + Send + Sync),
    schema: &str,
) {
    let view = plugin
        .list_tables_view(connection, "postgres", Some(schema.to_string()))
        .await
        .expect("tables view should load");
    let type_of = |name: &str| {
        let row = view
            .rows
            .iter()
            .find(|row| row.first().map(String::as_str) == Some(name))
            .unwrap_or_else(|| panic!("{name} should appear in the object list"));
        // 列顺序：name / owner / type / rows / size / indexes / tablespace / comment
        row.get(2).cloned().unwrap_or_default()
    };
    assert_eq!("Table", type_of("orders"));
    assert_eq!("Partitioned Table", type_of("orders_part"));
    assert_eq!("Foreign Table", type_of("remote_orders"));

    let matviews = plugin
        .list_materialized_views_view(connection, "postgres", Some(schema.to_string()))
        .await
        .expect("materialized views view should load");
    assert_eq!(DbNodeType::MaterializedView, matviews.db_node_type);
    assert!(
        matviews
            .rows
            .iter()
            .any(|row| row.first().map(String::as_str) == Some("mv_orders")),
        "mv_orders should appear in the materialized views object list"
    );

    let tables_view = plugin
        .list_tables_view(connection, "postgres", Some(schema.to_string()))
        .await
        .expect("tables view should reload");
    assert!(
        !tables_view
            .rows
            .iter()
            .any(|row| row.first().map(String::as_str) == Some("mv_orders")),
        "materialized views must not appear in the tables object list"
    );
}

async fn assert_tree_children(
    plugin: &PostgresPlugin,
    connection: &(dyn DbConnection + Send + Sync),
    schema: &str,
) {
    let schema_node = DbNode::new(
        format!("conn:{schema}"),
        schema,
        DbNodeType::Schema,
        "conn".to_string(),
        DatabaseType::PostgreSQL,
    )
    .with_metadata(std::collections::HashMap::from([(
        "database".to_string(),
        "postgres".to_string(),
    )]));

    let children = plugin
        .build_database_or_schema_children(connection, &schema_node, Some(schema.to_string()))
        .await
        .expect("schema children should build");

    let tables = children
        .iter()
        .find(|node| node.node_type == DbNodeType::TablesFolder)
        .expect("tables folder should exist");
    let node_type_of = |name: &str| {
        tables
            .children
            .iter()
            .find(|node| node.name == name)
            .unwrap_or_else(|| panic!("{name} should be a table node"))
            .node_type
    };
    assert_eq!(DbNodeType::Table, node_type_of("orders"));
    assert_eq!(DbNodeType::Table, node_type_of("orders_part"));
    assert_eq!(DbNodeType::ForeignTable, node_type_of("remote_orders"));

    let matviews = children
        .iter()
        .find(|node| node.node_type == DbNodeType::MaterializedViewsFolder)
        .expect("materialized views folder should exist");
    assert_eq!(
        vec![DbNodeType::MaterializedView],
        matviews
            .children
            .iter()
            .map(|node| node.node_type)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        Some("mv_orders"),
        matviews.children.first().map(|node| node.name.as_str()),
    );
}

async fn assert_foreign_table_lifecycle(
    plugin: &PostgresPlugin,
    connection: &(dyn DbConnection + Send + Sync),
    schema: &str,
    server: &str,
) {
    // 外部表可直接查询（数据面可用）。
    assert_eq!(
        2,
        query_row_count(connection, "remote_orders", schema).await
    );

    // 生成的 ALTER 必须能真的改掉非 public schema 下的外部表。
    let rename =
        plugin.rename_foreign_table("postgres", Some(schema), "remote_orders", "orders_fdw");
    execute(plugin, connection, &rename).await;
    let renamed = plugin
        .list_tables(connection, "postgres", Some(schema.to_string()))
        .await
        .expect("tables should list after rename");
    assert!(
        renamed.iter().any(|table| table.name == "orders_fdw"
            && table.object_type == TableObjectType::ForeignTable),
        "renamed foreign table should still be a foreign table, got {:?}",
        names(&renamed)
    );
    assert!(!renamed.iter().any(|table| table.name == "remote_orders"));

    // 生成的 DROP FOREIGN TABLE 必须命中外部表本身。
    let drop = plugin.drop_foreign_table("postgres", Some(schema), "orders_fdw");
    execute(plugin, connection, &drop).await;
    let after_drop = plugin
        .list_tables(connection, "postgres", Some(schema.to_string()))
        .await
        .expect("tables should list after drop");
    assert!(
        !after_drop.iter().any(|table| table.name == "orders_fdw"),
        "dropped foreign table should disappear, got {:?}",
        names(&after_drop)
    );

    // 重建一张外部表，确认删除普通表不会连带外部表（两条 DDL 互不干扰）。
    let rebuild = format!(
        "CREATE FOREIGN TABLE \"{schema}\".remote_orders_2 (id INTEGER, amount NUMERIC(10, 2)) \
         SERVER \"{server}\" OPTIONS (schema_name '{schema}', table_name 'orders');"
    );
    execute(plugin, connection, &rebuild).await;
    execute(
        plugin,
        connection,
        &plugin.drop_table("postgres", Some(schema), "orders_part_low"),
    )
    .await;
    let final_tables = plugin
        .list_tables(connection, "postgres", Some(schema.to_string()))
        .await
        .expect("tables should list at the end");
    assert!(
        final_tables
            .iter()
            .any(|table| table.name == "remote_orders_2"),
        "dropping a plain table must not touch foreign tables, got {:?}",
        names(&final_tables)
    );
}

async fn assert_materialized_view_lifecycle(
    plugin: &PostgresPlugin,
    connection: &(dyn DbConnection + Send + Sync),
    schema: &str,
) {
    // 物化视图存了数据，可以直接查。
    assert_eq!(2, query_row_count(connection, "mv_orders", schema).await);

    let drop = plugin.drop_materialized_view("postgres", Some(schema), "mv_orders");
    execute(plugin, connection, &drop).await;
    let matviews = plugin
        .list_materialized_views(connection, "postgres", Some(schema.to_string()))
        .await
        .expect("materialized views should list after drop");
    assert!(
        !matviews.iter().any(|view| view.name == "mv_orders"),
        "dropped materialized view should disappear"
    );
}

async fn query_row_count(
    connection: &(dyn DbConnection + Send + Sync),
    table: &str,
    schema: &str,
) -> usize {
    let result = connection
        .query(&format!("SELECT * FROM \"{schema}\".\"{table}\""))
        .await
        .expect("select should run");
    match result {
        db::executor::SqlResult::Query(query) => query.rows.len(),
        other => panic!("expected rows from {table}, got {other:?}"),
    }
}

fn names(tables: &[db::types::TableInfo]) -> Vec<String> {
    tables.iter().map(|table| table.name.clone()).collect()
}
