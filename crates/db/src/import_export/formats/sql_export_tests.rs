use super::*;
use crate::connection::{DbError, StreamingProgress};
use crate::executor::{ExecOptions, QueryColumnMeta, SqlSource};
use crate::mysql::MySqlPlugin;
use async_trait::async_trait;
use one_core::storage::{DatabaseType, DbConnectionConfig};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

struct PagedConnection {
    config: DbConnectionConfig,
    queries: Arc<Mutex<Vec<String>>>,
    pages: Arc<Mutex<Vec<Vec<Vec<Option<String>>>>>>,
}

impl PagedConnection {
    fn new(pages: Vec<Vec<Vec<Option<String>>>>) -> Self {
        Self {
            config: test_config(),
            queries: Arc::new(Mutex::new(Vec::new())),
            pages: Arc::new(Mutex::new(pages)),
        }
    }

    fn queries(&self) -> Vec<String> {
        self.queries.lock().unwrap().clone()
    }
}

#[async_trait]
impl DbConnection for PagedConnection {
    fn config(&self) -> &DbConnectionConfig {
        &self.config
    }

    fn set_config_database(&mut self, database: Option<String>) {
        self.config.database = database;
    }

    async fn connect(&mut self) -> std::result::Result<(), DbError> {
        Ok(())
    }

    async fn disconnect(&mut self) -> std::result::Result<(), DbError> {
        Ok(())
    }

    async fn execute(
        &self,
        _plugin: &dyn DatabasePlugin,
        _script: &str,
        _options: ExecOptions,
    ) -> std::result::Result<Vec<SqlResult>, DbError> {
        Ok(Vec::new())
    }

    async fn query(&self, query: &str) -> std::result::Result<SqlResult, DbError> {
        self.queries.lock().unwrap().push(query.to_string());
        let rows = self.pages.lock().unwrap().remove(0);
        Ok(SqlResult::Query(QueryResult {
            sql: query.to_string(),
            columns: vec!["id".to_string(), "name".to_string()],
            column_meta: vec![
                QueryColumnMeta::new("id", "BIGINT"),
                QueryColumnMeta::new("name", "VARCHAR"),
            ],
            rows,
            binary_cells: vec![],
            elapsed_ms: 1,
        }))
    }

    async fn current_database(&self) -> std::result::Result<Option<String>, DbError> {
        Ok(Some("app".to_string()))
    }

    async fn switch_database(&self, _database: &str) -> std::result::Result<(), DbError> {
        Ok(())
    }

    async fn execute_streaming(
        &self,
        _plugin: &dyn DatabasePlugin,
        _source: SqlSource,
        _options: ExecOptions,
        _sender: mpsc::Sender<StreamingProgress>,
    ) -> std::result::Result<(), DbError> {
        Ok(())
    }
}

fn row(id: usize) -> Vec<Option<String>> {
    vec![Some(id.to_string()), Some(format!("user'{id}"))]
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
        extra_params: Default::default(),
    }
}

#[tokio::test]
async fn sql_export_streams_table_data_in_pages() {
    let first_page = (0..SQL_EXPORT_PAGE_SIZE).map(row).collect::<Vec<_>>();
    let second_page = vec![row(SQL_EXPORT_PAGE_SIZE)];
    let connection = PagedConnection::new(vec![first_page, second_page]);
    let plugin = MySqlPlugin::new();
    let config = ExportConfig {
        database: "app".to_string(),
        tables: vec!["users".to_string()],
        ..ExportConfig::default()
    };
    let mut output = String::new();
    let events = Mutex::new(Vec::new());

    let rows = export_table_data_in_pages(
        &plugin,
        &connection,
        &config,
        "users",
        true,
        &mut output,
        &|event| events.lock().unwrap().push(event),
    )
    .await
    .expect("paged export should succeed");

    assert_eq!(1001, rows);
    assert!(output.is_empty());
    assert_eq!(
        vec![
            "SELECT * FROM `app`.`users` LIMIT 1000 OFFSET 0",
            "SELECT * FROM `app`.`users` LIMIT 1000 OFFSET 1000",
        ],
        connection.queries()
    );
    let events = events.lock().unwrap();
    assert_eq!(2, events.len());
    assert!(matches!(
        &events[0],
        ExportProgressEvent::DataExported { rows: 1000, data, .. }
            if data.contains("-- Data for table users") && data.contains("'user''0'")
    ));
    assert!(matches!(
        &events[1],
        ExportProgressEvent::DataExported { rows: 1, data, .. }
            if !data.contains("-- Data for table users") && data.contains("'user''1000'")
    ));
}
