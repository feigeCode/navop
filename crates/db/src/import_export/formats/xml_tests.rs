use super::XmlFormatHandler;
use crate::DatabasePlugin;
use crate::connection::{DbConnection, DbError, StreamingProgress};
use crate::executor::{
    BinaryCell, ExecOptions, ExecResult, QueryColumnMeta, QueryResult, SqlResult, SqlSource,
};
use crate::import_export::{ExportConfig, FormatHandler, ImportConfig};
use crate::mysql::MySqlPlugin;
use crate::sqlite::{SqliteDbConnection, SqlitePlugin};
use async_trait::async_trait;
use one_core::storage::{DatabaseType, DbConnectionConfig};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

#[derive(Clone)]
struct RecordedExecution {
    script: String,
    options: ExecOptions,
}

struct XmlTestConnection {
    config: DbConnectionConfig,
    query_result: Option<QueryResult>,
    executions: Arc<Mutex<Vec<RecordedExecution>>>,
}

impl XmlTestConnection {
    fn for_export(query_result: QueryResult) -> Self {
        Self {
            config: test_config(DatabaseType::MySQL),
            query_result: Some(query_result),
            executions: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn for_import() -> Self {
        Self {
            config: test_config(DatabaseType::MySQL),
            query_result: None,
            executions: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn executions(&self) -> Vec<RecordedExecution> {
        self.executions.lock().unwrap().clone()
    }
}

#[async_trait]
impl DbConnection for XmlTestConnection {
    fn config(&self) -> &DbConnectionConfig {
        &self.config
    }

    fn set_config_database(&mut self, database: Option<String>) {
        self.config.database = database;
    }

    async fn connect(&mut self) -> Result<(), DbError> {
        Ok(())
    }

    async fn disconnect(&mut self) -> Result<(), DbError> {
        Ok(())
    }

    async fn execute(
        &self,
        _plugin: &dyn DatabasePlugin,
        script: &str,
        options: ExecOptions,
    ) -> Result<Vec<SqlResult>, DbError> {
        self.executions.lock().unwrap().push(RecordedExecution {
            script: script.to_string(),
            options,
        });
        Ok(script
            .split(';')
            .map(str::trim)
            .filter(|statement| !statement.is_empty())
            .map(|statement| {
                let rows_affected = if statement.starts_with("TRUNCATE") {
                    99
                } else {
                    1
                };
                SqlResult::Exec(ExecResult {
                    sql: statement.to_string(),
                    rows_affected,
                    elapsed_ms: 1,
                    message: None,
                })
            })
            .collect())
    }

    async fn query(&self, query: &str) -> Result<SqlResult, DbError> {
        let mut result = self
            .query_result
            .clone()
            .ok_or_else(|| DbError::NotSupported("query is not used by this test".into()))?;
        result.sql = query.to_string();
        Ok(SqlResult::Query(result))
    }

    async fn current_database(&self) -> Result<Option<String>, DbError> {
        Ok(Some("app".to_string()))
    }

    async fn switch_database(&self, _database: &str) -> Result<(), DbError> {
        Ok(())
    }

    async fn execute_streaming(
        &self,
        _plugin: &dyn DatabasePlugin,
        _source: SqlSource,
        _options: ExecOptions,
        _sender: mpsc::Sender<StreamingProgress>,
    ) -> Result<(), DbError> {
        Ok(())
    }
}

fn test_config(database_type: DatabaseType) -> DbConnectionConfig {
    DbConnectionConfig {
        id: "test".to_string(),
        database_type,
        name: "test".to_string(),
        host: "localhost".to_string(),
        port: 0,
        username: String::new(),
        password: String::new(),
        database: Some("app".to_string()),
        service_name: None,
        sid: None,
        workspace_id: None,
        proxy: None,
        credential_reference: None,
        extra_params: Default::default(),
    }
}

#[tokio::test]
async fn xml_export_distinguishes_null_empty_text_and_binary_bytes() {
    let connection = XmlTestConnection::for_export(QueryResult {
        sql: String::new(),
        columns: vec![
            "nullable".to_string(),
            "empty".to_string(),
            "marker".to_string(),
            "payload".to_string(),
            "display name".to_string(),
        ],
        column_meta: vec![
            QueryColumnMeta::new("nullable", "TEXT"),
            QueryColumnMeta::new("empty", "TEXT"),
            QueryColumnMeta::new("marker", "TEXT"),
            QueryColumnMeta::new("payload", "BLOB"),
            QueryColumnMeta::new("display name", "TEXT"),
        ],
        rows: vec![vec![
            None,
            Some(String::new()),
            Some("0x0001ff".to_string()),
            Some("0x0001ff".to_string()),
            Some("A & <B>".to_string()),
        ]],
        binary_cells: vec![BinaryCell {
            row_index: 0,
            column_index: 3,
            bytes: vec![0x00, 0x01, 0xff],
        }],
        elapsed_ms: 1,
    });
    let config = ExportConfig {
        database: "app".to_string(),
        tables: vec!["odd table".to_string()],
        ..ExportConfig::default()
    };

    let result = XmlFormatHandler
        .export(&MySqlPlugin::new(), &connection, &config)
        .await
        .expect("XML export should succeed");

    roxmltree::Document::parse(&result.output).expect("export should be valid XML");
    assert!(result.output.contains("<row table=\"odd table\">"));
    assert!(
        result
            .output
            .contains("<field name=\"nullable\" null=\"true\"></field>")
    );
    assert!(result.output.contains("<field name=\"empty\"></field>"));
    assert!(
        result
            .output
            .contains("<field name=\"marker\">0x0001ff</field>")
    );
    assert!(
        result
            .output
            .contains("<field name=\"payload\" encoding=\"hex\">0001ff</field>")
    );
    assert!(
        result
            .output
            .contains("<field name=\"display name\">A &amp; &lt;B&gt;</field>")
    );
}

#[tokio::test]
async fn xml_import_builds_one_transactional_script_with_lossless_values() {
    let connection = XmlTestConnection::for_import();
    let config = ImportConfig {
        database: "app".to_string(),
        table: Some("users".to_string()),
        truncate_before_import: true,
        use_transaction: true,
        ..ImportConfig::default()
    };
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<data>
  <row table="users">
    <field name="nullable" null="true"></field>
    <field name="empty"></field>
    <field name="marker">O&apos;Reilly</field>
    <field name="payload" encoding="hex">0001ff</field>
  </row>
</data>"#;

    let result = XmlFormatHandler
        .import(&MySqlPlugin::new(), &connection, &config, xml)
        .await
        .expect("XML import should succeed");

    let executions = connection.executions();
    assert_eq!(1, executions.len());
    assert!(
        executions[0]
            .script
            .contains("TRUNCATE TABLE `app`.`users`")
    );
    assert!(executions[0].script.contains(
        "INSERT INTO `app`.`users` (`nullable`, `empty`, `marker`, `payload`) VALUES (NULL, '', 'O''Reilly', X'0001ff')"
    ));
    assert!(executions[0].options.transactional);
    assert_eq!(None, executions[0].options.max_rows);
    assert!(result.success);
    assert_eq!(1, result.rows_imported);
}

#[tokio::test]
async fn xml_import_accepts_legacy_table_and_column_tags() {
    let connection = XmlTestConnection::for_import();
    let config = ImportConfig {
        database: "app".to_string(),
        table: Some("users".to_string()),
        ..ImportConfig::default()
    };
    let xml = r#"<data><users><id>1</id><name>Alice</name></users></data>"#;

    XmlFormatHandler
        .import(&MySqlPlugin::new(), &connection, &config, xml)
        .await
        .expect("legacy XML should import");

    assert!(
        connection.executions()[0]
            .script
            .contains("INSERT INTO `app`.`users` (`id`, `name`) VALUES ('1', 'Alice')")
    );
}

fn sqlite_config(id: &str, path: &std::path::Path) -> DbConnectionConfig {
    let mut config = test_config(DatabaseType::SQLite);
    config.id = id.to_string();
    config.name = id.to_string();
    config.host = path.to_string_lossy().to_string();
    config.database = None;
    config
}

fn assert_execute_succeeded(results: &[SqlResult]) {
    if let Some(SqlResult::Error(error)) = results.iter().find(|result| result.is_error()) {
        panic!("SQL execution failed: {}", error.message);
    }
}

#[tokio::test]
async fn sqlite_xml_round_trip_preserves_binary_null_empty_and_hex_like_text() {
    let temp_dir = tempfile::tempdir().expect("temp dir should be created");
    let source_path = temp_dir.path().join("source.db");
    let target_path = temp_dir.path().join("target.db");
    let plugin = SqlitePlugin::new();
    let mut source = SqliteDbConnection::new(sqlite_config("source", &source_path));
    source.connect().await.expect("source should connect");
    let source_results = source
        .execute(
            &plugin,
            "CREATE TABLE data (payload BLOB, nullable TEXT, empty_text TEXT, marker TEXT);
             INSERT INTO data VALUES (X'0001ff', NULL, '', '0x0001ff');",
            ExecOptions::default(),
        )
        .await
        .expect("fixture should execute");
    assert_execute_succeeded(&source_results);

    let exported = XmlFormatHandler
        .export(
            &plugin,
            &source,
            &ExportConfig {
                database: "main".to_string(),
                tables: vec!["data".to_string()],
                ..ExportConfig::default()
            },
        )
        .await
        .expect("XML export should succeed");

    let mut target = SqliteDbConnection::new(sqlite_config("target", &target_path));
    target.connect().await.expect("target should connect");
    let target_schema = target
        .execute(
            &plugin,
            "CREATE TABLE data (payload BLOB, nullable TEXT, empty_text TEXT, marker TEXT);",
            ExecOptions::default(),
        )
        .await
        .expect("target schema should execute");
    assert_execute_succeeded(&target_schema);
    let imported = XmlFormatHandler
        .import(
            &plugin,
            &target,
            &ImportConfig {
                database: "main".to_string(),
                table: Some("data".to_string()),
                ..ImportConfig::default()
            },
            &exported.output,
        )
        .await
        .expect("XML import should succeed");
    assert!(imported.success, "{:?}", imported.errors);

    let SqlResult::Query(result) = target
        .query(
            "SELECT hex(payload), typeof(payload), nullable IS NULL,
                    length(empty_text), marker, typeof(marker) FROM data",
        )
        .await
        .expect("restored data should be queryable")
    else {
        panic!("expected query result");
    };
    assert_eq!(
        vec![
            Some("0001FF".to_string()),
            Some("blob".to_string()),
            Some("1".to_string()),
            Some("0".to_string()),
            Some("0x0001ff".to_string()),
            Some("text".to_string()),
        ],
        result.rows[0]
    );
}
