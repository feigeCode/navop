-- Default reasoning effort per provider.
-- Stored as one of "none"/"low"/"medium"/"high"; NULL keeps the upstream default.
ALTER TABLE llm_providers ADD COLUMN reasoning_effort TEXT;
