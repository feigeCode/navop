ALTER TABLE quick_commands ADD COLUMN group_name TEXT;
ALTER TABLE quick_commands ADD COLUMN group_color TEXT;

CREATE INDEX IF NOT EXISTS idx_quick_commands_group_name ON quick_commands(group_name);
