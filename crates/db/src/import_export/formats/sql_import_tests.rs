use super::SqlFormatHandler;
use crate::DatabasePlugin;
use crate::connection::{DbConnection, DbError, StreamingProgress};
use crate::executor::{ExecOptions, ExecResult, SqlResult, SqlSource};
use crate::import_export::{FormatHandler, ImportConfig};
use crate::mysql::MySqlPlugin;
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
    executions: Arc<Mutex<Vec<RecordedExecution>>>,
}

impl RecordingConnection {
    fn new() -> Self {
        Self {
            config: test_config(),
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

    async fn query(&self, _query: &str) -> Result<SqlResult, DbError> {
        Err(DbError::NotSupported(
            "query is not used by this test".into(),
        ))
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

fn test_config() -> DbConnectionConfig {
    DbConnectionConfig {
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
    }
}

#[tokio::test]
async fn sql_import_executes_truncate_and_statements_as_one_transactional_script() {
    let connection = RecordingConnection::new();
    let plugin = MySqlPlugin::new();
    let config = ImportConfig {
        database: "app".to_string(),
        table: Some("users".to_string()),
        truncate_before_import: true,
        use_transaction: true,
        ..ImportConfig::default()
    };

    let result = SqlFormatHandler
        .import(
            &plugin,
            &connection,
            &config,
            "INSERT INTO users VALUES (1); INSERT INTO users VALUES (2);",
        )
        .await
        .expect("SQL import should return a result");

    let executions = connection.executions();
    assert_eq!(1, executions.len());
    assert!(
        executions[0]
            .script
            .contains("TRUNCATE TABLE `app`.`users`")
    );
    assert!(
        executions[0]
            .script
            .contains("INSERT INTO users VALUES (1)")
    );
    assert!(
        executions[0]
            .script
            .contains("INSERT INTO users VALUES (2)")
    );
    assert!(executions[0].options.transactional);
    assert!(executions[0].options.stop_on_error);
    assert_eq!(None, executions[0].options.max_rows);
    assert!(!executions[0].options.streaming);
    assert!(result.success);
    assert_eq!(2, result.rows_imported);
}

#[tokio::test]
async fn sql_import_passes_disabled_transaction_option_to_single_execution() {
    let connection = RecordingConnection::new();
    let plugin = MySqlPlugin::new();
    let config = ImportConfig {
        use_transaction: false,
        ..ImportConfig::default()
    };

    SqlFormatHandler
        .import(
            &plugin,
            &connection,
            &config,
            "INSERT INTO users VALUES (1); INSERT INTO users VALUES (2);",
        )
        .await
        .expect("SQL import should return a result");

    let executions = connection.executions();
    assert_eq!(1, executions.len());
    assert!(!executions[0].options.transactional);
}
