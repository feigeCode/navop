//! 连接种类的展示名。
//!
//! `data_domain` 没有 rust-i18n 依赖，数据库产品名直接写死；Redis / MongoDB 同理。

use gpui::{App, SharedString};
use one_core::storage::DatabaseType;

fn label_mysql(_: &App) -> SharedString {
    "MySQL".into()
}

fn label_postgresql(_: &App) -> SharedString {
    "PostgreSQL".into()
}

fn label_sqlite(_: &App) -> SharedString {
    "SQLite".into()
}

fn label_duckdb(_: &App) -> SharedString {
    "DuckDB".into()
}

fn label_mssql(_: &App) -> SharedString {
    "SQL Server".into()
}

fn label_oracle(_: &App) -> SharedString {
    "Oracle".into()
}

fn label_clickhouse(_: &App) -> SharedString {
    "ClickHouse".into()
}

fn label_tdengine(_: &App) -> SharedString {
    "TDengine".into()
}

fn label_external(_: &App) -> SharedString {
    "外部驱动".into()
}

pub(super) fn label_redis(_: &App) -> SharedString {
    "Redis".into()
}

pub(super) fn label_mongodb(_: &App) -> SharedString {
    "MongoDB".into()
}

/// 数据库类型的展示名。
pub(super) fn db_kind_label(db_type: &DatabaseType) -> fn(&App) -> SharedString {
    match db_type {
        DatabaseType::MySQL => label_mysql,
        DatabaseType::PostgreSQL => label_postgresql,
        DatabaseType::SQLite => label_sqlite,
        DatabaseType::DuckDB => label_duckdb,
        DatabaseType::MSSQL => label_mssql,
        DatabaseType::Oracle => label_oracle,
        DatabaseType::ClickHouse => label_clickhouse,
        DatabaseType::TDengine => label_tdengine,
        DatabaseType::External { .. } => label_external,
    }
}
