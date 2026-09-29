use std::fs;

use serde_json::json;

use crate::provider_storage::{
    MAX_STORAGE_KEY_BYTES, ProviderStore, ProviderStoreError, ProviderStoreLimits,
    extension_storage_dir,
};

fn store() -> (tempfile::TempDir, ProviderStore) {
    // 目录测试都走 `<root>/<extension_id>/`,和生产一致。
    let dir = tempfile::tempdir().expect("tempdir");
    let extension_dir =
        extension_storage_dir(dir.path(), "com.navop.test").expect("extension storage dir");
    (dir, ProviderStore::new(extension_dir))
}

#[test]
fn set_then_get_round_trips_across_instances() {
    let (dir, store) = store();
    store
        .set("mqtt", "profile", json!({"host": "broker.local"}), None)
        .expect("set");
    assert_eq!(
        store.get("mqtt", "profile").expect("get"),
        Some(json!({"host": "broker.local"}))
    );

    // 新实例 = 新进程视角:数据必须来自磁盘而不是内存。
    let reopened = ProviderStore::new(extension_storage_dir(dir.path(), "com.navop.test").unwrap());
    assert_eq!(
        reopened.get("mqtt", "profile").expect("get"),
        Some(json!({"host": "broker.local"}))
    );
    // 命名空间彼此隔离。
    assert_eq!(reopened.get("other", "profile").expect("get"), None);
}

#[test]
fn missing_key_is_none_not_an_error() {
    let (_dir, store) = store();
    assert_eq!(store.get("mqtt", "absent").expect("get"), None);
}

#[test]
fn expired_entries_disappear() {
    let (_dir, store) = store();
    store.set("mqtt", "short", json!(1), Some(0)).expect("set");
    assert_eq!(store.get("mqtt", "short").expect("get"), None);
    store.set("mqtt", "forever", json!(1), None).expect("set");
    assert_eq!(store.get("mqtt", "forever").expect("get"), Some(json!(1)));
}

#[test]
fn namespaces_cannot_escape_the_extension_directory() {
    let (dir, store) = store();
    store.set("mqtt", "k", json!(1), None).expect("legit write");
    for hostile in ["../escape", "..", "a/b", ".hidden", "ünïcode", ""] {
        assert!(
            matches!(
                store.set(hostile, "k", json!(1), None),
                Err(ProviderStoreError::InvalidNamespace(_))
            ),
            "namespace {hostile:?} must be rejected"
        );
    }
    assert!(matches!(
        store.get("../escape", "k"),
        Err(ProviderStoreError::InvalidNamespace(_))
    ));
    // 扩展目录之外没有被创建任何东西。
    let created: Vec<_> = fs::read_dir(dir.path())
        .expect("read root")
        .map(|entry| entry.expect("entry").file_name())
        .collect();
    assert_eq!(created.len(), 1, "unexpected entries: {created:?}");
    assert_eq!(created[0], "com.navop.test");
    let inside: Vec<_> = fs::read_dir(dir.path().join("com.navop.test"))
        .expect("read extension dir")
        .map(|entry| entry.expect("entry").file_name())
        .collect();
    assert_eq!(inside.len(), 1, "unexpected entries: {inside:?}");
    assert_eq!(inside[0], "mqtt.json");
}

#[test]
fn invalid_and_oversized_keys_are_rejected() {
    let (_dir, store) = store();
    assert!(matches!(
        store.set("mqtt", "", json!(1), None),
        Err(ProviderStoreError::InvalidKey)
    ));
    let long = "k".repeat(MAX_STORAGE_KEY_BYTES + 1);
    assert!(matches!(
        store.get("mqtt", &long),
        Err(ProviderStoreError::InvalidKey)
    ));
}

#[test]
fn value_and_namespace_budgets_are_enforced() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = ProviderStore::with_limits(
        dir.path(),
        ProviderStoreLimits {
            max_value_bytes: 24,
            max_namespace_bytes: 20,
        },
    );

    // value 本身超限(`"x" * 32` 加上引号 = 34 字节 > 24)。
    let oversized = json!("x".repeat(32));
    assert!(matches!(
        store.set("mqtt", "big", oversized, None),
        Err(ProviderStoreError::ValueTooLarge { .. })
    ));

    // 单个 entry 放得下:key 1 字节 + value 12 字节 = 13。
    store
        .set("mqtt", "a", json!("1".repeat(10)), None)
        .expect("first entry fits");
    assert!(matches!(
        store.set("mqtt", "b", json!("2".repeat(10)), None),
        Err(ProviderStoreError::NamespaceTooLarge { .. })
    ));
    // 超限被拒后,先前写入的内容不能受损(不能出现"部分写入")。
    assert_eq!(
        store.get("mqtt", "a").expect("get"),
        Some(json!("1".repeat(10)))
    );
    assert_eq!(store.get("mqtt", "b").expect("get"), None);
}

#[test]
fn corrupt_files_are_quarantined_rather_than_fatal() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = ProviderStore::new(dir.path().join("ext"));
    store
        .set("mqtt", "profile", json!({"host": "a"}), None)
        .expect("set");

    let path = dir.path().join("ext/mqtt.json");
    fs::write(&path, b"{ this is not json").expect("corrupt");

    // 读:坏文件挪走,返回"没有",而不是永久报错。
    assert_eq!(store.get("mqtt", "profile").expect("get"), None);
    // 写:之后照常工作。
    store
        .set("mqtt", "profile", json!({"host": "b"}), None)
        .expect("set");
    assert_eq!(
        store.get("mqtt", "profile").expect("get"),
        Some(json!({"host": "b"}))
    );

    let quarantined = fs::read_dir(dir.path().join("ext"))
        .expect("read ext")
        .filter_map(|entry| entry.ok())
        .any(|entry| entry.file_name().to_string_lossy().contains("corrupt"));
    assert!(quarantined, "corrupt file should be kept aside");
}

#[test]
fn unknown_store_version_is_treated_as_corrupt() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = ProviderStore::new(dir.path().join("ext"));
    fs::create_dir_all(dir.path().join("ext")).expect("mkdir");
    fs::write(
        dir.path().join("ext/mqtt.json"),
        br#"{"version": 99, "entries": {"k": {"value": 1}}}"#,
    )
    .expect("write");
    // 语义不明的数据宁可重建,也不按猜测的格式读出来。
    assert_eq!(store.get("mqtt", "k").expect("get"), None);
}

#[test]
fn writes_are_atomic_and_leave_no_temp_files() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = ProviderStore::new(dir.path().join("ext"));
    for index in 0..5 {
        store
            .set("mqtt", &format!("key-{index}"), json!(index), None)
            .expect("set");
    }
    let leftovers: Vec<_> = fs::read_dir(dir.path().join("ext"))
        .expect("read ext")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains("tmp"))
        .collect();
    assert!(
        leftovers.is_empty(),
        "temp files left behind: {leftovers:?}"
    );
}

#[test]
fn extension_directory_rejects_traversal() {
    let dir = tempfile::tempdir().expect("tempdir");
    assert!(extension_storage_dir(dir.path(), "../evil").is_err());
    assert!(extension_storage_dir(dir.path(), "com.navop.mqtt").is_ok());
}
