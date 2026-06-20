use gpui::SharedString;
use rust_i18n::t;

use crate::status_message::format_notification_error;
use crate::{ExtensionManagerMode, MarketplaceEntry};

const MANIFEST_JSON_SUFFIX: &str = ".json";
const INSTALL_PROGRESS_VALUE: f32 = 28.0;

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
    loading: &mut bool,
    status: &mut SharedString,
    outcome: anyhow::Result<Vec<MarketplaceEntry>>,
) -> Option<String> {
    *loading = false;
    match outcome {
        Ok(entries) => {
            *marketplace_entries = entries;
            *status = t!(
                "Extension.loaded_marketplace",
                count = marketplace_entries.len()
            )
            .to_string()
            .into();
            None
        }
        Err(err) => {
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
    fn marketplace_load_success_replaces_entries_and_clears_loading() {
        let mut entries = vec![marketplace_entry("old")];
        let mut loading = true;
        let mut status = SharedString::from(t!("Extension.loading_marketplace").to_string());

        apply_marketplace_load_result(
            &mut entries,
            &mut loading,
            &mut status,
            Ok(vec![marketplace_entry("rust"), marketplace_entry("sql")]),
        );

        assert!(!loading);
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
    fn marketplace_load_failure_clears_loading_and_keeps_existing_entries() {
        let mut entries = vec![marketplace_entry("installed")];
        let mut loading = true;
        let mut status = SharedString::from(t!("Extension.loading_marketplace").to_string());

        let notification = apply_marketplace_load_result(
            &mut entries,
            &mut loading,
            &mut status,
            Err(anyhow::anyhow!("network down")
                .context("fetch release manifest from https://example.test/manifest.json")),
        );

        assert!(!loading);
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

    fn marketplace_entry(id: &str) -> MarketplaceEntry {
        MarketplaceEntry {
            id: id.to_string(),
            kind: ExtensionKind::Language,
            name: id.to_string(),
            version: "1.0.0".to_string(),
            description: String::new(),
            file_extensions: Vec::new(),
            asset_url: format!("https://example.test/{id}.tar.gz"),
            sha256: None,
            fallback_asset_url: None,
            manifest_url: None,
            manifest_fallback_url: None,
        }
    }
}
