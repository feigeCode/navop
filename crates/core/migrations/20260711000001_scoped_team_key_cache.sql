ALTER TABLE team_key_cache RENAME TO team_key_cache_unscoped;

CREATE TABLE team_key_cache (
    cloud_environment TEXT NOT NULL,
    user_id TEXT NOT NULL,
    team_id TEXT NOT NULL,
    team_name TEXT NOT NULL,
    key_version INTEGER NOT NULL DEFAULT 0,
    cached_key_version INTEGER,
    key_verification TEXT,
    encrypted_team_key TEXT,
    last_verified_at INTEGER,
    updated_at INTEGER NOT NULL,
    role TEXT,
    PRIMARY KEY (cloud_environment, user_id, team_id)
);

CREATE INDEX idx_team_key_cache_scope
    ON team_key_cache(cloud_environment, user_id, updated_at DESC);

DROP TABLE team_key_cache_unscoped;
