rust_i18n::i18n!("locales", fallback = "en");

mod actions;
mod host;
mod model;
mod offline_package_dialog;
mod permissions;
mod render;
mod state;
mod status_message;
mod view;

pub use host::ExtensionViewHost;
pub use model::{
    DownloadedMarketplaceExtension, ExtensionKind, ExtensionSummary, MarketplaceEntry,
    MarketplaceInstallOutcome, MarketplaceInstallState, PermissionReviewModel, filter_installed,
    filter_marketplace, marketplace_entry_install_id, marketplace_install_state,
};
pub use view::{ExtensionManagerMode, ExtensionManagerView};

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{
        ExtensionKind, ExtensionSummary, MarketplaceEntry, MarketplaceInstallState,
        filter_installed, filter_marketplace, marketplace_install_state,
    };

    #[test]
    fn marketplace_state_marks_matching_extension_installed() {
        let installed = vec![summary(
            ExtensionKind::Composite,
            "com.example.tools",
            "1.2.0",
        )];
        let entry = marketplace_entry(ExtensionKind::Composite, "com.example.tools", "1.2.0");

        let state = marketplace_install_state(&installed, &entry);

        assert_eq!(MarketplaceInstallState::Installed, state);
    }

    #[test]
    fn marketplace_state_detects_newer_marketplace_version() {
        let installed = vec![summary(ExtensionKind::Language, "rust", "1.2.0")];
        let entry = marketplace_entry(ExtensionKind::Language, "rust", "1.3.0");

        let state = marketplace_install_state(&installed, &entry);

        assert_eq!(MarketplaceInstallState::UpdateAvailable, state);
    }

    #[test]
    fn marketplace_state_uses_name_when_entry_id_is_missing() {
        let installed = vec![summary(ExtensionKind::DatabaseDriver, "fake_pg", "0.1.0")];
        let mut entry = marketplace_entry(ExtensionKind::DatabaseDriver, "fake_pg", "0.1.0");
        entry.id.clear();

        let state = marketplace_install_state(&installed, &entry);

        assert_eq!(MarketplaceInstallState::Installed, state);
    }

    #[test]
    fn installed_filter_matches_kind_name_and_description() {
        let installed = vec![
            summary(ExtensionKind::Language, "rust", "1.0.0").with_description("Rust syntax"),
            summary(ExtensionKind::Composite, "sql-tools", "1.0.0")
                .with_description("Database helpers"),
        ];

        let filtered = filter_installed(&installed, "data", Some(ExtensionKind::Composite));

        assert_eq!(1, filtered.len());
        assert_eq!("sql-tools", filtered[0].name);
    }

    #[test]
    fn marketplace_filter_matches_kind_name_and_file_extensions() {
        let mut entries = vec![
            marketplace_entry(ExtensionKind::Language, "rust", "1.0.0"),
            marketplace_entry(ExtensionKind::DatabaseDriver, "fake_pg", "1.0.0"),
        ];
        entries[1].file_extensions = vec!["psql".to_string()];

        let filtered = filter_marketplace(&entries, "psql", Some(ExtensionKind::DatabaseDriver));

        assert_eq!(1, filtered.len());
        assert_eq!("fake_pg", filtered[0].id);
    }

    fn summary(kind: ExtensionKind, name: &str, version: &str) -> ExtensionSummary {
        ExtensionSummary::new(kind, name, version, PathBuf::from(format!("/tmp/{name}")))
    }

    fn marketplace_entry(kind: ExtensionKind, id: &str, version: &str) -> MarketplaceEntry {
        MarketplaceEntry {
            id: id.to_string(),
            kind,
            name: id.to_string(),
            version: version.to_string(),
            description: String::new(),
            file_extensions: Vec::new(),
            asset_url: "https://example.test/ext.tar.gz".to_string(),
            sha256: None,
            fallback_asset_url: None,
            manifest_url: None,
            manifest_fallback_url: None,
        }
    }
}
