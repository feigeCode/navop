CREATE TABLE IF NOT EXISTS credential_entries (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    kind TEXT NOT NULL,
    username TEXT,
    password TEXT,
    private_key_path TEXT,
    private_key_content TEXT,
    passphrase TEXT,
    sync_enabled INTEGER NOT NULL DEFAULT 0,
    cloud_id TEXT,
    last_synced_at INTEGER,
    team_id TEXT,
    owner_id TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_credential_entries_name
    ON credential_entries(name);
CREATE INDEX IF NOT EXISTS idx_credential_entries_kind
    ON credential_entries(kind);
CREATE INDEX IF NOT EXISTS idx_credential_entries_cloud_id
    ON credential_entries(cloud_id);
CREATE INDEX IF NOT EXISTS idx_credential_entries_team_id
    ON credential_entries(team_id);
CREATE INDEX IF NOT EXISTS idx_credential_entries_owner_id
    ON credential_entries(owner_id);
