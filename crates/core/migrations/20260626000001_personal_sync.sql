CREATE TABLE IF NOT EXISTS personal_sync_conflicts (
    backend_profile_id TEXT NOT NULL,
    record_id TEXT NOT NULL,
    data_type TEXT NOT NULL,
    conflict_type TEXT NOT NULL,
    local_snapshot TEXT,
    remote_snapshot TEXT,
    detected_at INTEGER NOT NULL,
    PRIMARY KEY (backend_profile_id, record_id)
);

CREATE TABLE IF NOT EXISTS personal_sync_status (
    backend_profile_id TEXT PRIMARY KEY,
    health TEXT NOT NULL,
    last_success_at INTEGER,
    last_error TEXT,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS personal_sync_retry_queue (
    backend_profile_id TEXT NOT NULL,
    operation_key TEXT NOT NULL,
    retry_count INTEGER NOT NULL,
    next_retry_at INTEGER,
    last_error TEXT,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (backend_profile_id, operation_key)
);
