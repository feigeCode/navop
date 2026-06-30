-- Agent runtime sessions reuse chat_sessions with agent-specific metadata.
ALTER TABLE chat_sessions ADD COLUMN session_kind TEXT NOT NULL DEFAULT 'chat';
ALTER TABLE chat_sessions ADD COLUMN uid TEXT;
ALTER TABLE chat_sessions ADD COLUMN snapshot_json TEXT;
ALTER TABLE chat_sessions ADD COLUMN archived INTEGER NOT NULL DEFAULT 0;

CREATE UNIQUE INDEX IF NOT EXISTS idx_chat_sessions_agent_uid
ON chat_sessions (uid)
WHERE session_kind = 'agent' AND uid IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_chat_sessions_kind_archived_updated_at
ON chat_sessions (session_kind, archived, updated_at DESC);
