//! 扩展连接的图标解析:列表、卡片、标签页共用同一套规则,避免各处各写一份后细节对不上。
//!
//! 优先级:
//! 1. 连接表单 `Icon` 字段里用户的选择(本地图片 → 内置图标);
//! 2. 扩展 manifest `contributes.connections[].icon` 声明的图标文件;
//! 3. 都没有时返回 `None`,由调用方回退到通用扩展图标
//!    (`IconName::ExtensionsColor`,与「新建连接」列表一致)。

use gpui_component::Icon;
use one_core::storage::{StoredConnection, local_icon_asset_path, stored_connection_icon};

/// 解析扩展连接要显示的图标。`contribution` 为空(目录不可用)时只看用户选择。
pub fn resolve_extension_connection_icon(
    connection: &StoredConnection,
    contribution: Option<&extension_runtime::RegisteredResourceConnectionContribution>,
) -> Option<Icon> {
    stored_connection_icon(connection).or_else(|| manifest_connection_icon(contribution?))
}

/// 与 [`resolve_extension_connection_icon`] 相同,但未命中时回退到通用扩展图标。
/// 标签页没有「由调用方回退」的位置,直接用它。
pub fn extension_connection_icon_or_default(
    connection: &StoredConnection,
    contribution: Option<&extension_runtime::RegisteredResourceConnectionContribution>,
) -> Icon {
    resolve_extension_connection_icon(connection, contribution)
        .unwrap_or_else(|| one_assets::IconName::ExtensionsColor.color())
}

/// 扩展 manifest 为连接声明的图标文件;文件不存在时视为未声明。
fn manifest_connection_icon(
    contribution: &extension_runtime::RegisteredResourceConnectionContribution,
) -> Option<Icon> {
    let path = contribution.icon_path.as_ref()?;
    path.is_file()
        .then(|| Icon::default().path(local_icon_asset_path(path)).color())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    fn connection(config: serde_json::Value) -> StoredConnection {
        let params = one_core::storage::ExtensionConnectionParams::new(
            "com.example.demo",
            "demo",
            config.as_object().cloned().expect("config object"),
            BTreeMap::new(),
        )
        .expect("params should validate");
        StoredConnection::new_extension("Demo".to_string(), params, None)
    }

    fn contribution(
        icon_path: Option<std::path::PathBuf>,
    ) -> extension_runtime::RegisteredResourceConnectionContribution {
        extension_runtime::RegisteredResourceConnectionContribution {
            extension_id: "com.example.demo".into(),
            extension_root: std::path::PathBuf::from("/tmp/com.example.demo"),
            id: "demo".into(),
            label: "Demo".into(),
            description: None,
            icon_path,
            runtime_id: String::new(),
            resource_type: String::new(),
            route: None,
            terminal: None,
            form: Default::default(),
        }
    }

    #[test]
    fn auto_selection_falls_back_to_the_manifest_icon() {
        let directory = tempfile::tempdir().expect("temp dir");
        let svg = directory.path().join("demo.svg");
        std::fs::write(&svg, "<svg xmlns=\"http://www.w3.org/2000/svg\"/>").expect("icon");
        let contribution = contribution(Some(svg));

        // 「自动」即配置里没有选择:必须落到 manifest 图标,而不是 None。
        let auto = connection(serde_json::json!({ "target": "root@host" }));
        assert!(resolve_extension_connection_icon(&auto, Some(&contribution)).is_some());
    }

    #[test]
    fn missing_manifest_icon_and_selection_yield_none_for_the_caller_default() {
        let auto = connection(serde_json::json!({}));
        assert!(resolve_extension_connection_icon(&auto, Some(&contribution(None))).is_none());
        assert!(resolve_extension_connection_icon(&auto, None).is_none());
    }

    #[test]
    fn manifest_icon_that_no_longer_exists_is_ignored() {
        let contribution = contribution(Some("/no/such/icon.svg".into()));
        let auto = connection(serde_json::json!({}));
        assert!(resolve_extension_connection_icon(&auto, Some(&contribution)).is_none());
    }

    #[test]
    fn user_selection_wins_even_without_a_catalog() {
        let chosen = connection(serde_json::json!({ "icon": { "icon": "cisco" } }));
        assert!(resolve_extension_connection_icon(&chosen, None).is_some());
    }
}
