-- Agent 会话的上下文用量采样。
--
-- 与 `chat_sessions.snapshot_json.context_tokens` 的分工：快照里那份是「现在用了
-- 多少」的当前值，会被下一轮覆盖；这张表是**只追加**的流水，用于画趋势、按天汇总。
-- 两者都记，但语义不同——想画历史就只能靠流水。
CREATE TABLE IF NOT EXISTS agent_usage_samples (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    -- 会话 uid（与 chat_sessions.uid 对齐）。
    uid TEXT NOT NULL,
    -- 采样时刻的上下文占用（token）。
    used INTEGER NOT NULL,
    -- 模型上下文窗口；未知为 NULL，不猜。
    window INTEGER,
    -- 采样时的模型 id（用于按模型汇总）。
    model TEXT,
    recorded_at INTEGER NOT NULL
);

-- 按会话取最近若干条（每会话趋势）。
CREATE INDEX IF NOT EXISTS idx_agent_usage_samples_uid_time
ON agent_usage_samples(uid, recorded_at DESC, id DESC);

-- 按时间窗取全量（按天 / 按周汇总）。
CREATE INDEX IF NOT EXISTS idx_agent_usage_samples_time
ON agent_usage_samples(recorded_at DESC, id DESC);
