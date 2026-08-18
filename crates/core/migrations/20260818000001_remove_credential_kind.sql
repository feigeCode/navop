BEGIN;

DROP INDEX IF EXISTS idx_credential_entries_kind;
ALTER TABLE credential_entries DROP COLUMN kind;

COMMIT;
