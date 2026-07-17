CREATE TABLE IF NOT EXISTS connection_folders (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    connection_type TEXT NOT NULL,
    parent_id INTEGER,
    sort_order INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    FOREIGN KEY (parent_id) REFERENCES connection_folders(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_connection_folders_type
    ON connection_folders(connection_type);

CREATE INDEX IF NOT EXISTS idx_connection_folders_parent
    ON connection_folders(parent_id);

ALTER TABLE connections ADD COLUMN folder_id INTEGER REFERENCES connection_folders(id) ON DELETE SET NULL;

CREATE INDEX IF NOT EXISTS idx_connections_folder_id
    ON connections(folder_id);
