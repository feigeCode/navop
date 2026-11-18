-- Per-provider proxy override (issue: provider-level AI proxy).
-- Stored as a URL string; NULL means "inherit the global proxy setting".
ALTER TABLE llm_providers ADD COLUMN proxy TEXT;
