use crate::DatabasePlugin;
use crate::connection::DbConnection;
use crate::executor::{ExecOptions, SqlResult};
use crate::import_export::ImportConfig;

pub(super) struct ImportStatement {
    pub sql: String,
    pub error_prefix: String,
    pub counts_as_row: bool,
}

impl ImportStatement {
    pub fn truncate(sql: String) -> Self {
        Self {
            sql,
            error_prefix: "Truncate failed".to_string(),
            counts_as_row: false,
        }
    }

    pub fn row(sql: String, error_prefix: impl Into<String>) -> Self {
        Self {
            sql,
            error_prefix: error_prefix.into(),
            counts_as_row: true,
        }
    }
}

pub(super) async fn execute_import_statements(
    plugin: &dyn DatabasePlugin,
    connection: &dyn DbConnection,
    config: &ImportConfig,
    statements: Vec<ImportStatement>,
) -> (u64, Vec<String>) {
    if statements.is_empty() {
        return (0, Vec::new());
    }

    let script = statements
        .iter()
        .map(|statement| statement.sql.as_str())
        .collect::<Vec<_>>()
        .join(";\n");
    let options = ExecOptions {
        stop_on_error: config.stop_on_error,
        transactional: config.use_transaction,
        max_rows: None,
        streaming: false,
    };
    let results = match connection.execute(plugin, &script, options).await {
        Ok(results) => results,
        Err(error) => return (0, vec![format!("Import failed: {error}")]),
    };

    let mut rows_imported = 0;
    let mut errors = Vec::new();
    for (index, result) in results.into_iter().enumerate() {
        let context = statements.get(index);
        match result {
            SqlResult::Exec(result) if context.is_some_and(|item| item.counts_as_row) => {
                rows_imported += result.rows_affected;
            }
            SqlResult::Error(error) => {
                let prefix = context
                    .map(|item| item.error_prefix.as_str())
                    .unwrap_or("Import failed");
                errors.push(format!("{prefix}: {}", error.message));
            }
            SqlResult::Exec(_) | SqlResult::Query(_) => {}
        }
    }
    if config.use_transaction && !errors.is_empty() {
        rows_imported = 0;
    }
    (rows_imported, errors)
}
