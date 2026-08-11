PRAGMA foreign_keys = OFF;

BEGIN;

CREATE TABLE workspaces_new (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    color TEXT,
    icon TEXT,
    cloud_id TEXT,
    last_synced_at INTEGER,
    sort_order INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    parent_id INTEGER REFERENCES workspaces_new(id) ON DELETE SET NULL,
    sidebar_collapsed INTEGER NOT NULL DEFAULT 0
);

INSERT INTO workspaces_new (
    id,
    name,
    color,
    icon,
    cloud_id,
    last_synced_at,
    sort_order,
    created_at,
    updated_at,
    parent_id,
    sidebar_collapsed
)
SELECT
    id,
    name,
    color,
    icon,
    cloud_id,
    last_synced_at,
    sort_order,
    created_at,
    updated_at,
    parent_id,
    sidebar_collapsed
FROM workspaces;

DROP TABLE workspaces;
ALTER TABLE workspaces_new RENAME TO workspaces;

CREATE INDEX idx_workspaces_name ON workspaces(name);
CREATE INDEX idx_workspaces_cloud_id ON workspaces(cloud_id);
CREATE INDEX idx_workspaces_parent_id ON workspaces(parent_id);
CREATE UNIQUE INDEX idx_workspaces_root_name_unique
    ON workspaces(name)
    WHERE parent_id IS NULL;
CREATE UNIQUE INDEX idx_workspaces_parent_name_unique
    ON workspaces(parent_id, name)
    WHERE parent_id IS NOT NULL;

COMMIT;

PRAGMA foreign_keys = ON;
