ALTER TABLE workspaces ADD COLUMN parent_id INTEGER REFERENCES workspaces(id) ON DELETE SET NULL;

CREATE INDEX IF NOT EXISTS idx_workspaces_parent_id ON workspaces(parent_id);
