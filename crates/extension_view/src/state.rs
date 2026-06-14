use gpui::SharedString;

use crate::status_message::format_status_error;
use crate::{ExtensionManagerMode, MarketplaceEntry};

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
) {
    *loading = false;
    match outcome {
        Ok(entries) => {
            *marketplace_entries = entries;
            *status = format!("已加载 {} 个市场扩展", marketplace_entries.len()).into();
        }
        Err(err) => *status = format_status_error("加载扩展市场失败", &err).into(),
    }
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
        let mut status = SharedString::from("正在加载扩展市场...");

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
        assert_eq!("已加载 2 个市场扩展", status.as_ref());
    }

    #[test]
    fn marketplace_load_failure_clears_loading_and_keeps_existing_entries() {
        let mut entries = vec![marketplace_entry("installed")];
        let mut loading = true;
        let mut status = SharedString::from("正在加载扩展市场...");

        apply_marketplace_load_result(
            &mut entries,
            &mut loading,
            &mut status,
            Err(anyhow::anyhow!("network down")),
        );

        assert!(!loading);
        assert_eq!(["installed"], [entries[0].id.as_str()]);
        assert!(status.as_ref().contains("加载扩展市场失败"));
        assert!(status.as_ref().contains("network down"));
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
        }
    }
}
