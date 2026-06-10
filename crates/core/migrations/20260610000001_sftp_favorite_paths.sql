CREATE TABLE IF NOT EXISTS sftp_favorite_paths (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    connection_id INTEGER,
    connection_key TEXT NOT NULL,
    path TEXT NOT NULL,
    sort_order INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    FOREIGN KEY (connection_id) REFERENCES connections(id) ON DELETE CASCADE,
    UNIQUE(connection_key, path)
);

CREATE INDEX IF NOT EXISTS idx_sftp_favorite_paths_connection_key
    ON sftp_favorite_paths(connection_key, sort_order, created_at);

CREATE INDEX IF NOT EXISTS idx_sftp_favorite_paths_connection_id
    ON sftp_favorite_paths(connection_id);
