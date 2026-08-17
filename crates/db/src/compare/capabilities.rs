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
            schema_compare: false,
            data_compare: false,
            metadata_cache: false,
            table_ddl: false,
            indexes: false,
            foreign_keys: false,
            triggers: false,
            checks: false,
            comments: false,
            transactional_ddl: false,
            transactional_dml: false,
            streaming_query: false,
            server_side_checksum: false,
            merge_sql: false,
            upsert_sql: false,
        }
    }
}

impl CompareCapabilities {
    /// PostgreSQL's compare implementation supports the complete currently
    /// implemented table/index/FK path, including transactional DDL and DML.
    pub fn postgresql() -> Self {
        Self {
            schema_compare: true,
            data_compare: true,
            metadata_cache: true,
            table_ddl: true,
            indexes: true,
            foreign_keys: true,
            comments: true,
            transactional_ddl: true,
            transactional_dml: true,
            streaming_query: true,
            ..Default::default()
        }
    }

    /// MySQL supports the compare path, but DDL is not advertised as transactional.
    pub fn mysql() -> Self {
        Self {
            schema_compare: true,
            data_compare: true,
            metadata_cache: true,
            table_ddl: true,
            indexes: true,
            foreign_keys: true,
            comments: true,
            transactional_dml: true,
            streaming_query: true,
            ..Default::default()
        }
    }

    pub fn sqlite() -> Self {
        Self {
            schema_compare: true,
            data_compare: true,
            metadata_cache: true,
            table_ddl: true,
            indexes: true,
            foreign_keys: true,
            comments: true,
            transactional_ddl: true,
            transactional_dml: true,
            streaming_query: true,
            ..Default::default()
        }
    }

    pub fn sqlserver() -> Self {
        Self {
            schema_compare: true,
            data_compare: true,
            metadata_cache: true,
            table_ddl: true,
            indexes: true,
            foreign_keys: true,
            comments: true,
            transactional_ddl: true,
            transactional_dml: true,
            streaming_query: true,
            ..Default::default()
        }
    }

    pub fn clickhouse() -> Self {
        Self {
            schema_compare: true,
            data_compare: true,
            metadata_cache: true,
            table_ddl: true,
            indexes: true,
            comments: true,
            streaming_query: true,
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::CompareCapabilities;

    #[test]
    fn unknown_profile_is_conservative() {
        let capabilities = CompareCapabilities::default();
        assert!(!capabilities.schema_compare);
        assert!(!capabilities.data_compare);
        assert!(!capabilities.indexes);
        assert!(!capabilities.foreign_keys);
        assert!(!capabilities.streaming_query);
    }

    #[test]
    fn constructors_only_advertise_implemented_schema_objects() {
        for capabilities in [
            CompareCapabilities::postgresql(),
            CompareCapabilities::mysql(),
            CompareCapabilities::sqlite(),
            CompareCapabilities::sqlserver(),
            CompareCapabilities::clickhouse(),
        ] {
            assert!(
                !capabilities.triggers,
                "trigger comparison must stay disabled until it has a model, diff, and sync plan"
            );
            assert!(
                !capabilities.checks,
                "check comparison must stay disabled until it has a model, diff, and sync plan"
            );
            assert!(
                !capabilities.merge_sql,
                "MERGE is not emitted by the sync planner"
            );
            assert!(
                !capabilities.upsert_sql,
                "UPSERT is not emitted by the sync planner"
            );
        }
    }
}
