ALTER TABLE connections ADD COLUMN last_used_at INTEGER;

CREATE INDEX IF NOT EXISTS idx_connections_last_used_at ON connections(last_used_at);
