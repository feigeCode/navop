CREATE TABLE IF NOT EXISTS sql_execution_history (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    connection_id TEXT NOT NULL,
    database_name TEXT,
    schema_name TEXT,
    status TEXT NOT NULL CHECK (status IN ('success', 'error')),
    sql TEXT NOT NULL,
    summary TEXT NOT NULL,
    details TEXT NOT NULL DEFAULT '[]',
    affected_rows INTEGER NOT NULL DEFAULT 0,
    returned_rows INTEGER,
    elapsed_ms INTEGER NOT NULL DEFAULT 0,
    executed_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_sql_execution_history_connection_time
ON sql_execution_history(connection_id, executed_at DESC, id DESC);

CREATE INDEX IF NOT EXISTS idx_sql_execution_history_time
ON sql_execution_history(executed_at DESC, id DESC);
