//! 三个产品共享的应用启动辅助层。
//!
//! `app_kit` 只负责把产品身份落到进程级配置（数据目录、keychain），
//! 再调用产品自己的领域初始化；它不依赖任何具体领域 crate。

use domain_api::{AppIdentity, Product};
use gpui::App;

/// 在访问任何数据目录 / keychain 之前应用产品身份。
///
/// 必须在 `one_core::app_paths::initialize_app_paths` 之前调用。
pub fn configure_identity(identity: &'static AppIdentity) {
    one_core::app_dirs::configure_app_dir_names(
        identity.data_dir_name,
        identity.legacy_data_dir_names,
        identity.dir_migration_marker,
    );
    one_core::key_storage::configure_keyring_services(
        identity.keychain_service,
        identity.development_keychain_service,
    );
    one_core::app_paths::configure_app_path_names(
        identity.portable_marker,
        identity.development_data_dir_name,
        identity.data_dir_name,
    );
}

/// 应用身份后初始化产品领域。
///
/// 返回错误由调用方决定是否退出；`app_kit` 不在这里 `expect`，避免把
/// 各产品的错误处理策略写死。
pub fn bootstrap<P: Product>(product: &P, cx: &mut App) -> anyhow::Result<()> {
    configure_identity(product.identity());
    product.init(cx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_identity_keeps_legacy_directory_and_keychain() {
        assert_eq!(AppIdentity::FULL.data_dir_name, "navop");
        assert_eq!(AppIdentity::FULL.dir_migration_marker, ".one-hub-migration-complete");
        assert_eq!(AppIdentity::FULL.development_keychain_service, "com.navop.desktop.dev");
    }

    #[test]
    fn configure_identity_switches_app_directory_name() {
        configure_identity(&AppIdentity::SHELL);
        assert_eq!(one_core::app_dirs::current_app_dir_name(), "navop-shell");
    }

    #[test]
    fn standalone_identities_use_isolated_directories() {
        assert_eq!(AppIdentity::SHELL.data_dir_name, "navop-shell");
        assert_eq!(AppIdentity::DATA.data_dir_name, "navop-data");
        assert_ne!(AppIdentity::SHELL.keychain_service, AppIdentity::DATA.keychain_service);
    }
}
