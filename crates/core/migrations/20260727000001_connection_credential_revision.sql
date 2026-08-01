-- Local, non-secret generation used to invalidate cached connection
-- authentication contexts without hashing or persisting secret material.
--
-- Existing records start at generation 1. Full connection-record rewrites
-- advance this value atomically; metadata-only updates such as last-used and
-- sync bookkeeping deliberately leave it unchanged.
ALTER TABLE connections
ADD COLUMN credential_revision INTEGER NOT NULL DEFAULT 1
CHECK (credential_revision > 0);
