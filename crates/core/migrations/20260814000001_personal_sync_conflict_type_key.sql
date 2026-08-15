BEGIN;

CREATE TABLE personal_sync_conflicts_v2 (
    backend_profile_id TEXT NOT NULL,
    record_id TEXT NOT NULL,
    data_type TEXT NOT NULL,
    conflict_type TEXT NOT NULL,
    local_snapshot TEXT,
    remote_snapshot TEXT,
    detected_at INTEGER NOT NULL,
    PRIMARY KEY (backend_profile_id, data_type, record_id)
);

INSERT INTO personal_sync_conflicts_v2
    (backend_profile_id, record_id, data_type, conflict_type, local_snapshot, remote_snapshot, detected_at)
SELECT
    backend_profile_id, record_id, data_type, conflict_type, local_snapshot, remote_snapshot, detected_at
FROM personal_sync_conflicts;

DROP TABLE personal_sync_conflicts;

ALTER TABLE personal_sync_conflicts_v2 RENAME TO personal_sync_conflicts;

COMMIT;
