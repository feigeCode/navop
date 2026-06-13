//! 握手与生命周期。
//!
//! `init` 是协议的第一次往返:host 告诉扩展自己是谁、声明可提供哪些 API,
//! 扩展回报自己接受哪些 API、实际具备哪些 feature。
//!
//! `shutdown` 给扩展机会优雅退出(完成挂起请求、关闭文件 / socket);超过
//! `grace_ms` 后宿主会 SIGKILL。
//!
//! 详见 [`docs/design/extensions/api-database.md`] §4。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// `init` 请求参数(host → ext)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InitParams {
    /// 宿主主程序版本(语义化版本字符串)。
    pub host_version: String,
    /// 宿主声明可以提供的 API 命名空间 → 版本字符串。
    ///
    /// 例如 `{"database": "1.2", "ui": "1.0"}`。扩展据此选择实际使用哪些 API。
    pub api_offered: HashMap<String, String>,
    /// 本次扩展实例的唯一 id(UUID 字符串),用于日志关联与崩溃归因。
    pub instance_id: String,
    /// 启动配置。
    pub config: InitConfig,
}

impl InitParams {
    pub fn new(host_version: impl Into<String>, instance_id: impl Into<String>) -> Self {
        Self {
            host_version: host_version.into(),
            api_offered: HashMap::new(),
            instance_id: instance_id.into(),
            config: InitConfig::default(),
        }
    }

    pub fn with_api(mut self, name: impl Into<String>, version: impl Into<String>) -> Self {
        self.api_offered.insert(name.into(), version.into());
        self
    }
}

/// init 启动配置。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InitConfig {
    /// 日志级别(trace/debug/info/warn/error)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_level: Option<String>,
    /// 扩展可写的临时目录(沙盒文件系统根)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    /// 用户语言(BCP-47,例如 `zh-CN` / `en-US`)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
    /// 任意宿主提供的额外参数(代理设置、特性开关等)。
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub extra: Value,
}

/// `init` 响应结果(ext → host)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InitResult {
    /// 扩展自身版本。
    pub extension_version: String,
    /// 扩展实际使用了哪些 API(选自 [`InitParams::api_offered`])。
    pub api_used: HashMap<String, String>,
    /// 该扩展支持的 features(用 [`Capability`] 常量字符串)。
    #[serde(default)]
    pub features: Vec<String>,
    /// 该扩展实际实现的 wire method 全名列表(如 `"sql/explain"`、`"ddl/build_drop"`)。
    ///
    /// 比 [`features`](Self::features) 粒度更细:`features` 只能表达「我支持 sql 工具类」,
    /// `methods` 能精确到「我实现了 `sql/format` 但没实现 `sql/explain`」。宿主以本字段
    /// 为权威(运行时动态),静态 manifest 声明仅作启动前 / 兼容回退。空表示未声明
    /// (legacy 模式:照常调用,`METHOD_NOT_FOUND` 后回退)。
    #[serde(default)]
    pub methods: Vec<String>,
    /// 此扩展实例 ready 的 driver id 列表。
    #[serde(default)]
    pub drivers_ready: Vec<String>,
    /// 任意扩展自定义的初始化输出(供宿主写日志)。
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub extra: Value,
}

impl InitResult {
    pub fn new(extension_version: impl Into<String>) -> Self {
        Self {
            extension_version: extension_version.into(),
            api_used: HashMap::new(),
            features: Vec::new(),
            methods: Vec::new(),
            drivers_ready: Vec::new(),
            extra: Value::Null,
        }
    }

    pub fn with_api(mut self, name: impl Into<String>, version: impl Into<String>) -> Self {
        self.api_used.insert(name.into(), version.into());
        self
    }

    pub fn with_feature(mut self, cap: impl Into<String>) -> Self {
        self.features.push(cap.into());
        self
    }

    /// 声明实现了某个 wire method(全名,如 [`method::SQL_EXPLAIN`](crate::method)).
    pub fn with_method(mut self, method: impl Into<String>) -> Self {
        self.methods.push(method.into());
        self
    }

    pub fn with_driver(mut self, driver_id: impl Into<String>) -> Self {
        self.drivers_ready.push(driver_id.into());
        self
    }

    /// 是否声明了某个 capability。
    pub fn has_feature(&self, cap: &str) -> bool {
        self.features.iter().any(|f| f == cap)
    }

    /// 是否声明实现了某个 wire method。
    pub fn declares_method(&self, method: &str) -> bool {
        self.methods.iter().any(|m| m == method)
    }
}

/// `shutdown` 请求参数。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShutdownParams {
    /// 优雅关闭时长(毫秒)。超时后宿主强制 kill。
    #[serde(default = "default_grace_ms")]
    pub grace_ms: u32,
}

fn default_grace_ms() -> u32 {
    30_000
}

impl Default for ShutdownParams {
    fn default() -> Self {
        Self {
            grace_ms: default_grace_ms(),
        }
    }
}

/// 常用的 Capability(`init` 响应里的 `features` 字符串值)。
///
/// 这是一个语义化的字符串常量集合,而不是 enum——扩展可以声明宿主未知的
/// capability,宿主在 features 交集里取自己认识的那些来决定后续路径。
pub struct Capability;

impl Capability {
    /// 支持流式 cursor(`cursor/fetch` 多次拉取)。
    pub const STREAMING: &'static str = "streaming";
    /// 支持 `$/cancelRequest` 取消任意 in-flight 请求。
    pub const CANCEL_REQUEST: &'static str = "cancel_request";
    /// 参数化 SQL 支持命名参数(`:name` / `$name`)。
    pub const NAMED_PARAMS: &'static str = "named_params";
    /// 支持丰富的错误结构(`ErrorData` 各 fields 都可用)。
    pub const RICH_ERRORS: &'static str = "rich_errors";
    /// 支持事务(`tx/begin` 系列)。
    pub const TRANSACTIONS: &'static str = "transactions";
    /// 支持事务嵌套(SAVEPOINT)。
    pub const NESTED_TRANSACTIONS: &'static str = "nested_transactions";
    /// 支持 `exec/batch` 批量执行。
    pub const BATCH_EXEC: &'static str = "batch_exec";
    /// 支持 `data/export` + `data/import_*` 数据流。
    pub const DATA_PIPE: &'static str = "data_pipe";
    /// 支持 `sql/parse`、`sql/format`、`sql/build`。
    pub const SQL_TOOLS: &'static str = "sql_tools";
    /// 支持 `completion/provide`。
    pub const COMPLETION: &'static str = "completion";
    /// 支持 `lint/analyze`。
    pub const LINT: &'static str = "lint";
    /// 支持 `ddl/build` 和 legacy `ddl/build_*`。
    pub const DDL_BUILDER: &'static str = "ddl_builder";
    /// 支持 schema 内省的全部方法。
    pub const SCHEMA_INTROSPECTION: &'static str = "schema_introspection";
    /// 支持通过 `host/ssh/open_tunnel` 申请隧道。
    pub const SSH_TUNNEL: &'static str = "ssh_tunnel";
    /// 支持服务器端游标(大结果集分页)。
    pub const SERVER_CURSOR: &'static str = "server_cursor";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_params_new_sets_defaults() {
        let p = InitParams::new("1.0.0", "abc");
        assert_eq!(p.host_version, "1.0.0");
        assert_eq!(p.instance_id, "abc");
        assert!(p.api_offered.is_empty());
        assert!(p.config.log_level.is_none());
    }

    #[test]
    fn init_params_with_api_chains() {
        let p = InitParams::new("1.0.0", "id")
            .with_api("database", "1.2")
            .with_api("ui", "1.0");
        assert_eq!(p.api_offered.len(), 2);
        assert_eq!(
            p.api_offered.get("database").map(String::as_str),
            Some("1.2")
        );
        assert_eq!(p.api_offered.get("ui").map(String::as_str), Some("1.0"));
    }

    #[test]
    fn init_params_round_trip() {
        let p = InitParams::new("1.4.2", "550e")
            .with_api("database", "1.2")
            .with_api("ui", "1.0");
        let s = serde_json::to_string(&p).unwrap();
        let parsed: InitParams = serde_json::from_str(&s).unwrap();
        assert_eq!(parsed.host_version, "1.4.2");
        assert_eq!(parsed.instance_id, "550e");
        assert_eq!(parsed.api_offered.len(), 2);
    }

    #[test]
    fn init_config_serializes_skip_none() {
        let c = InitConfig::default();
        let s = serde_json::to_string(&c).unwrap();
        // 全空时不该有 log_level / workspace / locale / extra
        assert!(!s.contains("log_level"));
        assert!(!s.contains("workspace"));
        assert!(!s.contains("locale"));
        assert!(!s.contains("extra"));
    }

    #[test]
    fn init_config_with_fields() {
        let c = InitConfig {
            log_level: Some("info".into()),
            workspace: Some("/tmp/x".into()),
            locale: Some("zh-CN".into()),
            extra: Value::Null,
        };
        let s = serde_json::to_string(&c).unwrap();
        assert!(s.contains(r#""log_level":"info""#));
        assert!(s.contains(r#""workspace":"/tmp/x""#));
        assert!(s.contains(r#""locale":"zh-CN""#));
    }

    #[test]
    fn init_result_builder_chains() {
        let r = InitResult::new("1.2.0")
            .with_api("database", "1.2")
            .with_feature(Capability::STREAMING)
            .with_feature(Capability::CANCEL_REQUEST)
            .with_driver("cassandra");
        assert_eq!(r.extension_version, "1.2.0");
        assert_eq!(r.api_used.len(), 1);
        assert_eq!(r.features.len(), 2);
        assert_eq!(r.drivers_ready, vec!["cassandra".to_string()]);
    }

    #[test]
    fn init_result_has_feature_query() {
        let r = InitResult::new("1.0.0").with_feature(Capability::STREAMING);
        assert!(r.has_feature(Capability::STREAMING));
        assert!(!r.has_feature(Capability::TRANSACTIONS));
    }

    #[test]
    fn init_result_methods_round_trip_and_default_empty() {
        // 未带 methods 字段的旧响应仍能解析(legacy),methods 为空。
        let legacy: InitResult =
            serde_json::from_str(r#"{"extension_version":"1.0.0","api_used":{}}"#).unwrap();
        assert!(legacy.methods.is_empty());

        let r = InitResult::new("1.2.0")
            .with_method(crate::method::SQL_FORMAT)
            .with_method(crate::method::DDL_BUILD_DROP);
        assert!(r.declares_method(crate::method::SQL_FORMAT));
        assert!(!r.declares_method(crate::method::SQL_EXPLAIN));

        let s = serde_json::to_string(&r).unwrap();
        let parsed: InitResult = serde_json::from_str(&s).unwrap();
        assert_eq!(
            parsed.methods,
            vec!["sql/format".to_string(), "ddl/build_drop".to_string()]
        );
    }

    #[test]
    fn init_result_round_trip() {
        let r = InitResult::new("1.2.0")
            .with_api("database", "1.2")
            .with_feature(Capability::STREAMING)
            .with_driver("d1");
        let s = serde_json::to_string(&r).unwrap();
        let parsed: InitResult = serde_json::from_str(&s).unwrap();
        assert_eq!(parsed.extension_version, "1.2.0");
        assert_eq!(
            parsed.api_used.get("database").map(String::as_str),
            Some("1.2")
        );
        assert_eq!(parsed.features, vec!["streaming".to_string()]);
        assert_eq!(parsed.drivers_ready, vec!["d1".to_string()]);
    }

    #[test]
    fn shutdown_params_default_grace_30s() {
        let s = ShutdownParams::default();
        assert_eq!(s.grace_ms, 30_000);
    }

    #[test]
    fn shutdown_params_round_trip() {
        let s = ShutdownParams { grace_ms: 5_000 };
        let ser = serde_json::to_string(&s).unwrap();
        let parsed: ShutdownParams = serde_json::from_str(&ser).unwrap();
        assert_eq!(parsed.grace_ms, 5_000);
    }

    #[test]
    fn shutdown_params_default_value_on_missing() {
        let parsed: ShutdownParams = serde_json::from_str("{}").unwrap();
        assert_eq!(parsed.grace_ms, 30_000);
    }

    #[test]
    fn capability_constants_are_stable_strings() {
        // 这层是 wire 协议契约,改字面量等于破坏兼容性,锁住值。
        assert_eq!(Capability::STREAMING, "streaming");
        assert_eq!(Capability::CANCEL_REQUEST, "cancel_request");
        assert_eq!(Capability::NAMED_PARAMS, "named_params");
        assert_eq!(Capability::RICH_ERRORS, "rich_errors");
        assert_eq!(Capability::TRANSACTIONS, "transactions");
        assert_eq!(Capability::SSH_TUNNEL, "ssh_tunnel");
    }
}
