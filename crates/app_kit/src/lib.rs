//! 三个产品共享的应用启动辅助层。
//!
//! `app_kit` 只负责把产品身份落到进程级配置（数据目录、keychain），
//! 再调用产品自己的领域初始化；它不依赖任何具体领域 crate。

use domain_api::{AppIdentity, Domain, DomainId, DomainRegistry, Product};
use gpui::App;
use std::sync::OnceLock;

rust_i18n::i18n!("locales", fallback = "en");

pub mod actions;
pub mod connection_actions;
pub mod file_association;
pub mod host;
pub mod import;
pub mod integration;
pub mod new_connection;
pub mod settings;
pub mod tabs;
pub mod update;
pub mod visuals;
pub mod vault;

pub use connection_actions::{
    delete_connection, duplicate_connection, edit_connection, open_new_connection,
    open_new_connection_in_workspace,
};
pub use host::{ShellHost, set_shell_host, shell_host};
pub use settings::open_settings;

static DOMAIN_REGISTRY: OnceLock<DomainRegistry> = OnceLock::new();
static EMPTY_DOMAIN_REGISTRY: OnceLock<DomainRegistry> = OnceLock::new();
static APP_IDENTITY: OnceLock<&'static AppIdentity> = OnceLock::new();
static DOMAINS: OnceLock<Vec<std::sync::Arc<dyn Domain>>> = OnceLock::new();

/// 当前产品的身份；`bootstrap` 之后可用。
pub fn identity() -> Option<&'static AppIdentity> {
    APP_IDENTITY.get().copied()
}

/// 当前产品注册的领域表；未 bootstrap 时返回空表。
pub fn registry() -> &'static DomainRegistry {
    DOMAIN_REGISTRY
        .get()
        .unwrap_or_else(|| EMPTY_DOMAIN_REGISTRY.get_or_init(DomainRegistry::default))
}

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
    let _ = public_mcp::discovery::configure_discovery_identity(
        public_mcp::discovery::DiscoveryIdentity {
            app: identity.mcp_app_name,
            legacy: identity.mcp_legacy_app_name,
            file_name: "public-mcp.json",
        },
    );
}

/// 应用身份后初始化产品领域。
///
/// 返回错误由调用方决定是否退出；`app_kit` 不在这里 `expect`，避免把
/// 各产品的错误处理策略写死。
pub fn bootstrap<P: Product>(product: &P, cx: &mut App) -> anyhow::Result<()> {
    let _ = APP_IDENTITY.set(product.identity());
    configure_identity(product.identity());
    if one_core::app_paths::initialized_paths().is_none() {
        let context = one_core::app_paths::process_context()?;
        one_core::app_paths::initialize_app_paths(&Default::default(), &context)?;
    }
    let data_dir = one_core::app_paths::initialized_paths()
        .and_then(|paths| paths.data_dir())
        .ok_or_else(|| anyhow::anyhow!("app data directory is not initialized"))?;
    let (canonical, legacy) = public_mcp::discovery::public_mcp_discovery_paths_in(data_dir);
    let _ = public_mcp::discovery::set_public_mcp_discovery_paths(canonical, legacy);
    let domains = product.domains();
    let mut registry = DomainRegistry::default();
    for domain in &domains {
        registry.register_domain(domain.id());
        domain.register(&mut registry);
    }
    let _ = DOMAINS.set(domains);
    let _ = DOMAIN_REGISTRY.set(registry);
    product.init(cx)?;
    if product.mcp_managed_by_app_kit() {
        if product.features().public_mcp {
            enable_default_toolsets(cx);
            mcp_host::start(cx, product.identity())?;
        } else {
            mcp_host::stop(cx);
        }
    }
    Ok(())
}

/// 调用当前产品每个领域的初始化（`Domain::init`）。
///
/// 领域初始化包含数据库缓存、SQL 编辑器、Redis/Mongo 视图、终端视图等
/// 领域专属子系统；公共壳只按注册顺序调用，不知道具体是什么。
pub fn init_domains(cx: &mut App) -> anyhow::Result<()> {
    let Some(domains) = DOMAINS.get() else {
        return Ok(());
    };
    for domain in domains {
        domain.init(cx)?;
    }
    Ok(())
}

/// 收集各领域的关停任务，退出时按注册顺序 await。
pub fn shutdown_domains(cx: &mut App) -> Vec<gpui::Task<()>> {
    let Some(domains) = DOMAINS.get() else {
        return Vec::new();
    };
    domains
        .iter()
        .filter_map(|domain| domain.shutdown(cx))
        .collect()
}

/// 按注册顺序让每个领域注册自己的快捷键。
pub fn refresh_domain_keybindings(cx: &mut App) {
    let Some(domains) = DOMAINS.get() else {
        return;
    };
    for domain in domains {
        domain.refresh_keybindings(cx);
    }
}

/// 首次启动写入本产品的默认 MCP 工具集，与领域包保持一致。
///
/// 工具集开关一次写盘后由用户接管：后续只改设置里的勾选，不再覆盖。
fn enable_default_toolsets(cx: &mut App) {
    let mut toolsets = vec![mcp_host::McpToolset::Connections];
    for domain in registry().domains() {
        match *domain {
            DomainId::SHELL => {
                toolsets.extend([mcp_host::McpToolset::Terminal, mcp_host::McpToolset::Sftp])
            }
            DomainId::DATABASE => toolsets.extend([
                mcp_host::McpToolset::Database,
                mcp_host::McpToolset::Redis,
                mcp_host::McpToolset::Mongo,
            ]),
            _ => {}
        }
    }
    mcp_host::enable_default_toolsets(cx, &toolsets);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_identity_keeps_legacy_directory_and_keychain() {
        assert_eq!(AppIdentity::FULL.data_dir_name, "navop");
        assert_eq!(
            AppIdentity::FULL.dir_migration_marker,
            ".one-hub-migration-complete"
        );
        assert_eq!(
            AppIdentity::FULL.development_keychain_service,
            "com.navop.desktop.dev"
        );
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
        assert_ne!(
            AppIdentity::SHELL.keychain_service,
            AppIdentity::DATA.keychain_service
        );
    }
}
