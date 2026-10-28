use gpui::{App, AssetSource};
use std::sync::Arc;

/// 产品身份。三产品只在各自的 `Product` 实现里提供不同值。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AppIdentity {
    pub app_id: &'static str,
    pub display_name: &'static str,
    pub bundle_id: &'static str,
    pub binary_name: &'static str,
    pub data_dir_name: &'static str,
    pub legacy_data_dir_names: &'static [&'static str],
    pub keychain_service: &'static str,
    pub development_keychain_service: &'static str,
    pub dir_migration_marker: &'static str,
    pub portable_marker: &'static str,
    pub mcp_app_name: &'static str,
    pub mcp_update_channel: &'static str,
    pub file_associations: &'static [&'static str],
}

impl AppIdentity {
    /// 全量版沿用既有身份，避免老用户迁移数据目录与 keychain。
    pub const FULL: Self = Self {
        app_id: "navop",
        display_name: "Navop",
        bundle_id: "com.onetcli.app",
        binary_name: "navop",
        data_dir_name: "navop",
        legacy_data_dir_names: &["one-hub"],
        keychain_service: "com.navop.desktop",
        development_keychain_service: "com.navop.desktop.dev",
        dir_migration_marker: ".one-hub-migration-complete",
        portable_marker: "navop.portable",
        mcp_app_name: "navop",
        mcp_update_channel: "stable",
        file_associations: &["db", "duckdb", "md", "cast"],
    };

    pub const SHELL: Self = Self {
        app_id: "navop-shell",
        display_name: "Navop Shell",
        bundle_id: "com.navop.shell",
        binary_name: "navop-shell",
        data_dir_name: "navop-shell",
        legacy_data_dir_names: &[],
        keychain_service: "com.navop.desktop.shell",
        development_keychain_service: "com.navop.desktop.shell.dev",
        dir_migration_marker: ".navop-shell-migration-complete",
        portable_marker: "navop-shell.portable",
        mcp_app_name: "navop-shell",
        mcp_update_channel: "shell",
        file_associations: &[],
    };

    pub const DATA: Self = Self {
        app_id: "navop-data",
        display_name: "Navop Data",
        bundle_id: "com.navop.data",
        binary_name: "navop-data",
        data_dir_name: "navop-data",
        legacy_data_dir_names: &[],
        keychain_service: "com.navop.desktop.data",
        development_keychain_service: "com.navop.desktop.data.dev",
        dir_migration_marker: ".navop-data-migration-complete",
        portable_marker: "navop-data.portable",
        mcp_app_name: "navop-data",
        mcp_update_channel: "data",
        file_associations: &[],
    };
}

/// 与具体领域无关的产品级功能开关；领域裁剪由 `Product::domains` 决定。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProductFeatures {
    pub ai: bool,
    pub notes: bool,
    pub team: bool,
    pub cloud_sync: bool,
    pub credential_vault: bool,
    pub toolbox: bool,
    pub public_mcp: bool,
}

impl ProductFeatures {
    pub const FULL: Self = Self {
        ai: true,
        notes: true,
        team: true,
        cloud_sync: true,
        credential_vault: true,
        toolbox: true,
        public_mcp: true,
    };

    pub const SHELL: Self = Self {
        public_mcp: false,
        ..Self::FULL
    };

    pub const DATA: Self = Self {
        public_mcp: false,
        ..Self::FULL
    };
}

/// 一个产品由身份、功能开关、资产与领域列表组成。
pub trait Product: 'static {
    type Assets: AssetSource;

    fn identity(&self) -> &'static AppIdentity;

    fn features(&self) -> ProductFeatures;

    fn assets(&self) -> Self::Assets;

    fn domains(&self) -> Vec<Arc<dyn super::Domain>>;

    fn init(&self, cx: &mut App) {
        for domain in self.domains() {
            domain.init(cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn three_products_have_distinct_identities() {
        assert_ne!(AppIdentity::FULL.app_id, AppIdentity::SHELL.app_id);
        assert_ne!(AppIdentity::FULL.bundle_id, AppIdentity::DATA.bundle_id);
        assert_ne!(AppIdentity::SHELL.binary_name, AppIdentity::DATA.binary_name);
        assert_eq!(AppIdentity::FULL.data_dir_name, "navop");
    }

    #[test]
    fn standalone_products_disable_public_mcp_only() {
        assert!(ProductFeatures::FULL.public_mcp);
        assert!(!ProductFeatures::SHELL.public_mcp);
        assert!(!ProductFeatures::DATA.public_mcp);
        assert!(ProductFeatures::SHELL.ai);
        assert!(ProductFeatures::DATA.cloud_sync);
    }
}
