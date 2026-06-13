//! 所有方法名常量。
//!
//! 这层不带任何逻辑,只是把 wire protocol 里出现过的 `method` 字符串集中起来,
//! 避免业务代码硬编码字面量(打错字编译都过)。
//!
//! 命名约定:`<namespace>/<action>`,namespace 全小写,action 用 snake_case。
//! 协议元方法以 `$/` 开头(借鉴 LSP 的命名,辨识度高)。

// -- 协议元方法 --
/// 取消任意 in-flight 请求,按 request id。
pub const CANCEL_REQUEST: &str = "$/cancelRequest";
/// 心跳/连通性检查,空 params 即可。
pub const PING: &str = "$/ping";

// -- 生命周期 --
pub const INIT: &str = "init";
pub const SHUTDOWN: &str = "shutdown";

// -- 连接管理 --
pub const CONN_TEST: &str = "conn/test";
pub const CONN_OPEN: &str = "conn/open";
pub const CONN_CLOSE: &str = "conn/close";
pub const CONN_PING: &str = "conn/ping";
pub const CONN_USE: &str = "conn/use";

// -- Schema 内省 --
pub const SCHEMA_DATABASES: &str = "schema/databases";
pub const SCHEMA_SCHEMAS: &str = "schema/schemas";
pub const SCHEMA_OBJECTS: &str = "schema/objects";
pub const SCHEMA_COLUMNS: &str = "schema/columns";
pub const SCHEMA_INDEXES: &str = "schema/indexes";
pub const SCHEMA_FOREIGN_KEYS: &str = "schema/foreign_keys";
pub const SCHEMA_CHECKS: &str = "schema/checks";
pub const SCHEMA_VIEWS: &str = "schema/views";
pub const SCHEMA_FUNCTIONS: &str = "schema/functions";
pub const SCHEMA_PROCEDURES: &str = "schema/procedures";
pub const SCHEMA_TRIGGERS: &str = "schema/triggers";
pub const SCHEMA_SEQUENCES: &str = "schema/sequences";
pub const SCHEMA_TYPES: &str = "schema/types";
pub const SCHEMA_VIEW_DEFINITION: &str = "schema/view_definition";
pub const SCHEMA_DUMP_DDL: &str = "schema/dump_ddl";

// -- 查询执行 --
pub const QUERY_START: &str = "query/start";
pub const CURSOR_FETCH: &str = "cursor/fetch";
pub const CURSOR_CANCEL: &str = "cursor/cancel";
pub const CURSOR_CLOSE: &str = "cursor/close";

// -- 非查询执行 --
pub const EXEC_RUN: &str = "exec/run";
pub const EXEC_BATCH: &str = "exec/batch";

// -- 事务 --
pub const TX_BEGIN: &str = "tx/begin";
pub const TX_COMMIT: &str = "tx/commit";
pub const TX_ROLLBACK: &str = "tx/rollback";
pub const TX_SAVEPOINT: &str = "tx/savepoint";
pub const TX_RELEASE: &str = "tx/release";

// -- SQL 工具 --
pub const SQL_PARSE: &str = "sql/parse";
pub const SQL_FORMAT: &str = "sql/format";
pub const SQL_EXPLAIN: &str = "sql/explain";
pub const SQL_BUILD: &str = "sql/build";

// -- 编辑器辅助 --
pub const COMPLETION_PROVIDE: &str = "completion/provide";
pub const LINT_ANALYZE: &str = "lint/analyze";

// -- DDL 构造 --
pub const DDL_BUILD: &str = "ddl/build";
pub const DDL_BUILD_CREATE_TABLE: &str = "ddl/build_create_table";
pub const DDL_BUILD_ALTER_TABLE: &str = "ddl/build_alter_table";
pub const DDL_BUILD_DROP: &str = "ddl/build_drop";

// -- 数据导入导出 --
pub const DATA_EXPORT: &str = "data/export";
pub const DATA_IMPORT_BEGIN: &str = "data/import_begin";
pub const DATA_IMPORT_CHUNK: &str = "data/import_chunk";
pub const DATA_IMPORT_COMMIT: &str = "data/import_commit";
pub const DATA_IMPORT_ABORT: &str = "data/import_abort";
pub const STREAM_READ: &str = "stream/read";
pub const STREAM_CLOSE: &str = "stream/close";

// -- Host API(扩展 → 宿主) --
pub const HOST_REQUEST_CREDENTIAL: &str = "host/request_credential";
pub const HOST_NOTIFY: &str = "host/notify";
pub const HOST_QUICK_PICK: &str = "host/quick_pick";
pub const HOST_CONFIRM: &str = "host/confirm";
pub const HOST_OPEN_VIEW: &str = "host/open_view";
pub const HOST_SSH_OPEN_TUNNEL: &str = "host/ssh/open_tunnel";
pub const HOST_STORAGE_GET: &str = "host/storage/get";
pub const HOST_STORAGE_SET: &str = "host/storage/set";
pub const HOST_LOG: &str = "host/log";

// -- 事件通知(扩展 → 宿主,无 id) --
pub const EVENT_CONN_LOST: &str = "conn/lost";
pub const EVENT_CONN_RESTORED: &str = "conn/restored";
pub const EVENT_LOG: &str = "log";
pub const EVENT_METRIC: &str = "metric";
pub const EVENT_WARNING: &str = "warning";
pub const EVENT_SCHEMA_CHANGED: &str = "schema_changed";

/// 判断一个 method 是否属于协议元方法(以 `$/` 开头)。
pub fn is_meta_method(method: &str) -> bool {
    method.starts_with("$/")
}

/// 协议里定义过的全部 method 名(用于校验驱动声明的 `methods` 是否拼写正确)。
pub const ALL_METHODS: &[&str] = &[
    CANCEL_REQUEST,
    PING,
    INIT,
    SHUTDOWN,
    CONN_TEST,
    CONN_OPEN,
    CONN_CLOSE,
    CONN_PING,
    CONN_USE,
    SCHEMA_DATABASES,
    SCHEMA_SCHEMAS,
    SCHEMA_OBJECTS,
    SCHEMA_COLUMNS,
    SCHEMA_INDEXES,
    SCHEMA_FOREIGN_KEYS,
    SCHEMA_CHECKS,
    SCHEMA_VIEWS,
    SCHEMA_FUNCTIONS,
    SCHEMA_PROCEDURES,
    SCHEMA_TRIGGERS,
    SCHEMA_SEQUENCES,
    SCHEMA_TYPES,
    SCHEMA_VIEW_DEFINITION,
    SCHEMA_DUMP_DDL,
    QUERY_START,
    CURSOR_FETCH,
    CURSOR_CANCEL,
    CURSOR_CLOSE,
    EXEC_RUN,
    EXEC_BATCH,
    TX_BEGIN,
    TX_COMMIT,
    TX_ROLLBACK,
    TX_SAVEPOINT,
    TX_RELEASE,
    SQL_PARSE,
    SQL_FORMAT,
    SQL_EXPLAIN,
    SQL_BUILD,
    COMPLETION_PROVIDE,
    LINT_ANALYZE,
    DDL_BUILD,
    DDL_BUILD_CREATE_TABLE,
    DDL_BUILD_ALTER_TABLE,
    DDL_BUILD_DROP,
    DATA_EXPORT,
    DATA_IMPORT_BEGIN,
    DATA_IMPORT_CHUNK,
    DATA_IMPORT_COMMIT,
    DATA_IMPORT_ABORT,
    STREAM_READ,
    STREAM_CLOSE,
];

/// 该 method 名是否是协议已知方法。驱动声明 `methods` 时用它过滤拼写错误。
pub fn is_known(method: &str) -> bool {
    ALL_METHODS.contains(&method)
}

/// 判断 driver/extension 的 method 声明是否可接受。
///
/// 协议标准 method 必须在 [`ALL_METHODS`] 中，避免 `schema/columns` 这类拼写错误
/// 悄悄变成“不支持”。扩展私有能力必须显式放到 `x/...` 命名空间，避免污染协议命名空间。
pub fn is_allowed_declaration(method: &str) -> bool {
    is_known(method)
        || method
            .strip_prefix("x/")
            .is_some_and(|private| !private.trim().is_empty())
}

/// 取 method 的 namespace 部分(`/` 之前)。返回空表示无 namespace 或元方法。
pub fn namespace(method: &str) -> &str {
    if let Some(meta) = method.strip_prefix("$/") {
        return meta.split('/').next().unwrap_or("");
    }
    method.split('/').next().unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancel_request_method_string() {
        assert_eq!(CANCEL_REQUEST, "$/cancelRequest");
    }

    #[test]
    fn lifecycle_methods() {
        assert_eq!(INIT, "init");
        assert_eq!(SHUTDOWN, "shutdown");
    }

    #[test]
    fn conn_methods_use_slash_separator() {
        assert!(CONN_OPEN.starts_with("conn/"));
        assert!(CONN_CLOSE.starts_with("conn/"));
        assert!(CONN_PING.starts_with("conn/"));
    }

    #[test]
    fn schema_methods_all_namespaced() {
        for m in [
            SCHEMA_DATABASES,
            SCHEMA_SCHEMAS,
            SCHEMA_OBJECTS,
            SCHEMA_COLUMNS,
            SCHEMA_INDEXES,
            SCHEMA_FOREIGN_KEYS,
            SCHEMA_CHECKS,
            SCHEMA_VIEWS,
            SCHEMA_FUNCTIONS,
            SCHEMA_PROCEDURES,
            SCHEMA_TRIGGERS,
            SCHEMA_SEQUENCES,
            SCHEMA_TYPES,
            SCHEMA_VIEW_DEFINITION,
            SCHEMA_DUMP_DDL,
        ] {
            assert!(m.starts_with("schema/"), "expected schema/* prefix: {m}");
        }
    }

    #[test]
    fn query_and_cursor_methods() {
        assert_eq!(QUERY_START, "query/start");
        assert!(CURSOR_FETCH.starts_with("cursor/"));
        assert!(CURSOR_CANCEL.starts_with("cursor/"));
        assert!(CURSOR_CLOSE.starts_with("cursor/"));
    }

    #[test]
    fn tx_methods_use_tx_namespace() {
        for m in [TX_BEGIN, TX_COMMIT, TX_ROLLBACK, TX_SAVEPOINT, TX_RELEASE] {
            assert!(m.starts_with("tx/"));
        }
    }

    #[test]
    fn host_methods_use_host_namespace() {
        for m in [
            HOST_REQUEST_CREDENTIAL,
            HOST_NOTIFY,
            HOST_QUICK_PICK,
            HOST_CONFIRM,
            HOST_OPEN_VIEW,
            HOST_STORAGE_GET,
            HOST_STORAGE_SET,
            HOST_LOG,
        ] {
            assert!(m.starts_with("host/"));
        }
        assert_eq!(HOST_SSH_OPEN_TUNNEL, "host/ssh/open_tunnel");
    }

    #[test]
    fn is_meta_method_detects_dollar_prefix() {
        assert!(is_meta_method(CANCEL_REQUEST));
        assert!(is_meta_method(PING));
        assert!(!is_meta_method(INIT));
        assert!(!is_meta_method(CONN_OPEN));
        assert!(!is_meta_method(""));
    }

    #[test]
    fn namespace_extracts_prefix() {
        assert_eq!(namespace(CONN_OPEN), "conn");
        assert_eq!(namespace(SCHEMA_DATABASES), "schema");
        assert_eq!(namespace(QUERY_START), "query");
        assert_eq!(namespace(TX_BEGIN), "tx");
    }

    #[test]
    fn namespace_for_meta_method() {
        // `$/cancelRequest` 没有 namespace,返回 "cancelRequest" 表示整段 meta name
        assert_eq!(namespace(CANCEL_REQUEST), "cancelRequest");
        assert_eq!(namespace(PING), "ping");
    }

    #[test]
    fn namespace_for_no_slash() {
        assert_eq!(namespace("init"), "init");
        assert_eq!(namespace(""), "");
    }

    #[test]
    fn is_known_accepts_defined_methods_rejects_typos() {
        assert!(is_known(SQL_FORMAT));
        assert!(is_known(SQL_BUILD));
        assert!(is_known(DDL_BUILD));
        assert!(is_known(DDL_BUILD_DROP));
        assert!(is_known(SCHEMA_COLUMNS));
        assert!(!is_known("sql/fromat"));
        assert!(!is_known("ddl/build_nonsense"));
        assert!(!is_known(""));
    }

    #[test]
    fn declaration_allows_known_protocol_and_private_extension_methods() {
        assert!(is_allowed_declaration(SCHEMA_COLUMNS));
        assert!(is_allowed_declaration("x/demo/profile"));
        assert!(!is_allowed_declaration("schema/colums"));
        assert!(!is_allowed_declaration("x/"));
        assert!(!is_allowed_declaration(""));
    }

    #[test]
    fn all_methods_has_no_duplicates() {
        let mut seen = std::collections::HashSet::new();
        for m in ALL_METHODS {
            assert!(seen.insert(*m), "duplicate method in ALL_METHODS: {m}");
        }
    }
}
