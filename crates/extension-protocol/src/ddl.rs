//! DDL 构造 (`ddl/*`)。
//!
//! 不直接发到数据库,而是把声明式的 `Spec` 转成驱动方言的 SQL 字符串,
//! 由 host 自行决定何时执行(可能给用户预览、加事务、批量打包等)。
//!
//! 详见 [`docs/design/extensions/api-database.md`] §12。

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::conn::ConnId;
use crate::schema::ObjectKind;

// ============================================================================
// 通用 Spec
// ============================================================================

/// 列声明,用于 create_table / alter_table。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ColumnSpec {
    pub name: String,
    /// 完整类型字符串,例如 `VARCHAR(255)` / `numeric(10,2)`。
    #[serde(rename = "type")]
    pub type_str: String,
    #[serde(default = "default_nullable_true")]
    pub nullable: bool,
    /// 默认值表达式(SQL 文本),`NULL` 用 None 表示不写默认。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    #[serde(default)]
    pub is_primary: bool,
    #[serde(default)]
    pub is_unique: bool,
    #[serde(default)]
    pub auto_increment: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub comment: String,
    /// 字符集 / collation / 列附加约束。
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub extra: Value,
}

fn default_nullable_true() -> bool {
    true
}

/// 索引声明(组合索引列序敏感)。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IndexSpec {
    pub name: String,
    pub columns: Vec<String>,
    /// `btree` / `hash` / `gin` / `fulltext` / ...
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default)]
    pub is_unique: bool,
    /// 部分索引 WHERE 子句。
    #[serde(rename = "where", default, skip_serializing_if = "Option::is_none")]
    pub where_clause: Option<String>,
}

/// 外键声明。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ForeignKeySpec {
    pub name: String,
    pub from_columns: Vec<String>,
    pub to_table: String,
    pub to_columns: Vec<String>,
    /// `cascade` / `restrict` / `set_null` / `set_default` / `no_action`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_delete: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_update: Option<String>,
}

/// 表声明(CREATE / ALTER 共用)。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TableSpec {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database: Option<String>,
    pub columns: Vec<ColumnSpec>,
    /// 主键(如果不在某一列上声明 `is_primary` 而是组合主键)。
    #[serde(default)]
    pub primary_key: Vec<String>,
    #[serde(default)]
    pub indexes: Vec<IndexSpec>,
    #[serde(default)]
    pub foreign_keys: Vec<ForeignKeySpec>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub comment: String,
    /// 表选项(engine / charset / partition by / replication 等),驱动专属。
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub options: Value,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ColumnRenameSpec {
    pub old_name: String,
    pub new_name: String,
}

// ============================================================================
// ddl/build
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DdlBuildOp {
    CreateDatabase,
    ModifyDatabase,
    DropDatabase,
    CreateSchema,
    DropSchema,
    CommentSchema,
    RenameTable,
    TruncateTable,
    CreateTable,
    AlterTable,
    DropTable,
    DropView,
    ColumnDefinition,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildDdlParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conn_id: Option<ConnId>,
    pub op: DdlBuildOp,
    #[serde(default)]
    pub payload: Value,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BuildDdlResult {
    #[serde(default)]
    pub statements: Vec<String>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

// ============================================================================
// ddl/build_create_table
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildCreateTableParams {
    /// 仅在需要方言 / 服务端 schema 时填(本地无 conn 也可以 build)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conn_id: Option<ConnId>,
    pub spec: TableSpec,
    #[serde(default)]
    pub options: CreateTableOptions,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateTableOptions {
    #[serde(default)]
    pub if_not_exists: bool,
    /// 是否生成 indexes / foreign_keys 子句(默认 true)。
    #[serde(default = "default_true_field")]
    pub with_indexes: bool,
    #[serde(default = "default_true_field")]
    pub with_foreign_keys: bool,
    #[serde(default = "default_true_field")]
    pub with_comments: bool,
    /// `CREATE TEMP TABLE`(若驱动支持)。
    #[serde(default)]
    pub temporary: bool,
}

fn default_true_field() -> bool {
    true
}

impl Default for CreateTableOptions {
    fn default() -> Self {
        Self {
            if_not_exists: false,
            with_indexes: true,
            with_foreign_keys: true,
            with_comments: true,
            temporary: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildCreateTableResult {
    #[serde(default)]
    pub sql: String,
    /// 拆分的多条 statement(某些方言索引 / FK 必须分开)。
    #[serde(default)]
    pub statements: Vec<String>,
}

// ============================================================================
// ddl/build_alter_table
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildAlterTableParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conn_id: Option<ConnId>,
    pub from_spec: TableSpec,
    pub to_spec: TableSpec,
    #[serde(default)]
    pub column_renames: Vec<ColumnRenameSpec>,
    #[serde(default)]
    pub options: AlterTableOptions,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AlterTableOptions {
    /// 是否允许 destructive 改动(rename / drop column / change type)。
    #[serde(default)]
    pub allow_destructive: bool,
    /// 是否生成回滚脚本。
    #[serde(default)]
    pub with_rollback: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BuildAlterTableResult {
    pub statements: Vec<String>,
    /// 对应的回滚语句(顺序倒序,与 `statements` 一一对应)。
    #[serde(default)]
    pub rollback_statements: Vec<String>,
    /// 警告(可能的数据丢失 / 长事务等)。
    #[serde(default)]
    pub warnings: Vec<String>,
}

// ============================================================================
// ddl/build_drop
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildDropParams {
    pub kind: ObjectKind,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database: Option<String>,
    #[serde(default)]
    pub if_exists: bool,
    /// CASCADE / RESTRICT。
    #[serde(default)]
    pub cascade: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildDropResult {
    pub sql: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn column_spec_default_nullable_true() {
        let c: ColumnSpec = serde_json::from_str(r#"{"name":"x","type":"int"}"#).unwrap();
        assert_eq!(c.name, "x");
        assert_eq!(c.type_str, "int");
        assert!(c.nullable);
        assert!(!c.is_primary);
        assert!(c.default.is_none());
    }

    #[test]
    fn column_spec_full_round_trip() {
        let c = ColumnSpec {
            name: "id".into(),
            type_str: "BIGINT UNSIGNED".into(),
            nullable: false,
            default: None,
            is_primary: true,
            is_unique: true,
            auto_increment: true,
            comment: "primary key".into(),
            extra: serde_json::json!({"charset": "utf8mb4"}),
        };
        let j = serde_json::to_string(&c).unwrap();
        let parsed: ColumnSpec = serde_json::from_str(&j).unwrap();
        assert_eq!(parsed.name, "id");
        assert!(!parsed.nullable);
        assert!(parsed.is_primary);
        assert!(parsed.auto_increment);
        assert_eq!(parsed.comment, "primary key");
    }

    #[test]
    fn index_spec_with_where_clause() {
        let i = IndexSpec {
            name: "idx_active".into(),
            columns: vec!["created_at".into()],
            kind: Some("btree".into()),
            is_unique: false,
            where_clause: Some("deleted_at IS NULL".into()),
        };
        let j = serde_json::to_string(&i).unwrap();
        assert!(j.contains(r#""where":"deleted_at IS NULL""#));
        let parsed: IndexSpec = serde_json::from_str(&j).unwrap();
        assert_eq!(parsed.where_clause.as_deref(), Some("deleted_at IS NULL"));
    }

    #[test]
    fn ddl_build_op_serializes_as_snake_case() {
        let params = BuildDdlParams {
            conn_id: None,
            op: DdlBuildOp::ModifyDatabase,
            payload: serde_json::json!({"database_name": "analytics"}),
        };

        let json = serde_json::to_value(params).unwrap();

        assert_eq!(json["op"], "modify_database");
        assert_eq!(json["payload"]["database_name"], "analytics");
    }

    #[test]
    fn foreign_key_spec_round_trip() {
        let f = ForeignKeySpec {
            name: "fk_user".into(),
            from_columns: vec!["user_id".into()],
            to_table: "users".into(),
            to_columns: vec!["id".into()],
            on_delete: Some("cascade".into()),
            on_update: Some("restrict".into()),
        };
        let j = serde_json::to_string(&f).unwrap();
        let parsed: ForeignKeySpec = serde_json::from_str(&j).unwrap();
        assert_eq!(parsed.from_columns, vec!["user_id".to_string()]);
        assert_eq!(parsed.on_delete.as_deref(), Some("cascade"));
        assert_eq!(parsed.on_update.as_deref(), Some("restrict"));
    }

    #[test]
    fn table_spec_with_composite_primary_key() {
        let t = TableSpec {
            name: "users".into(),
            schema: Some("public".into()),
            database: None,
            columns: vec![
                ColumnSpec {
                    name: "id".into(),
                    type_str: "int".into(),
                    nullable: false,
                    ..Default::default()
                },
                ColumnSpec {
                    name: "tenant".into(),
                    type_str: "varchar(64)".into(),
                    nullable: false,
                    ..Default::default()
                },
            ],
            primary_key: vec!["id".into(), "tenant".into()],
            indexes: vec![],
            foreign_keys: vec![],
            comment: String::new(),
            options: serde_json::json!({"engine": "InnoDB"}),
        };
        let j = serde_json::to_string(&t).unwrap();
        let parsed: TableSpec = serde_json::from_str(&j).unwrap();
        assert_eq!(parsed.primary_key.len(), 2);
        assert_eq!(parsed.columns.len(), 2);
        assert_eq!(parsed.options, serde_json::json!({"engine": "InnoDB"}));
    }

    #[test]
    fn build_create_table_params_with_options() {
        let p = BuildCreateTableParams {
            conn_id: Some(17),
            spec: TableSpec {
                name: "users".into(),
                ..Default::default()
            },
            options: CreateTableOptions {
                if_not_exists: true,
                with_indexes: true,
                with_foreign_keys: true,
                with_comments: false,
                temporary: false,
            },
        };
        let j = serde_json::to_string(&p).unwrap();
        let parsed: BuildCreateTableParams = serde_json::from_str(&j).unwrap();
        assert_eq!(parsed.conn_id, Some(17));
        assert!(parsed.options.if_not_exists);
        assert!(parsed.options.with_indexes);
        assert!(!parsed.options.with_comments);
    }

    #[test]
    fn create_table_options_defaults_true_for_with_indexes() {
        let o: CreateTableOptions = serde_json::from_str("{}").unwrap();
        assert!(o.with_indexes);
        assert!(o.with_foreign_keys);
        assert!(o.with_comments);
        assert!(!o.if_not_exists);
    }

    #[test]
    fn build_create_table_result_round_trip() {
        let r = BuildCreateTableResult {
            sql: "CREATE TABLE users (id INT)".into(),
            statements: vec![
                "CREATE TABLE users (id INT)".into(),
                "CREATE INDEX i1 ON users (id)".into(),
            ],
        };
        let j = serde_json::to_string(&r).unwrap();
        let parsed: BuildCreateTableResult = serde_json::from_str(&j).unwrap();
        assert_eq!(parsed.statements.len(), 2);
    }

    #[test]
    fn build_create_table_result_missing_sql_defaults_for_statement_only_drivers() {
        let parsed: BuildCreateTableResult =
            serde_json::from_str(r#"{"statements":["CREATE TABLE users (id INT)"]}"#).unwrap();

        assert_eq!(parsed.sql, "");
        assert_eq!(parsed.statements, vec!["CREATE TABLE users (id INT)"]);
    }

    #[test]
    fn build_alter_table_with_rollback() {
        let p = BuildAlterTableParams {
            conn_id: None,
            from_spec: TableSpec {
                name: "users".into(),
                ..Default::default()
            },
            to_spec: TableSpec {
                name: "users".into(),
                columns: vec![ColumnSpec {
                    name: "age".into(),
                    type_str: "int".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            column_renames: vec![ColumnRenameSpec {
                old_name: "name".into(),
                new_name: "full_name".into(),
            }],
            options: AlterTableOptions {
                allow_destructive: false,
                with_rollback: true,
            },
        };
        let j = serde_json::to_string(&p).unwrap();
        let parsed: BuildAlterTableParams = serde_json::from_str(&j).unwrap();
        assert!(parsed.options.with_rollback);
        assert!(!parsed.options.allow_destructive);
        assert_eq!(parsed.column_renames[0].old_name, "name");
        assert_eq!(parsed.column_renames[0].new_name, "full_name");
    }

    #[test]
    fn build_alter_table_result_with_warnings() {
        let r = BuildAlterTableResult {
            statements: vec!["ALTER TABLE users ADD COLUMN age int".into()],
            rollback_statements: vec!["ALTER TABLE users DROP COLUMN age".into()],
            warnings: vec!["may take a long time on large table".into()],
        };
        let j = serde_json::to_string(&r).unwrap();
        let parsed: BuildAlterTableResult = serde_json::from_str(&j).unwrap();
        assert_eq!(parsed.statements.len(), 1);
        assert_eq!(parsed.rollback_statements.len(), 1);
        assert_eq!(parsed.warnings.len(), 1);
    }

    #[test]
    fn build_drop_params_for_table() {
        let p = BuildDropParams {
            kind: ObjectKind::Table,
            name: "users".into(),
            schema: None,
            database: None,
            if_exists: true,
            cascade: false,
        };
        let j = serde_json::to_string(&p).unwrap();
        let parsed: BuildDropParams = serde_json::from_str(&j).unwrap();
        assert_eq!(parsed.kind, ObjectKind::Table);
        assert!(parsed.if_exists);
        assert!(!parsed.cascade);
    }

    #[test]
    fn build_drop_result_round_trip() {
        let r = BuildDropResult {
            sql: "DROP TABLE IF EXISTS users".into(),
        };
        let j = serde_json::to_string(&r).unwrap();
        assert_eq!(j, r#"{"sql":"DROP TABLE IF EXISTS users"}"#);
    }
}
