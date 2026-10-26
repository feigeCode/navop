-- 云同步「数据范围」扩展所需的两张表
--
-- sync_item_state：逐条同步基线。把 cloud_id / checksum / 变更时间 / 上次同步时间
--   从业务表外置出来，这样 quick_commands / sftp_favorite_paths / llm_providers
--   不需要加列，笔记与已保存 SQL 这类文件系统数据也有地方存基线。
--   base_payload 仅供设置类 Singleton 做字段级三方合并（Q8）使用。
--
-- sync_type_status：按数据类型聚合的同步结果，供设置页每一行展示
--   「上次同步时间 / 最近失败警告」（Q12）。

CREATE TABLE IF NOT EXISTS sync_item_state (
    scope_env      TEXT NOT NULL,
    user_id        TEXT NOT NULL,
    data_type      TEXT NOT NULL,
    local_key      TEXT NOT NULL,
    cloud_id       TEXT,
    checksum       TEXT,
    content_ts     INTEGER NOT NULL DEFAULT 0,
    last_synced_at INTEGER,
    base_payload   TEXT,
    PRIMARY KEY (scope_env, user_id, data_type, local_key)
);

CREATE INDEX IF NOT EXISTS idx_sync_item_state_cloud
    ON sync_item_state (data_type, cloud_id);

CREATE TABLE IF NOT EXISTS sync_type_status (
    scope_env       TEXT NOT NULL,
    user_id         TEXT NOT NULL,
    data_type       TEXT NOT NULL,
    last_success_at INTEGER,
    last_error_code TEXT,
    last_error_at   INTEGER,
    last_error_msg  TEXT,
    PRIMARY KEY (scope_env, user_id, data_type)
);
