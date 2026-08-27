BEGIN;

-- Redis 连接未填写用户名时，把 params JSON 中的 username 从 null 规范化为空字符串。
-- 此前存的 null 在构造连接 URL 时会被当作 default 用户认证（AUTH default <pass>），
-- 在不支持/不期望用户名认证的 Redis 上会导致连接失败或 30 秒超时。
-- 新保存的连接已由表单改为存储空字符串，此迁移仅清理历史存量数据。
UPDATE connections
SET params = json_set(params, '$.username', '')
WHERE connection_type = 'Redis'
  AND json_type(params, '$.username') = 'null';

COMMIT;
