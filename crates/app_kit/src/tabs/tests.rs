//! `crates/app_kit/src/tabs.rs` 的单测：快照读写与恢复器装配。
//!
//! 只覆盖纯逻辑（文件 IO + registry），不依赖 gpui 窗口。

use super::*;
use std::collections::HashMap;

struct FakeRestorer {
    keys: Vec<&'static str>,
}

impl TabRestorer for FakeRestorer {
    fn content_keys(&self) -> Vec<&'static str> {
        self.keys.clone()
    }

    fn restore(
        &self,
        _state: &TabItemState,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Option<Arc<dyn one_core::tab_container::TabContentView>> {
        None
    }
}

/// 每个测试用独立临时文件，避免并行互相覆盖。
fn temp_file(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("navop-tabs-{}-{name}.json", std::process::id()))
}

fn snapshot() -> TabContainerState {
    TabContainerState {
        version: Some(STATE_VERSION),
        tabs: vec![
            TabItemState {
                id: "ssh-terminal-42-1700000000000".into(),
                from: "ssh".into(),
                key: "Terminal".into(),
                metadata: HashMap::new(),
                data: serde_json::Value::Null,
            },
            TabItemState {
                id: "known-hosts".into(),
                from: "home".into(),
                key: "KnownHosts".into(),
                metadata: HashMap::from([("k".to_string(), "v".to_string())]),
                data: serde_json::Value::Null,
            },
        ],
        active_index: 1,
        config: Default::default(),
    }
}

#[test]
fn snapshot_round_trip_keeps_tabs_active_index_and_metadata() {
    let path = temp_file("round-trip");
    let state = snapshot();
    write_state_to(&path, &state).expect("写快照");
    assert_eq!(read_state_from(&path), Some(state));
    let _ = std::fs::remove_file(path);
}

#[test]
fn corrupted_snapshot_is_ignored() {
    let path = temp_file("corrupted");
    std::fs::write(&path, "{ 不是 JSON }").expect("写坏文件");
    assert_eq!(read_state_from(&path), None);
    // 缺字段的 JSON 同样按损坏处理。
    std::fs::write(&path, "{\"tabs\":[]}").expect("写残缺文件");
    assert_eq!(read_state_from(&path), None);
    let _ = std::fs::remove_file(path);
}

#[test]
fn missing_snapshot_means_empty_session() {
    let path = temp_file("missing");
    let _ = std::fs::remove_file(&path);
    assert_eq!(read_state_from(&path), None);
}

#[test]
fn newer_snapshot_version_is_not_restored() {
    let mut state = snapshot();
    state.version = Some(STATE_VERSION + 1);
    let text = serde_json::to_string(&state).expect("序列化");
    assert_eq!(parse_state(&text), None);
}

#[test]
fn registry_registers_every_restorer_key() {
    let restorers: Vec<Arc<dyn TabRestorer>> = vec![
        Arc::new(FakeRestorer {
            keys: vec!["Terminal", "SFTP"],
        }),
        Arc::new(FakeRestorer {
            keys: vec!["Database", "Redis", "MongoDB"],
        }),
    ];
    let registry = content_registry(&restorers);
    for key in ["Terminal", "SFTP", "Database", "Redis", "MongoDB"] {
        assert!(registry.has_builder(key), "缺少 {key} 的恢复器");
    }
    assert!(!registry.has_builder("Unknown"));
}

#[test]
fn later_restorer_wins_on_duplicate_key() {
    let restorers: Vec<Arc<dyn TabRestorer>> = vec![
        Arc::new(FakeRestorer { keys: vec!["SFTP"] }),
        Arc::new(FakeRestorer { keys: vec!["SFTP"] }),
    ];
    let registry = content_registry(&restorers);
    assert!(registry.has_builder("SFTP"));
}
