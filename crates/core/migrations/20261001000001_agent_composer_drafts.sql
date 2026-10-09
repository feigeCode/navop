-- 未发送的输入草稿（每个会话一条）。
--
-- 与 `chat_sessions.snapshot_json` 里的草稿字段分工：快照那份是**随消息一起**写盘的
-- 附带信息，只有「会话有历史」时才存在——刚开的新会话里写了半句话，快照根本没地方放。
-- 这张表独立于历史，只要用户打过字就有行，附件（base64 图片）也一起存在这里。
--
-- 空草稿不留行：`text` 与 `attachments` 都空时删行，行的存在本身即「这条会话有草稿」，
-- 会话列表要标记「有草稿」时直接按 uid 查这张表，不必把每条快照反序列化一遍。
CREATE TABLE IF NOT EXISTS agent_composer_drafts (
    -- 会话 uid（与 chat_sessions.uid 对齐）。
    session_uid TEXT PRIMARY KEY,
    -- 输入框里的文字原样（含 `@提及` 原文）。
    text TEXT NOT NULL DEFAULT '',
    -- 附件 JSON 数组（图片按 base64 存）；结构由 ai_chat_view 侧定义，这里只当不透明串。
    attachments TEXT NOT NULL DEFAULT '[]',
    -- 最后编辑时刻（Unix 秒）。
    updated_at INTEGER NOT NULL
);

-- 按更新时间清理 / 排序时用。
CREATE INDEX IF NOT EXISTS idx_agent_composer_drafts_updated
ON agent_composer_drafts(updated_at DESC);
