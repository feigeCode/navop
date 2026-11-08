-- Extra request headers for OpenAI-compatible gateways (issue #505).
-- Stored as a JSON array of [name, value] pairs; NULL keeps legacy rows unchanged.
ALTER TABLE llm_providers ADD COLUMN extra_headers TEXT;
