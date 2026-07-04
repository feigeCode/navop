CREATE TABLE IF NOT EXISTS terminal_command_history (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    scope_key TEXT NOT NULL,
    scope_kind TEXT NOT NULL,
    connection_id INTEGER,
    command TEXT NOT NULL,
    use_count INTEGER NOT NULL DEFAULT 1,
    favorite INTEGER NOT NULL DEFAULT 0,
    first_used_at INTEGER NOT NULL,
    last_used_at INTEGER NOT NULL,
    last_exit_code INTEGER,
    cwd TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE(scope_key, command)
);

CREATE INDEX IF NOT EXISTS idx_terminal_command_history_scope_latest
ON terminal_command_history(scope_key, favorite DESC, last_used_at DESC, id DESC);

CREATE INDEX IF NOT EXISTS idx_terminal_command_history_scope_used
ON terminal_command_history(scope_key, favorite DESC, use_count DESC, last_used_at DESC, id DESC);

CREATE INDEX IF NOT EXISTS idx_terminal_command_history_command
ON terminal_command_history(command);
