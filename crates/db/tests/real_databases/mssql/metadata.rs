use db::connection::DbConnection;
use db::executor::ExecOptions;
use db::mssql::MsSqlPlugin;
use db::plugin::DatabasePlugin;

use crate::real_databases::common::assertions::assert_no_sql_errors;
use crate::real_databases::common::env::{mssql_config, skip_database};

const SCHEMA: &str = "dbo";

/// 回归用例：系统目录视图里 `is_nullable` / `is_unique` 是可空 `bit`，
/// TDS 会把它编成变长类型 `BITN`。解码器缺 `BITN` 时整个元数据查询会失败，
/// 表设计器只剩表头、没有任何列。
#[tokio::test]
async fn mssql_real_metadata_reads_nullable_bit_columns() {
    let Some(config) = mssql_config() else {
        skip_database("MSSQL", "ONETCLI_TEST_MSSQL_PASSWORD");
        return;
    };
    let database = config
        .database
        .clone()
        .expect("MSSQL 测试配置必须有默认库名");
    let plugin = MsSqlPlugin::new();
    let connection = plugin
        .create_connection(config)
        .await
        .expect("MSSQL 应该能连接");
    let table = format!("navop_meta_probe_{}", std::process::id());
    let conn = connection.as_ref();

    drop_fixture(&plugin, conn, &table).await;
    create_fixture(&plugin, conn, &table).await;

    let columns = plugin
        .list_columns(conn, &database, Some(SCHEMA.to_string()), &table)
        .await
        .expect("list_columns 必须能解码可空 bit 列");

    let names: Vec<&str> = columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "id",
            "label",
            "optional_flag",
            "amount",
            "created_at",
            "offset_at"
        ]
    );

    let optional_flag = find_column(&columns, "optional_flag");
    assert!(
        optional_flag.is_nullable,
        "可空 bit 列的 is_nullable 应为 true"
    );
    assert!(!optional_flag.is_primary_key);

    let label = find_column(&columns, "label");
    assert!(!label.is_nullable, "NOT NULL 列的 is_nullable 应为 false");

    let id = find_column(&columns, "id");
    assert!(id.is_primary_key, "主键列应被标记为 is_primary_key");
    assert!(!id.is_nullable);
    assert!(
        id.is_auto_increment,
        "IDENTITY 列应被标记为 is_auto_increment（设计器靠它保留 IDENTITY）"
    );
    // `c.is_identity` 在列表里只对 IDENTITY 列为真，普通列不能误报。
    assert!(!label.is_auto_increment, "普通列不应被标记为自增");
    assert!(!find_column(&columns, "amount").is_auto_increment);

    let indexes = plugin
        .list_indexes(conn, &database, Some(SCHEMA.to_string()), &table)
        .await
        .expect("list_indexes 必须能解码可空 bit 列");
    // MSSQL 的 list_indexes 设计上排除主键（主键由 list_columns.is_primary_key 携带）。
    assert!(
        indexes.iter().all(|i| !i.is_primary),
        "list_indexes 不应返回主键索引"
    );
    let unique_index = indexes
        .iter()
        .find(|i| i.name == "ux_label")
        .expect("应返回 ux_label 唯一索引");
    assert!(unique_index.is_unique, "唯一索引 is_unique 应为 true");
    assert_eq!(unique_index.columns, vec!["label".to_string()]);

    drop_fixture(&plugin, conn, &table).await;
}

fn find_column<'a>(columns: &'a [db::types::ColumnInfo], name: &str) -> &'a db::types::ColumnInfo {
    columns
        .iter()
        .find(|c| c.name == name)
        .unwrap_or_else(|| panic!("缺少列 {name}"))
}

async fn create_fixture(
    plugin: &MsSqlPlugin,
    connection: &(dyn DbConnection + Send + Sync),
    table: &str,
) {
    execute(
        plugin,
        connection,
        &format!(
            "CREATE TABLE [{SCHEMA}].[{table}] (\
                 id BIGINT IDENTITY(1,1) NOT NULL PRIMARY KEY, \
                 label NVARCHAR(64) NOT NULL, \
                 optional_flag BIT NULL, \
                 amount DECIMAL(10,2) NULL, \
                 created_at DATETIME2 NOT NULL, \
                 offset_at DATETIMEOFFSET NULL)"
        ),
    )
    .await;
    execute(
        plugin,
        connection,
        &format!("CREATE UNIQUE INDEX [ux_label] ON [{SCHEMA}].[{table}] ([label])"),
    )
    .await;
}

async fn drop_fixture(
    plugin: &MsSqlPlugin,
    connection: &(dyn DbConnection + Send + Sync),
    table: &str,
) {
    execute(
        plugin,
        connection,
        &format!("DROP TABLE IF EXISTS [{SCHEMA}].[{table}]"),
    )
    .await;
}

async fn execute(plugin: &MsSqlPlugin, connection: &(dyn DbConnection + Send + Sync), sql: &str) {
    let results = connection
        .execute(plugin, sql, ExecOptions::default())
        .await
        .expect("MSSQL 脚本应能执行");
    assert_no_sql_errors(&results, sql);
}
