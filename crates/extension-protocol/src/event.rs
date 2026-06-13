//! 单向事件通知(notification,无 id,无应答)。
//!
//! 这些消息由**扩展进程**主动发出,host 路由到对应的订阅者。
//! 与 [`crate::envelope::Notification`] 的关系:`Notification` 是 envelope 层
//! (jsonrpc / method / params),这里定义业务层的 params 类型。
//!
//! 详见 [`docs/design/extensions/api-database.md`] §15。

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::conn::ConnId;
use crate::schema::ObjectKind;

// ============================================================================
// conn/lost / conn/restored
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnLostEvent {
    pub conn_id: ConnId,
    pub reason: String,
    /// 扩展是否在尝试自动重连。
    #[serde(default)]
    pub recoverable: bool,
    /// 已尝试重连次数。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retries: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnRestoredEvent {
    pub conn_id: ConnId,
    /// 重连耗时(毫秒)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elapsed_ms: Option<u32>,
}

// ============================================================================
// log
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEvent {
    pub level: LogLevel,
    /// 通常是扩展模块路径,如 `cass.driver`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub module: Option<String>,
    pub message: String,
    /// 结构化字段,与 `tracing` 风格一致。
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub fields: Value,
    /// 关联到具体 conn(可选,便于宿主路由到 conn 日志面板)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conn_id: Option<ConnId>,
    /// ISO 8601 时间戳(可选,默认 host 收到时打);避免时钟漂移建议留空。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
}

// ============================================================================
// metric
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricEvent {
    /// 指标名,推荐 dotted 命名:`query.latency_ms` / `conn.active_count`。
    pub name: String,
    /// 数值(f64,涵盖整数和浮点)。
    pub value: f64,
    /// 标签(host 转给可观测后端)。
    #[serde(default)]
    pub tags: std::collections::HashMap<String, String>,
    /// `counter` / `gauge` / `histogram`,默认 `gauge`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<MetricKind>,
    /// 单位(`ms` / `bytes` / `count`)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricKind {
    Counter,
    Gauge,
    Histogram,
}

// ============================================================================
// warning
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WarningEvent {
    /// 规则码,例如 `deprecated_syntax` / `slow_query`。
    pub code: String,
    pub message: String,
    /// 关联连接,可选。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conn_id: Option<ConnId>,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub extra: Value,
}

// ============================================================================
// schema_changed
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SchemaChangeKind {
    /// 新建。
    Created,
    /// 修改(结构变了)。
    Altered,
    /// 删除。
    Dropped,
    /// 重命名;`name` 是新名,旧名放 `previous_name`。
    Renamed,
    /// 不知道是什么改动,只是「确实变了,刷新一下」。
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaChangedEvent {
    pub conn_id: ConnId,
    /// 受影响的对象类型。
    pub kind: ObjectKind,
    /// 对象名(包含 schema/database qualifier,host 自行解析)。
    pub name: String,
    /// 改动类型。
    pub change: SchemaChangeKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_name: Option<String>,
    /// 改动来源:`extension` / `external`(其他客户端) / `dml_trigger`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub extra: Value,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conn_lost_event_with_retries() {
        let e = ConnLostEvent {
            conn_id: 17,
            reason: "TCP RST".into(),
            recoverable: true,
            retries: Some(2),
        };
        let j = serde_json::to_string(&e).unwrap();
        let parsed: ConnLostEvent = serde_json::from_str(&j).unwrap();
        assert_eq!(parsed.conn_id, 17);
        assert!(parsed.recoverable);
        assert_eq!(parsed.retries, Some(2));
    }

    #[test]
    fn conn_lost_event_default_recoverable_false() {
        let e: ConnLostEvent = serde_json::from_str(r#"{"conn_id":1,"reason":"x"}"#).unwrap();
        assert!(!e.recoverable);
        assert!(e.retries.is_none());
    }

    #[test]
    fn conn_restored_event_round_trip() {
        let e = ConnRestoredEvent {
            conn_id: 17,
            elapsed_ms: Some(1_234),
        };
        let j = serde_json::to_string(&e).unwrap();
        let parsed: ConnRestoredEvent = serde_json::from_str(&j).unwrap();
        assert_eq!(parsed.conn_id, 17);
        assert_eq!(parsed.elapsed_ms, Some(1_234));
    }

    #[test]
    fn log_level_serde_lowercase() {
        assert_eq!(
            serde_json::to_string(&LogLevel::Trace).unwrap(),
            r#""trace""#
        );
        assert_eq!(serde_json::to_string(&LogLevel::Warn).unwrap(), r#""warn""#);
        let parsed: LogLevel = serde_json::from_str(r#""error""#).unwrap();
        assert_eq!(parsed, LogLevel::Error);
    }

    #[test]
    fn log_event_with_fields_and_conn() {
        let e = LogEvent {
            level: LogLevel::Info,
            module: Some("driver.cass".into()),
            message: "query executed".into(),
            fields: serde_json::json!({"rows": 100, "ms": 5}),
            conn_id: Some(17),
            timestamp: None,
        };
        let j = serde_json::to_string(&e).unwrap();
        let parsed: LogEvent = serde_json::from_str(&j).unwrap();
        assert_eq!(parsed.level, LogLevel::Info);
        assert_eq!(parsed.module.as_deref(), Some("driver.cass"));
        assert_eq!(parsed.conn_id, Some(17));
        assert_eq!(parsed.fields, serde_json::json!({"rows": 100, "ms": 5}));
    }

    #[test]
    fn log_event_minimal_fields() {
        let e = LogEvent {
            level: LogLevel::Debug,
            module: None,
            message: "ping".into(),
            fields: Value::Null,
            conn_id: None,
            timestamp: None,
        };
        let j = serde_json::to_string(&e).unwrap();
        assert!(!j.contains("module"));
        assert!(!j.contains("fields"));
        assert!(!j.contains("conn_id"));
        assert!(!j.contains("timestamp"));
    }

    #[test]
    fn metric_kind_serde_snake_case() {
        assert_eq!(
            serde_json::to_string(&MetricKind::Counter).unwrap(),
            r#""counter""#
        );
        assert_eq!(
            serde_json::to_string(&MetricKind::Histogram).unwrap(),
            r#""histogram""#
        );
        let parsed: MetricKind = serde_json::from_str(r#""gauge""#).unwrap();
        assert_eq!(parsed, MetricKind::Gauge);
    }

    #[test]
    fn metric_event_with_tags_and_kind() {
        let mut tags = std::collections::HashMap::new();
        tags.insert("db".to_string(), "ks1".to_string());
        let e = MetricEvent {
            name: "query.latency_ms".into(),
            value: 123.4,
            tags,
            kind: Some(MetricKind::Histogram),
            unit: Some("ms".into()),
        };
        let j = serde_json::to_string(&e).unwrap();
        let parsed: MetricEvent = serde_json::from_str(&j).unwrap();
        assert_eq!(parsed.name, "query.latency_ms");
        assert!((parsed.value - 123.4).abs() < f64::EPSILON);
        assert_eq!(parsed.tags.len(), 1);
        assert_eq!(parsed.kind, Some(MetricKind::Histogram));
        assert_eq!(parsed.unit.as_deref(), Some("ms"));
    }

    #[test]
    fn metric_event_minimal() {
        let e = MetricEvent {
            name: "conn.active".into(),
            value: 3.0,
            tags: Default::default(),
            kind: None,
            unit: None,
        };
        let j = serde_json::to_string(&e).unwrap();
        let parsed: MetricEvent = serde_json::from_str(&j).unwrap();
        assert_eq!(parsed.value, 3.0);
        assert!(parsed.tags.is_empty());
        assert!(parsed.kind.is_none());
    }

    #[test]
    fn warning_event_round_trip() {
        let e = WarningEvent {
            code: "deprecated_syntax".into(),
            message: "BATCH ANY syntax deprecated".into(),
            conn_id: Some(17),
            extra: serde_json::json!({"line": 12}),
        };
        let j = serde_json::to_string(&e).unwrap();
        let parsed: WarningEvent = serde_json::from_str(&j).unwrap();
        assert_eq!(parsed.code, "deprecated_syntax");
        assert_eq!(parsed.conn_id, Some(17));
        assert_eq!(parsed.extra, serde_json::json!({"line": 12}));
    }

    #[test]
    fn schema_change_kind_serde() {
        assert_eq!(
            serde_json::to_string(&SchemaChangeKind::Created).unwrap(),
            r#""created""#
        );
        assert_eq!(
            serde_json::to_string(&SchemaChangeKind::Altered).unwrap(),
            r#""altered""#
        );
        assert_eq!(
            serde_json::to_string(&SchemaChangeKind::Renamed).unwrap(),
            r#""renamed""#
        );
        let parsed: SchemaChangeKind = serde_json::from_str(r#""dropped""#).unwrap();
        assert_eq!(parsed, SchemaChangeKind::Dropped);
    }

    #[test]
    fn schema_changed_event_for_create() {
        let e = SchemaChangedEvent {
            conn_id: 17,
            kind: ObjectKind::Table,
            name: "users".into(),
            change: SchemaChangeKind::Created,
            previous_name: None,
            source: Some("extension".into()),
            extra: Value::Null,
        };
        let j = serde_json::to_string(&e).unwrap();
        let parsed: SchemaChangedEvent = serde_json::from_str(&j).unwrap();
        assert_eq!(parsed.change, SchemaChangeKind::Created);
        assert_eq!(parsed.kind, ObjectKind::Table);
        assert_eq!(parsed.source.as_deref(), Some("extension"));
        assert!(parsed.previous_name.is_none());
    }

    #[test]
    fn schema_changed_event_for_rename() {
        let e = SchemaChangedEvent {
            conn_id: 17,
            kind: ObjectKind::Table,
            name: "users_v2".into(),
            change: SchemaChangeKind::Renamed,
            previous_name: Some("users".into()),
            source: Some("external".into()),
            extra: Value::Null,
        };
        let j = serde_json::to_string(&e).unwrap();
        assert!(j.contains(r#""previous_name":"users""#));
        let parsed: SchemaChangedEvent = serde_json::from_str(&j).unwrap();
        assert_eq!(parsed.change, SchemaChangeKind::Renamed);
        assert_eq!(parsed.previous_name.as_deref(), Some("users"));
    }
}
