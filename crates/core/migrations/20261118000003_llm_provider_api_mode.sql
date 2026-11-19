-- LLM provider 接口协议（issue #565）：让用户选择 /chat/completions 或 /responses。
-- 存 snake_case 字面量（chat_completions / responses）；NULL 表示默认 chat_completions。
ALTER TABLE llm_providers ADD COLUMN api_mode TEXT;
