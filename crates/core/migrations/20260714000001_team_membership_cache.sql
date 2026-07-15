CREATE TABLE IF NOT EXISTS team_membership_cache (
    cloud_environment TEXT NOT NULL,
    user_id TEXT NOT NULL,
    team_id TEXT NOT NULL,
    team_name TEXT NOT NULL,
    role TEXT,
    membership_state TEXT NOT NULL DEFAULT 'active'
        CHECK (membership_state IN ('active', 'departed', 'unknown')),
    last_seen_at INTEGER,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (cloud_environment, user_id, team_id)
);

CREATE INDEX IF NOT EXISTS idx_team_membership_cache_scope
    ON team_membership_cache(cloud_environment, user_id, membership_state, updated_at DESC);

INSERT OR IGNORE INTO team_membership_cache (
    cloud_environment, user_id, team_id, team_name, role,
    membership_state, last_seen_at, updated_at
)
SELECT cloud_environment, user_id, team_id, team_name, role,
       'active', updated_at, updated_at
FROM team_key_cache;
