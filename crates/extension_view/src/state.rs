use gpui::SharedString;
use rust_i18n::t;

use crate::status_message::format_notification_error;
use crate::{ExtensionKind, ExtensionManagerMode, ExtensionSummary, MarketplaceEntry};

const MANIFEST_JSON_SUFFIX: &str = ".json";
const INSTALL_PROGRESS_VALUE: f32 = 28.0;
/// 浏览态分区顺序，与过滤 chips 一致。
pub(crate) const EXTENSION_KINDS: [ExtensionKind; 6] = [
    ExtensionKind::Language,
    ExtensionKind::LanguageBundle,
    ExtensionKind::DatabaseDriver,
    ExtensionKind::RemoteDesktopProvider,
    ExtensionKind::AcpAgent,
    ExtensionKind::Composite,
];

/// Market 分区：`kind` 为 `None` 表示扁平结果列表（搜索/有更新）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MarketplaceSection {
    pub kind: Option<ExtensionKind>,
    pub entries: Vec<MarketplaceEntry>,
}

/// 有搜索词 / 仅看更新 / 单类筛选时扁平展示；纯浏览时按 Kind 分区。
pub(crate) fn marketplace_sections(
    filtered: Vec<MarketplaceEntry>,
    selected_kind: Option<ExtensionKind>,
    query: &str,
    updates_only: bool,
) -> Vec<MarketplaceSection> {
    if filtered.is_empty() {
        return Vec::new();
    }
    if !query.trim().is_empty() || updates_only || selected_kind.is_some() {
        return vec![MarketplaceSection {
            kind: selected_kind,
            entries: filtered,
        }];
    }
    EXTENSION_KINDS
        .into_iter()
        .filter_map(|kind| {
            let entries: Vec<_> = filtered
                .iter()
                .filter(|entry| entry.kind == kind)
                .cloned()
                .collect();
            (!entries.is_empty()).then_some(MarketplaceSection {
                kind: Some(kind),
                entries,
            })
        })
        .collect()
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum MarketplaceLoadState {
    #[default]
    NotLoaded,
    Loading,
    Loaded,
    Failed(SharedString),
}

impl MarketplaceLoadState {
    pub(crate) fn is_loading(&self) -> bool {
        matches!(self, Self::Loading)
    }
}

pub(crate) fn should_auto_load_marketplace(
    mode: ExtensionManagerMode,
    marketplace_entries_empty: bool,
    marketplace_load_attempted: bool,
    loading: bool,
) -> bool {
    mode == ExtensionManagerMode::Marketplace
        && marketplace_entries_empty
        && !marketplace_load_attempted
        && !loading
}

pub(crate) fn apply_marketplace_load_result(
    marketplace_entries: &mut Vec<MarketplaceEntry>,
    load_state: &mut MarketplaceLoadState,
    status: &mut SharedString,
    outcome: anyhow::Result<Vec<MarketplaceEntry>>,
) -> Option<String> {
    match outcome {
        Ok(entries) => {
            *marketplace_entries = entries;
            *load_state = MarketplaceLoadState::Loaded;
            *status = t!(
                "Extension.loaded_marketplace",
                count = marketplace_entries.len()
            )
            .to_string()
            .into();
            None
        }
        Err(err) => {
            *load_state = MarketplaceLoadState::Failed(format!("{err:#}").into());
            *status = t!("Extension.load_marketplace_failed").to_string().into();
            Some(format_notification_error(
                &t!("Extension.load_marketplace_failed").to_string(),
                &err,
            ))
        }
    }
}

pub(crate) fn marketplace_manifest_url_from_query(query: &str) -> Option<String> {
    let trimmed = query.trim();
    if !is_http_url(trimmed) {
        return None;
    }
    has_json_path(trimmed).then(|| trimmed.to_string())
}

pub(crate) fn marketplace_filter_query(query: &str) -> &str {
    if marketplace_manifest_url_from_query(query).is_some() {
        ""
    } else {
        query
    }
}

pub(crate) fn install_progress_value(is_installing: bool) -> Option<f32> {
    is_installing.then_some(INSTALL_PROGRESS_VALUE)
}

pub(crate) fn apply_installed_reload_success(
    installed: &mut Vec<ExtensionSummary>,
    busy: &mut Option<String>,
    status: &mut SharedString,
    name: &str,
    reloaded: Vec<ExtensionSummary>,
) {
    *installed = reloaded;
    *busy = None;
    *status = t!("Extension.reloaded", name = name).to_string().into();
}

fn is_http_url(value: &str) -> bool {
    value.starts_with("https://") || value.starts_with("http://")
}

fn has_json_path(url: &str) -> bool {
    let without_fragment = url.split('#').next().unwrap_or(url);
    let path = without_fragment
        .split('?')
        .next()
        .unwrap_or(without_fragment);
    path.to_ascii_lowercase().ends_with(MANIFEST_JSON_SUFFIX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ExtensionKind;
    use ExtensionManagerMode::{Installed, Marketplace};

    #[test]
    fn marketplace_auto_load_runs_once_until_user_refreshes() {
        let cases = [
            (Marketplace, true, false, false, true),
            (Marketplace, true, true, false, false),
            (Marketplace, false, false, false, false),
            (Installed, true, false, false, false),
            (Marketplace, true, false, true, false),
        ];
        for (mode, entries_empty, attempted, loading, expected) in cases {
            assert_eq!(
                expected,
                should_auto_load_marketplace(mode, entries_empty, attempted, loading)
            );
        }
    }

    #[test]
    fn marketplace_load_success_replaces_entries_and_marks_loaded() {
        let mut entries = vec![marketplace_entry("old")];
        let mut load_state = MarketplaceLoadState::Loading;
        let mut status = SharedString::from(t!("Extension.loading_marketplace").to_string());

        apply_marketplace_load_result(
            &mut entries,
            &mut load_state,
            &mut status,
            Ok(vec![marketplace_entry("rust"), marketplace_entry("sql")]),
        );

        assert_eq!(MarketplaceLoadState::Loaded, load_state);
        assert_eq!(
            ["rust", "sql"],
            [entries[0].id.as_str(), entries[1].id.as_str()]
        );
        assert_eq!(
            t!("Extension.loaded_marketplace", count = 2).to_string(),
            status.as_ref()
        );
    }

    #[test]
    fn marketplace_load_failure_marks_failed_and_keeps_existing_entries() {
        let mut entries = vec![marketplace_entry("installed")];
        let mut load_state = MarketplaceLoadState::Loading;
        let mut status = SharedString::from(t!("Extension.loading_marketplace").to_string());

        let notification = apply_marketplace_load_result(
            &mut entries,
            &mut load_state,
            &mut status,
            Err(anyhow::anyhow!("network down")
                .context("fetch release manifest from https://example.test/manifest.json")),
        );

        let MarketplaceLoadState::Failed(detail) = &load_state else {
            panic!("failed marketplace load should retain an error detail");
        };
        assert!(detail.contains("https://example.test/manifest.json"));
        assert!(detail.contains("network down"));
        assert_eq!(["installed"], [entries[0].id.as_str()]);
        assert_eq!(
            t!("Extension.load_marketplace_failed").to_string(),
            status.as_ref()
        );
        let notification = notification.expect("失败时应该返回通知文案");
        assert!(notification.contains(t!("Extension.load_marketplace_failed").as_ref()));
        assert!(notification.contains("https://example.test/manifest.json"));
        assert!(notification.contains("network down"));
    }

    #[test]
    fn marketplace_manifest_url_from_query_accepts_http_json_manifest() {
        assert_eq!(
            Some("https://example.test/extensions/manifest.json".to_string()),
            marketplace_manifest_url_from_query(" https://example.test/extensions/manifest.json ")
        );
        assert_eq!(
            Some("http://example.test/manifest.json?ts=1".to_string()),
            marketplace_manifest_url_from_query("http://example.test/manifest.json?ts=1")
        );
    }

    #[test]
    fn marketplace_manifest_url_from_query_rejects_plain_search_and_assets() {
        assert_eq!(None, marketplace_manifest_url_from_query("rust"));
        assert_eq!(
            None,
            marketplace_manifest_url_from_query("https://example.test/package.tar.gz")
        );
        assert_eq!(
            None,
            marketplace_manifest_url_from_query("/tmp/extensions/manifest.json")
        );
    }

    #[test]
    fn install_progress_value_only_shows_while_installing() {
        assert_eq!(Some(28.0), install_progress_value(true));
        assert_eq!(None, install_progress_value(false));
    }

    #[test]
    fn installed_reload_success_replaces_entries_and_clears_busy_state() {
        let mut installed = vec![marketplace_summary("old", "1.0.0")];
        let mut busy = Some("reload:old".to_string());
        let mut status = SharedString::from("正在重新加载 old...");

        apply_installed_reload_success(
            &mut installed,
            &mut busy,
            &mut status,
            "old",
            vec![marketplace_summary("old", "1.0.1")],
        );

        assert_eq!(None, busy);
        assert_eq!(["1.0.1"], [installed[0].version.as_str()]);
        assert_eq!(
            t!("Extension.reloaded", name = "old").to_string(),
            status.as_ref()
        );
    }

    fn marketplace_summary(name: &str, version: &str) -> crate::ExtensionSummary {
        crate::ExtensionSummary::new(
            ExtensionKind::Composite,
            name,
            version,
            std::path::PathBuf::from(format!("/tmp/{name}")),
        )
    }

    fn marketplace_entry(id: &str) -> MarketplaceEntry {
        marketplace_entry_with_kind(id, ExtensionKind::Language)
    }

    fn marketplace_entry_with_kind(id: &str, kind: ExtensionKind) -> MarketplaceEntry {
        MarketplaceEntry {
            id: id.to_string(),
            kind,
            name: id.to_string(),
            version: "1.0.0".to_string(),
            description: String::new(),
            file_extensions: Vec::new(),
            required_host_version: None,
            host_compatible: true,
            asset_url: format!("https://example.test/{id}.tar.gz"),
            sha256: None,
            fallback_asset_url: None,
            manifest_url: None,
            manifest_fallback_url: None,
            screenshots: Vec::new(),
        }
    }

    #[test]
    fn browse_mode_groups_marketplace_by_kind_in_stable_order() {
        let filtered = vec![
            marketplace_entry_with_kind("pg", ExtensionKind::DatabaseDriver),
            marketplace_entry_with_kind("rust", ExtensionKind::Language),
            marketplace_entry_with_kind("mysql", ExtensionKind::DatabaseDriver),
        ];

        let sections = marketplace_sections(filtered, None, "", false);

        assert_eq!(
            [
                Some(ExtensionKind::Language),
                Some(ExtensionKind::DatabaseDriver)
            ],
            sections
                .iter()
                .map(|section| section.kind)
                .collect::<Vec<_>>()
                .as_slice()
        );
        assert_eq!(1, sections[0].entries.len());
        assert_eq!(2, sections[1].entries.len());
    }

    #[test]
    fn active_filter_flattens_marketplace_into_single_section() {
        let filtered = vec![
            marketplace_entry_with_kind("rust", ExtensionKind::Language),
            marketplace_entry_with_kind("pg", ExtensionKind::DatabaseDriver),
        ];

        let by_query = marketplace_sections(filtered.clone(), None, "ru", false);
        let by_updates = marketplace_sections(filtered.clone(), None, "", true);
        let by_kind = marketplace_sections(filtered, Some(ExtensionKind::Language), "", false);

        assert_eq!(1, by_query.len());
        assert_eq!(None, by_query[0].kind);
        assert_eq!(2, by_query[0].entries.len());
        assert_eq!(1, by_updates.len());
        assert_eq!(None, by_updates[0].kind);
        assert_eq!(1, by_kind.len());
        assert_eq!(Some(ExtensionKind::Language), by_kind[0].kind);
    }

    #[test]
    fn empty_marketplace_filter_yields_no_sections() {
        assert!(marketplace_sections(Vec::new(), None, "", false).is_empty());
    }
}
