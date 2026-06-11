use serde::{Deserialize, Serialize};

/// 比较能力声明
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompareCapabilities {
    /// 支持结构比较
    pub schema_compare: bool,
    /// 支持数据比较
    pub data_compare: bool,
    /// 支持元数据缓存
    pub metadata_cache: bool,
    /// 支持获取表 DDL
    pub table_ddl: bool,
    /// 支持索引
    pub indexes: bool,
    /// 支持外键
    pub foreign_keys: bool,
    /// 支持触发器
    pub triggers: bool,
    /// 支持检查约束
    pub checks: bool,
    /// 支持注释
    pub comments: bool,
    /// DDL 支持事务
    pub transactional_ddl: bool,
    /// DML 支持事务
    pub transactional_dml: bool,
    /// 支持流式查询
    pub streaming_query: bool,
    /// 支持服务器端 checksum
    pub server_side_checksum: bool,
    /// 支持 MERGE 语句
    pub merge_sql: bool,
    /// 支持 UPSERT 语句
    pub upsert_sql: bool,
}

impl Default for CompareCapabilities {
    fn default() -> Self {
        Self {
            schema_compare: true,
            data_compare: true,
            metadata_cache: true,
            table_ddl: false,
            indexes: true,
            foreign_keys: true,
            triggers: false,
            checks: false,
            comments: true,
            transactional_ddl: false,
            transactional_dml: true,
            streaming_query: true,
            server_side_checksum: false,
            merge_sql: false,
            upsert_sql: false,
        }
    }
}

impl CompareCapabilities {
    /// PostgreSQL 的比较能力
    pub fn postgresql() -> Self {
        Self {
            table_ddl: true,
            triggers: true,
            checks: true,
            transactional_ddl: true,
            upsert_sql: true,
            ..Default::default()
        }
    }

    /// MySQL 的比较能力
    pub fn mysql() -> Self {
        Self {
            table_ddl: true,
            triggers: true,
            checks: true,
            transactional_ddl: false,
            ..Default::default()
        }
    }

    /// SQLite 的比较能力
    pub fn sqlite() -> Self {
        Self {
            table_ddl: true,
            foreign_keys: true,
            triggers: true,
            checks: true,
            transactional_ddl: true,
            upsert_sql: true,
            ..Default::default()
        }
    }

    /// SQL Server 的比较能力
    pub fn sqlserver() -> Self {
        Self {
            table_ddl: true,
            triggers: true,
            checks: true,
            transactional_ddl: true,
            merge_sql: true,
            ..Default::default()
        }
    }

    /// ClickHouse 的比较能力
    pub fn clickhouse() -> Self {
        Self {
            table_ddl: true,
            foreign_keys: false,
            triggers: false,
            checks: false,
            transactional_ddl: false,
            transactional_dml: false,
            ..Default::default()
        }
    }
}
