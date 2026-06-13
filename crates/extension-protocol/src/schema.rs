//! Schema 内省 (`schema/*`)。
//!
//! 所有方法返回结构化对象,host 直接渲染为树 / 表格。Driver 的私有信息走
//! `extra` 字段(serde_json::Value),不影响 host 上层的通用渲染。
//!
//! 详见 [`docs/design/extensions/api-database.md`] §6。

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::conn::ConnId;

// ============================================================================
// Database / Schema 列表
// ============================================================================

/// `schema/databases` 请求参数。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabasesParams {
    pub conn_id: ConnId,
}

/// 单个 database 描述。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DatabaseInfo {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub charset: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collation: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub comment: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub extra: Value,
}

/// `schema/schemas` 请求参数。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemasParams {
    pub conn_id: ConnId,
    pub database: String,
}

/// 单个 schema 描述。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SchemaInfo {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub comment: String,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub extra: Value,
}

// ============================================================================
// Objects(表 / 视图 / 类型 / 函数 / 触发器)
// ============================================================================

/// 对象类型(`schema/objects` 里 `kinds` 过滤参数 & 返回的 `kind` 字段)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectKind {
    Table,
    View,
    MaterializedView,
    Type,
    Function,
    Procedure,
    Trigger,
    Sequence,
    Index,
    ForeignKey,
}

impl ObjectKind {
    /// 用作 `extension.json` / wire JSON 的 snake_case 字面量。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Table => "table",
            Self::View => "view",
            Self::MaterializedView => "materialized_view",
            Self::Type => "type",
            Self::Function => "function",
            Self::Procedure => "procedure",
            Self::Trigger => "trigger",
            Self::Sequence => "sequence",
            Self::Index => "index",
            Self::ForeignKey => "foreign_key",
        }
    }
}

/// `schema/objects` 请求参数。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjectsParams {
    pub conn_id: ConnId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    /// 过滤 kinds,空表示全部。
    #[serde(default)]
    pub kinds: Vec<ObjectKind>,
}

/// 通用对象描述,table / view / function / ... 都用同一结构,
/// kind-specific 字段放 `extra`。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjectInfo {
    pub name: String,
    pub kind: ObjectKind,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub comment: String,
    /// 估算行数(table),null 表示未知。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub row_count_estimate: Option<u64>,
    /// 表存储大小(bytes)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    /// ISO 8601 datetime。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub extra: Value,
}

// ============================================================================
// Columns
// ============================================================================

/// `schema/columns` 请求参数。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnsParams {
    pub conn_id: ConnId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    pub table: String,
}

/// 列详情(更详细的描述,与 [`crate::row::ColumnSpec`] 区分:
/// `ColumnSpec` 是查询结果集 schema,`ColumnInfo` 是表 schema)。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ColumnInfo {
    /// 列序号(1-based,匹配 SQL 标准)。
    pub ordinal: u32,
    pub name: String,
    /// 通用语义化类型,例如 `text` / `uuid` / `int4`。
    #[serde(rename = "type")]
    pub type_str: String,
    /// 驱动报出的原始类型(可能带长度/精度)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_type: Option<String>,
    pub nullable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    /// 是否参与主键。
    #[serde(default)]
    pub is_primary: bool,
    #[serde(default)]
    pub is_unique: bool,
    /// Cassandra 等列存数据库特有:分区键 / 聚簇键。
    #[serde(default)]
    pub is_partition_key: bool,
    #[serde(default)]
    pub is_clustering_key: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_length: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub precision: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<i32>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub comment: String,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub extra: Value,
}

// ============================================================================
// 索引 / 外键 / 视图 / 函数 / 过程 / 触发器 / 序列 / 类型
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexesParams {
    pub conn_id: ConnId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    pub table: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IndexInfo {
    pub name: String,
    pub table: String,
    pub columns: Vec<String>,
    /// `btree` / `hash` / `gin` / `gist` / `fulltext` / ...
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default)]
    pub is_unique: bool,
    #[serde(default)]
    pub is_primary: bool,
    /// 部分索引条件,例如 `WHERE deleted_at IS NULL`。
    #[serde(rename = "where", default, skip_serializing_if = "Option::is_none")]
    pub where_clause: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub comment: String,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub extra: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForeignKeysParams {
    pub conn_id: ConnId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    pub table: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ForeignKeyInfo {
    pub name: String,
    pub from_table: String,
    pub from_columns: Vec<String>,
    pub to_table: String,
    pub to_columns: Vec<String>,
    /// `cascade` / `restrict` / `set_null` / `set_default` / `no_action`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_delete: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_update: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub comment: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChecksParams {
    pub conn_id: ConnId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    pub table: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CheckInfo {
    pub name: String,
    pub table: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub definition: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub comment: String,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub extra: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewsParams {
    pub conn_id: ConnId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
}

/// 视图描述(包含 materialized view)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewInfo {
    pub name: String,
    pub kind: ObjectKind, // View | MaterializedView
    pub definition_sql: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub comment: String,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub extra: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionsParams {
    pub conn_id: ConnId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FunctionInfo {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub return_type: Option<String>,
    #[serde(default)]
    pub args: Vec<FunctionArg>,
    /// PL/SQL / SQL / JS / ... 函数体语言。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub definition: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub comment: String,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub extra: Value,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FunctionArg {
    pub name: String,
    #[serde(rename = "type")]
    pub type_str: String,
    /// `in` / `out` / `inout`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProceduresParams {
    pub conn_id: ConnId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
}

pub type ProcedureInfo = FunctionInfo; // 同结构,语义不同

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggersParams {
    pub conn_id: ConnId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub table: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TriggerInfo {
    pub name: String,
    pub table: String,
    /// `before` / `after` / `instead_of`。
    pub timing: String,
    /// `insert` / `update` / `delete` / `truncate`,可能是多个。
    pub event: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub definition: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub comment: String,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub extra: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SequencesParams {
    pub conn_id: ConnId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SequenceInfo {
    pub name: String,
    /// 序列起点。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_value: Option<i64>,
    /// 步长(允许负数)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub increment: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_value: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_value: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_value: Option<i64>,
    #[serde(default)]
    pub cycle: bool,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub extra: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypesParams {
    pub conn_id: ConnId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TypeInfo {
    pub name: String,
    /// `enum` / `composite` / `domain` / `range` / `udt` / ...。
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub definition: Option<String>,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub extra: Value,
}

// ============================================================================
// View Definition / Dump DDL
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewDefinitionParams {
    pub conn_id: ConnId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    pub view: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewDefinitionResult {
    pub sql: String,
    #[serde(default)]
    pub is_materialized: bool,
}

/// 一个 dump 目标对象引用。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjectRef {
    pub kind: ObjectKind,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database: Option<String>,
}

/// `schema/dump_ddl` 请求参数。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DumpDdlParams {
    pub conn_id: ConnId,
    pub objects: Vec<ObjectRef>,
    #[serde(default)]
    pub options: DumpDdlOptions,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DumpDdlOptions {
    #[serde(default)]
    pub if_not_exists: bool,
    #[serde(default)]
    pub with_indexes: bool,
    #[serde(default)]
    pub with_foreign_keys: bool,
    #[serde(default)]
    pub with_triggers: bool,
    #[serde(default)]
    pub with_comments: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DumpDdlResult {
    /// 顺序的 SQL 语句列表(每条独立,host 拼接成脚本时自行加分号)。
    pub statements: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_kind_serde_snake_case() {
        assert_eq!(
            serde_json::to_string(&ObjectKind::MaterializedView).unwrap(),
            r#""materialized_view""#
        );
        assert_eq!(
            serde_json::to_string(&ObjectKind::ForeignKey).unwrap(),
            r#""foreign_key""#
        );
        let parsed: ObjectKind = serde_json::from_str(r#""table""#).unwrap();
        assert_eq!(parsed, ObjectKind::Table);
    }

    #[test]
    fn object_kind_as_str_matches_serde() {
        for k in [
            ObjectKind::Table,
            ObjectKind::View,
            ObjectKind::MaterializedView,
            ObjectKind::Type,
            ObjectKind::Function,
            ObjectKind::Procedure,
            ObjectKind::Trigger,
            ObjectKind::Sequence,
            ObjectKind::Index,
            ObjectKind::ForeignKey,
        ] {
            let from_serde = serde_json::to_string(&k).unwrap();
            // 去掉双引号
            let trimmed = from_serde.trim_matches('"');
            assert_eq!(trimmed, k.as_str(), "as_str() mismatch for {k:?}");
        }
    }

    #[test]
    fn databases_params_round_trip() {
        let p = DatabasesParams { conn_id: 17 };
        let j = serde_json::to_string(&p).unwrap();
        assert_eq!(j, r#"{"conn_id":17}"#);
        let parsed: DatabasesParams = serde_json::from_str(&j).unwrap();
        assert_eq!(parsed.conn_id, 17);
    }

    #[test]
    fn database_info_round_trip() {
        let d = DatabaseInfo {
            name: "ks1".into(),
            owner: Some("cassandra".into()),
            size_bytes: Some(1234567),
            extra: serde_json::json!({"replication": "{'class':'SimpleStrategy'}"}),
            ..Default::default()
        };
        let j = serde_json::to_string(&d).unwrap();
        let parsed: DatabaseInfo = serde_json::from_str(&j).unwrap();
        assert_eq!(parsed.name, "ks1");
        assert_eq!(parsed.owner.as_deref(), Some("cassandra"));
        assert_eq!(parsed.size_bytes, Some(1234567));
    }

    #[test]
    fn database_info_skips_empty_comment() {
        let d = DatabaseInfo {
            name: "db".into(),
            comment: String::new(),
            ..Default::default()
        };
        let j = serde_json::to_string(&d).unwrap();
        assert!(!j.contains("comment"));
    }

    #[test]
    fn objects_params_with_kinds_filter() {
        let p = ObjectsParams {
            conn_id: 1,
            database: Some("db".into()),
            schema: None,
            kinds: vec![ObjectKind::Table, ObjectKind::View],
        };
        let j = serde_json::to_string(&p).unwrap();
        assert!(j.contains(r#""kinds":["table","view"]"#));
    }

    #[test]
    fn objects_params_default_kinds_empty() {
        let p: ObjectsParams = serde_json::from_str(r#"{"conn_id":1}"#).unwrap();
        assert_eq!(p.conn_id, 1);
        assert!(p.kinds.is_empty());
        assert!(p.database.is_none());
    }

    #[test]
    fn object_info_round_trip() {
        let o = ObjectInfo {
            name: "users".into(),
            kind: ObjectKind::Table,
            comment: String::new(),
            row_count_estimate: Some(12345),
            size_bytes: Some(9876543),
            created_at: Some("2024-01-01T00:00:00Z".into()),
            updated_at: None,
            extra: Value::Null,
        };
        let j = serde_json::to_string(&o).unwrap();
        assert!(j.contains(r#""kind":"table""#));
        assert!(j.contains(r#""row_count_estimate":12345"#));
        assert!(!j.contains("updated_at"));
        assert!(!j.contains("comment"));
        let parsed: ObjectInfo = serde_json::from_str(&j).unwrap();
        assert_eq!(parsed.name, "users");
    }

    #[test]
    fn column_info_full_round_trip() {
        let c = ColumnInfo {
            ordinal: 1,
            name: "id".into(),
            type_str: "uuid".into(),
            raw_type: Some("uuid".into()),
            nullable: false,
            default: None,
            is_primary: true,
            is_unique: true,
            is_partition_key: true,
            is_clustering_key: false,
            max_length: None,
            precision: None,
            scale: None,
            comment: String::new(),
            extra: serde_json::json!({"kind": "partition"}),
        };
        let j = serde_json::to_string(&c).unwrap();
        let parsed: ColumnInfo = serde_json::from_str(&j).unwrap();
        assert_eq!(parsed.ordinal, 1);
        assert_eq!(parsed.name, "id");
        assert!(parsed.is_primary);
        assert!(parsed.is_partition_key);
        assert_eq!(parsed.extra, serde_json::json!({"kind": "partition"}));
    }

    #[test]
    fn index_info_with_where_clause() {
        let i = IndexInfo {
            name: "idx_active".into(),
            table: "users".into(),
            columns: vec!["id".into()],
            kind: Some("btree".into()),
            is_unique: true,
            is_primary: false,
            where_clause: Some("deleted_at IS NULL".into()),
            comment: String::new(),
            extra: Value::Null,
        };
        let j = serde_json::to_string(&i).unwrap();
        assert!(j.contains(r#""where":"deleted_at IS NULL""#));
        let parsed: IndexInfo = serde_json::from_str(&j).unwrap();
        assert_eq!(parsed.where_clause.as_deref(), Some("deleted_at IS NULL"));
    }

    #[test]
    fn foreign_key_info_round_trip() {
        let fk = ForeignKeyInfo {
            name: "fk_user".into(),
            from_table: "orders".into(),
            from_columns: vec!["user_id".into()],
            to_table: "users".into(),
            to_columns: vec!["id".into()],
            on_delete: Some("cascade".into()),
            on_update: None,
            comment: String::new(),
        };
        let j = serde_json::to_string(&fk).unwrap();
        let parsed: ForeignKeyInfo = serde_json::from_str(&j).unwrap();
        assert_eq!(parsed.from_columns, vec!["user_id".to_string()]);
        assert_eq!(parsed.on_delete.as_deref(), Some("cascade"));
        assert!(parsed.on_update.is_none());
    }

    #[test]
    fn view_info_with_materialized() {
        let v = ViewInfo {
            name: "v_users".into(),
            kind: ObjectKind::MaterializedView,
            definition_sql: "SELECT * FROM users".into(),
            comment: String::new(),
            extra: Value::Null,
        };
        let j = serde_json::to_string(&v).unwrap();
        assert!(j.contains(r#""kind":"materialized_view""#));
    }

    #[test]
    fn function_info_with_args() {
        let f = FunctionInfo {
            name: "to_lower".into(),
            return_type: Some("text".into()),
            args: vec![FunctionArg {
                name: "input".into(),
                type_str: "text".into(),
                mode: Some("in".into()),
                default: None,
            }],
            language: Some("sql".into()),
            definition: Some("SELECT LOWER($1)".into()),
            comment: String::new(),
            extra: Value::Null,
        };
        let j = serde_json::to_string(&f).unwrap();
        let parsed: FunctionInfo = serde_json::from_str(&j).unwrap();
        assert_eq!(parsed.name, "to_lower");
        assert_eq!(parsed.args.len(), 1);
        assert_eq!(parsed.args[0].mode.as_deref(), Some("in"));
    }

    #[test]
    fn trigger_info_round_trip() {
        let t = TriggerInfo {
            name: "tg_audit".into(),
            table: "users".into(),
            timing: "after".into(),
            event: "update".into(),
            definition: Some("BEGIN INSERT INTO audit VALUES (NEW.*); END".into()),
            comment: String::new(),
            extra: Value::Null,
        };
        let j = serde_json::to_string(&t).unwrap();
        let parsed: TriggerInfo = serde_json::from_str(&j).unwrap();
        assert_eq!(parsed.timing, "after");
        assert_eq!(parsed.event, "update");
    }

    #[test]
    fn sequence_info_round_trip() {
        let s = SequenceInfo {
            name: "seq_id".into(),
            start_value: Some(1),
            increment: Some(1),
            min_value: Some(1),
            max_value: Some(i64::MAX),
            current_value: Some(42),
            cycle: false,
            extra: Value::Null,
        };
        let j = serde_json::to_string(&s).unwrap();
        let parsed: SequenceInfo = serde_json::from_str(&j).unwrap();
        assert_eq!(parsed.current_value, Some(42));
    }

    #[test]
    fn type_info_round_trip() {
        let t = TypeInfo {
            name: "status_enum".into(),
            kind: "enum".into(),
            definition: Some("('pending','active','done')".into()),
            extra: Value::Null,
        };
        let j = serde_json::to_string(&t).unwrap();
        let parsed: TypeInfo = serde_json::from_str(&j).unwrap();
        assert_eq!(parsed.kind, "enum");
    }

    #[test]
    fn view_definition_params_and_result() {
        let p = ViewDefinitionParams {
            conn_id: 1,
            database: None,
            schema: None,
            view: "v1".into(),
        };
        let r = ViewDefinitionResult {
            sql: "CREATE VIEW v1 AS SELECT 1".into(),
            is_materialized: false,
        };
        let jp = serde_json::to_string(&p).unwrap();
        let jr = serde_json::to_string(&r).unwrap();
        assert!(jp.contains(r#""view":"v1""#));
        assert!(jr.contains(r#""is_materialized":false"#));
    }

    #[test]
    fn dump_ddl_params_round_trip() {
        let p = DumpDdlParams {
            conn_id: 17,
            objects: vec![
                ObjectRef {
                    kind: ObjectKind::Table,
                    name: "users".into(),
                    schema: None,
                    database: None,
                },
                ObjectRef {
                    kind: ObjectKind::View,
                    name: "v1".into(),
                    schema: Some("public".into()),
                    database: None,
                },
            ],
            options: DumpDdlOptions {
                if_not_exists: true,
                with_indexes: true,
                with_foreign_keys: true,
                with_triggers: false,
                with_comments: true,
            },
        };
        let j = serde_json::to_string(&p).unwrap();
        let parsed: DumpDdlParams = serde_json::from_str(&j).unwrap();
        assert_eq!(parsed.objects.len(), 2);
        assert!(parsed.options.if_not_exists);
        assert!(!parsed.options.with_triggers);
    }

    #[test]
    fn dump_ddl_result_statements() {
        let r = DumpDdlResult {
            statements: vec![
                "CREATE TABLE users (...)".into(),
                "CREATE INDEX i1 ON users (...)".into(),
            ],
        };
        let j = serde_json::to_string(&r).unwrap();
        assert!(j.contains(r#""statements":["#));
        let parsed: DumpDdlResult = serde_json::from_str(&j).unwrap();
        assert_eq!(parsed.statements.len(), 2);
    }
}
