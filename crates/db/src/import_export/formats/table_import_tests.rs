use super::{CsvFormatHandler, JsonFormatHandler, TxtFormatHandler};
use crate::DatabasePlugin;
use crate::connection::{DbConnection, DbError, StreamingProgress};
use crate::executor::{ExecOptions, ExecResult, QueryResult, SqlResult, SqlSource};
use crate::import_export::{FormatHandler, ImportConfig};
use crate::mysql::MySqlPlugin;
use crate::types::ColumnInfo;
use async_trait::async_trait;
use one_core::storage::{DatabaseType, DbConnectionConfig};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

#[derive(Clone)]
struct RecordedExecution {
    script: String,
    options: ExecOptions,
}

struct RecordingConnection {
    config: DbConnectionConfig,
    columns: Vec<ColumnInfo>,
    executions: Arc<Mutex<Vec<RecordedExecution>>>,
}

impl RecordingConnection {
    fn new() -> Self {
        Self::with_columns(vec![
            column_info("id", "int"),
            column_info("name", "varchar(255)"),
        ])
    }

    fn with_columns(columns: Vec<ColumnInfo>) -> Self {
        Self {
            config: DbConnectionConfig {
                id: "test".to_string(),
                database_type: DatabaseType::MySQL,
                name: "mysql".to_string(),
                host: "localhost".to_string(),
                port: 3306,
                username: "root".to_string(),
                password: String::new(),
                database: Some("app".to_string()),
                service_name: None,
                sid: None,
                workspace_id: None,
                proxy: None,
                credential_reference: None,
                extra_params: Default::default(),
            },
            columns,
            executions: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn executions(&self) -> Vec<RecordedExecution> {
        self.executions.lock().unwrap().clone()
    }
}

#[async_trait]
impl DbConnection for RecordingConnection {
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
        if !query.contains("INFORMATION_SCHEMA.COLUMNS") {
            return Err(DbError::NotSupported(format!(
                "unexpected query in test: {query}"
            )));
        }

        Ok(SqlResult::Query(QueryResult {
            sql: query.to_string(),
            columns: vec![
                "COLUMN_NAME".to_string(),
                "COLUMN_TYPE".to_string(),
                "IS_NULLABLE".to_string(),
                "COLUMN_KEY".to_string(),
                "COLUMN_DEFAULT".to_string(),
                "COLUMN_COMMENT".to_string(),
                "CHARACTER_SET_NAME".to_string(),
                "COLLATION_NAME".to_string(),
            ],
            column_meta: Vec::new(),
            rows: self
                .columns
                .iter()
                .map(|column| {
                    vec![
                        Some(column.name.clone()),
                        Some(column.data_type.clone()),
                        Some(if column.is_nullable { "YES" } else { "NO" }.to_string()),
                        Some(if column.is_primary_key { "PRI" } else { "" }.to_string()),
                        column.default_value.clone(),
                        column.comment.clone(),
                        column.charset.clone(),
                        column.collation.clone(),
                    ]
                })
                .collect(),
            binary_cells: Vec::new(),
            elapsed_ms: 1,
        }))
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

fn column_info(name: &str, data_type: &str) -> ColumnInfo {
    ColumnInfo {
        name: name.to_string(),
        data_type: data_type.to_string(),
        is_nullable: true,
        is_primary_key: false,
        default_value: None,
        comment: None,
        charset: None,
        collation: None,
    }
}

fn import_config() -> ImportConfig {
    ImportConfig {
        database: "app".to_string(),
        table: Some("users".to_string()),
        truncate_before_import: true,
        use_transaction: true,
        stop_on_error: false,
        ..ImportConfig::default()
    }
}

fn assert_single_transactional_script(
    connection: &RecordingConnection,
    expected_table: &str,
    expected_insert_fragments: &[&str],
) {
    let executions = connection.executions();
    assert_eq!(1, executions.len());
    assert!(
        executions[0]
            .script
            .starts_with(&format!("TRUNCATE TABLE `app`.`{expected_table}`"))
    );
    for fragment in expected_insert_fragments {
        assert!(
            executions[0].script.contains(fragment),
            "missing fragment `{fragment}` in script:\n{}",
            executions[0].script
        );
    }
    assert!(executions[0].options.transactional);
    assert!(!executions[0].options.stop_on_error);
    assert_eq!(None, executions[0].options.max_rows);
    assert!(!executions[0].options.streaming);
}

#[tokio::test]
async fn csv_import_executes_truncate_and_rows_as_one_transactional_script() {
    let connection = RecordingConnection::new();
    let result = CsvFormatHandler
        .import(
            &MySqlPlugin::new(),
            &connection,
            &import_config(),
            "id,name\n1,Alice\n2,Bob\n",
        )
        .await
        .expect("CSV import should succeed");

    assert_eq!(2, result.rows_imported);
    assert_single_transactional_script(
        &connection,
        "users",
        &[
            "INSERT INTO `app`.`users` (`id`, `name`) VALUES ('1', 'Alice')",
            "INSERT INTO `app`.`users` (`id`, `name`) VALUES ('2', 'Bob')",
        ],
    );
}

#[tokio::test]
async fn json_import_executes_truncate_and_rows_as_one_transactional_script() {
    let connection = RecordingConnection::new();
    let result = JsonFormatHandler
        .import(
            &MySqlPlugin::new(),
            &connection,
            &import_config(),
            r#"[{"id":1,"name":"Alice"},{"id":2,"name":"Bob"}]"#,
        )
        .await
        .expect("JSON import should succeed");

    assert_eq!(2, result.rows_imported);
    assert_single_transactional_script(
        &connection,
        "users",
        &[
            "INSERT INTO `app`.`users` (`id`, `name`) VALUES (1, 'Alice')",
            "INSERT INTO `app`.`users` (`id`, `name`) VALUES (2, 'Bob')",
        ],
    );
}

#[tokio::test]
async fn txt_import_executes_truncate_and_rows_as_one_transactional_script() {
    let connection = RecordingConnection::new();
    let result = TxtFormatHandler
        .import(
            &MySqlPlugin::new(),
            &connection,
            &import_config(),
            "id\tname\n1\tAlice\n2\tBob\n",
        )
        .await
        .expect("TXT import should succeed");

    assert_eq!(2, result.rows_imported);
    assert_single_transactional_script(
        &connection,
        "users",
        &[
            "INSERT INTO `app`.`users` (`id`, `name`) VALUES ('1', 'Alice')",
            "INSERT INTO `app`.`users` (`id`, `name`) VALUES ('2', 'Bob')",
        ],
    );
}

fn bit_import_config() -> ImportConfig {
    ImportConfig {
        database: "app".to_string(),
        table: Some("test_bit".to_string()),
        truncate_before_import: true,
        use_transaction: true,
        stop_on_error: false,
        ..ImportConfig::default()
    }
}

fn bit_columns() -> Vec<ColumnInfo> {
    vec![
        column_info("id", "int"),
        column_info("bit_name", "bit(1)"),
        column_info("bit_mask", "bit(8)"),
        column_info("text_value", "varchar(20)"),
    ]
}

#[tokio::test]
async fn csv_import_formats_mysql_bit_values_as_unquoted_literals() {
    let connection = RecordingConnection::with_columns(bit_columns());
    let result = CsvFormatHandler
        .import(
            &MySqlPlugin::new(),
            &connection,
            &bit_import_config(),
            "id,bit_name,bit_mask,text_value\n1,1,5,1\n2,0,0x0F,0x0F\n",
        )
        .await
        .expect("CSV BIT import should succeed");

    assert_eq!(2, result.rows_imported);
    assert_single_transactional_script(
        &connection,
        "test_bit",
        &[
            "INSERT INTO `app`.`test_bit` (`id`, `bit_name`, `bit_mask`, `text_value`) VALUES ('1', 1, 5, '1')",
            "INSERT INTO `app`.`test_bit` (`id`, `bit_name`, `bit_mask`, `text_value`) VALUES ('2', 0, 0x0F, '0x0F')",
        ],
    );
}

#[tokio::test]
async fn txt_import_formats_mysql_bit_values_as_unquoted_literals() {
    let connection = RecordingConnection::with_columns(bit_columns());
    let result = TxtFormatHandler
        .import(
            &MySqlPlugin::new(),
            &connection,
            &bit_import_config(),
            "id\tbit_name\tbit_mask\ttext_value\n1\t1\t5\t1\n2\t0\t0x0F\t0x0F\n",
        )
        .await
        .expect("TXT BIT import should succeed");

    assert_eq!(2, result.rows_imported);
    assert_single_transactional_script(
        &connection,
        "test_bit",
        &[
            "INSERT INTO `app`.`test_bit` (`id`, `bit_name`, `bit_mask`, `text_value`) VALUES ('1', 1, 5, '1')",
            "INSERT INTO `app`.`test_bit` (`id`, `bit_name`, `bit_mask`, `text_value`) VALUES ('2', 0, 0x0F, '0x0F')",
        ],
    );
}

#[tokio::test]
async fn json_import_formats_string_encoded_mysql_bit_values_as_unquoted_literals() {
    let connection = RecordingConnection::with_columns(bit_columns());
    let result = JsonFormatHandler
        .import(
            &MySqlPlugin::new(),
            &connection,
            &bit_import_config(),
            r#"[{"id":1,"bit_name":"1","bit_mask":"0x0F","text_value":"1"},{"id":2,"bit_name":"0","bit_mask":"5","text_value":"0x0F"}]"#,
        )
        .await
        .expect("JSON BIT import should succeed");

    assert_eq!(2, result.rows_imported);
    assert_single_transactional_script(
        &connection,
        "test_bit",
        &[
            "INSERT INTO `app`.`test_bit` (`id`, `bit_name`, `bit_mask`, `text_value`) VALUES (1, 1, 0x0F, '1')",
            "INSERT INTO `app`.`test_bit` (`id`, `bit_name`, `bit_mask`, `text_value`) VALUES (2, 0, 5, '0x0F')",
        ],
    );
}
