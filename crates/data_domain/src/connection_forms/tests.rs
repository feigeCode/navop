//! 连接种类映射的纯函数单测（不依赖 GPUI 全局状态）。

use super::*;
use one_core::storage::{DbConnectionConfig, MongoDBParams, RedisParams};

fn database_connection(db_type: DatabaseType) -> StoredConnection {
    let config = DbConnectionConfig {
        id: String::new(),
        database_type: db_type,
        name: "测试库".to_string(),
        host: "127.0.0.1".to_string(),
        port: 3306,
        username: "root".to_string(),
        password: String::new(),
        credential_reference: None,
        database: None,
        service_name: None,
        sid: None,
        workspace_id: None,
        proxy: None,
        extra_params: Default::default(),
    };
    StoredConnection::new_database("测试库".to_string(), config, None)
}

fn redis_connection() -> StoredConnection {
    let params: RedisParams = serde_json::from_value(serde_json::json!({
        "host": "127.0.0.1",
        "port": 6379,
        "password": null,
        "username": null,
        "db_index": 0,
    }))
    .expect("RedisParams 应能反序列化");
    StoredConnection::new_redis("缓存".to_string(), params, None)
}

fn mongodb_connection() -> StoredConnection {
    let params: MongoDBParams =
        serde_json::from_value(serde_json::json!({})).expect("MongoDBParams 应能反序列化");
    StoredConnection::new_mongodb("文档库".to_string(), params, None)
}

/// 种类 ID 集合 = 内置数据库各一个 + redis + mongodb。
///
/// 已安装的外部驱动会追加种类，这里只断言内置部分。
#[test]
fn kind_ids_cover_builtin_databases_redis_and_mongodb() {
    let kinds: Vec<ConnectionKindDescriptor> = kind_descriptors()
        .into_iter()
        .filter(|kind| !kind.id.starts_with(DB_EXTERNAL_KIND_PREFIX))
        .collect();
    assert_eq!(
        DatabaseType::builtin_all().len() + 2,
        kinds.len(),
        "每种内置数据库各一个种类，另加 Redis / MongoDB"
    );

    let ids: Vec<&str> = kinds.iter().map(|kind| kind.id.as_ref()).collect();
    for db_type in DatabaseType::builtin_all() {
        assert!(
            ids.contains(&format!("db:{}", db_type.as_str()).as_str()),
            "缺少数据库种类 db:{}",
            db_type.as_str()
        );
    }
    assert!(ids.contains(&REDIS_KIND_ID));
    assert!(ids.contains(&MONGO_KIND_ID));
    assert!(
        kinds
            .iter()
            .all(|kind| kind.group == ConnectionKindGroup::Database)
    );
}

/// Redis / MongoDB 的 order 必须排在内置数据库之后。
#[test]
fn redis_and_mongodb_kinds_are_ordered_after_databases() {
    let kinds = kind_descriptors();
    let max_db_order = kinds
        .iter()
        .filter(|kind| kind.id.starts_with(DB_KIND_PREFIX))
        .filter(|kind| !kind.id.starts_with(DB_EXTERNAL_KIND_PREFIX))
        .map(|kind| kind.order)
        .max()
        .expect("至少有一种内置数据库");
    let redis = kinds.iter().find(|kind| kind.id == REDIS_KIND_ID);
    let mongodb = kinds.iter().find(|kind| kind.id == MONGO_KIND_ID);
    assert!(redis.is_some_and(|kind| kind.order > max_db_order));
    assert!(mongodb.is_some_and(|kind| kind.order > max_db_order));
}

/// `kind_for`：每种内置数据库都映射到 `db:{database_type}`。
#[test]
fn kind_id_maps_each_builtin_database_type() {
    for db_type in DatabaseType::builtin_all() {
        let connection = database_connection(db_type.clone());
        assert_eq!(
            Some(SharedString::from(format!("db:{}", db_type.as_str()))),
            kind_id_for_connection(&connection),
            "数据库类型 {} 映射错误",
            db_type.as_str()
        );
    }
}

/// `kind_for`：Redis / MongoDB 映射到固定 ID。
#[test]
fn kind_id_maps_redis_and_mongodb() {
    assert_eq!(
        Some(SharedString::from(REDIS_KIND_ID)),
        kind_id_for_connection(&redis_connection())
    );
    assert_eq!(
        Some(SharedString::from(MONGO_KIND_ID)),
        kind_id_for_connection(&mongodb_connection())
    );
}

/// `kind_for`：其它连接类型返回 `None`。
#[test]
fn kind_id_returns_none_for_unsupported_connection_type() {
    let mut connection = database_connection(DatabaseType::MySQL);
    connection.connection_type = ConnectionType::SshSftp;
    assert_eq!(None, kind_id_for_connection(&connection));

    connection.connection_type = ConnectionType::Ftp;
    assert_eq!(None, kind_id_for_connection(&connection));
}

/// `resolve_kind`：只认本领域 ID，未知 / 外部驱动一律拒绝。
#[test]
fn resolve_kind_rejects_unknown_ids() {
    assert!(resolve_kind("ssh").is_none());
    assert!(resolve_kind("redis2").is_none());
    assert!(resolve_kind("db:Unknown").is_none());
    assert!(resolve_kind("db:External").is_none());
    assert!(matches!(
        resolve_kind("db:MySQL"),
        Some(KindTarget::Database(DatabaseType::MySQL))
    ));
    assert!(matches!(
        resolve_kind(REDIS_KIND_ID),
        Some(KindTarget::Redis)
    ));
    assert!(matches!(
        resolve_kind(MONGO_KIND_ID),
        Some(KindTarget::Mongo)
    ));
}

/// 驱动清单：只填反序列化必需的字段。
fn driver_manifest(value: serde_json::Value) -> IpcDriverManifest {
    serde_json::from_value(value).expect("驱动清单应能反序列化")
}

fn external_driver(id: &str, name: &str) -> IpcDriverManifest {
    driver_manifest(serde_json::json!({
        "id": id,
        "name": name,
        "description": format!("{name} 数据库连接"),
        "entry": { "command": "driver" },
        "transport": { "name": "stdio" },
    }))
}

/// 外部驱动种类：跳过内置类型代表与声明不展示的驱动。
#[test]
fn external_kind_descriptors_skip_builtin_and_hidden_drivers() {
    let mut hidden = external_driver("hidden", "Hidden");
    hidden.ui.show_in_new_connection = false;
    let registry = IpcDriverRegistry::from_drivers(vec![
        external_driver("kingbase", "KingbaseES"),
        external_driver("duckdb", "DuckDB"),
        external_driver("oracle-go", "Oracle Go"),
        external_driver("tdengine", "TDengine"),
        external_driver("clickhouse", "ClickHouse"),
        hidden,
    ]);

    let kinds = external_kind_descriptors(&registry);
    let ids: Vec<&str> = kinds.iter().map(|kind| kind.id.as_ref()).collect();
    assert_eq!(vec!["db:external:kingbase"], ids);
}

/// 外部驱动种类带上清单里的名字、描述与类目。
#[test]
fn external_kind_descriptor_carries_manifest_metadata() {
    let mut driver = external_driver("kingbase", "KingbaseES");
    driver.category = Some("domestic_database".to_string());
    let registry = IpcDriverRegistry::from_drivers(vec![driver]);

    let kind = &external_kind_descriptors(&registry)[0];
    assert_eq!("db:external:kingbase", kind.id.as_ref());
    assert_eq!(ConnectionKindGroup::Database, kind.group);
    assert!(kind.order >= EXTERNAL_KIND_ORDER_BASE);
    assert_eq!(
        Some("KingbaseES"),
        kind.label_override().map(|v| v.as_ref())
    );
    assert_eq!(
        Some("KingbaseES 数据库连接"),
        kind.description().map(|v| v.as_ref())
    );
    assert_eq!(
        Some("domestic_database"),
        kind.category().map(|v| v.as_ref())
    );
}

/// 外部驱动的连接映射回 `db:external:{driver_id}`，并能还原为表单种类。
#[test]
fn external_database_type_round_trips_through_kind_id() {
    let connection = database_connection(DatabaseType::external("kingbase"));
    assert_eq!(
        Some(SharedString::from("db:external:kingbase")),
        kind_id_for_connection(&connection)
    );
    assert!(matches!(
        resolve_kind("db:external:kingbase"),
        Some(KindTarget::Database(DatabaseType::External { .. }))
    ));
    assert!(resolve_kind("db:external:").is_none());
}
