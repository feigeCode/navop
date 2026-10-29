//! 更新 feed 与当前版本的运行时配置。
//!
//! 独立产品各自使用自己那份更新清单；全量产品沿用历史路径，保证
//! 已发布的老版本还能收到升级提示。

use std::sync::OnceLock;

use domain_api::AppIdentity;
use one_core::config::{UpdateConfig, public_base_url, update_url_from_public_base};

/// 运行时覆盖更新接口地址；与 `one_core::config` 共用同一个环境变量。
const UPDATE_URL_ENV: &str = "NAVOP_UPDATE_URL";
/// 编译期注入的更新接口地址。
const BUILD_TIME_UPDATE_URL: Option<&str> = option_env!("NAVOP_UPDATE_URL");
/// 更新下载页地址（可选）。
const UPDATE_DOWNLOAD_URL_ENV: &str = "NAVOP_UPDATE_DOWNLOAD_URL";
/// 编译期注入的更新下载页地址。
const BUILD_TIME_DOWNLOAD_URL: Option<&str> = option_env!("NAVOP_UPDATE_DOWNLOAD_URL");
/// 更新清单所在的分发子目录。
const UPDATE_ROOT: &str = "updates";
/// 统一更新清单文件名。
const UPDATE_MANIFEST: &str = "latest.json";
/// 全量产品的 `app_id`：它的 feed 不含产品段，保持历史路径。
const LEGACY_FULL_APP_ID: &str = "navop";

/// 当前产品版本号；未覆盖时用 `app_kit` 自身版本（workspace 统一）。
static CURRENT_VERSION: OnceLock<&'static str> = OnceLock::new();

/// 覆盖当前产品版本号；只有第一次调用生效。
pub fn set_current_version(version: &'static str) {
    let _ = CURRENT_VERSION.set(version);
}

/// 当前产品版本号。
pub fn current_version() -> &'static str {
    CURRENT_VERSION
        .get()
        .copied()
        .unwrap_or(env!("CARGO_PKG_VERSION"))
}

/// 当前产品使用的更新配置。
pub fn update_config() -> UpdateConfig {
    let runtime_url = override_update_url();
    let base_url = public_base_url();
    let mut config = match crate::identity() {
        Some(identity) => {
            derive_update_config(runtime_url.as_deref(), base_url.as_deref(), identity)
        }
        // 尚未 bootstrap 时无法派生产品 feed，只能退化为「仅环境变量覆盖」。
        None => UpdateConfig {
            update_url: runtime_url.unwrap_or_default(),
            download_url: None,
        },
    };
    config.download_url = configured_download_url();
    config
}

/// 是否值得发起更新检查。
///
/// 有 feed 就检查；全量产品在没有 feed 时仍回退到 GitHub Releases，这条历史
/// 路径由更新源选择逻辑处理，所以这里不能把它一起关掉。
pub fn update_check_available(config: &UpdateConfig) -> bool {
    config.is_valid() || crate::identity().is_some_and(is_legacy_full)
}

/// 纯函数版本：由覆盖地址、站点根地址与产品身份派生更新配置。
pub fn derive_update_config(
    runtime: Option<&str>,
    base: Option<&str>,
    identity: &AppIdentity,
) -> UpdateConfig {
    let update_url = normalize_url(runtime.map(ToOwned::to_owned))
        .or_else(|| base.and_then(|base| feed_url(base, identity)))
        .unwrap_or_default();
    UpdateConfig {
        update_url,
        download_url: None,
    }
}

/// 一次性的覆盖地址：运行时环境变量优先，其次是编译期注入值。
fn override_update_url() -> Option<String> {
    normalize_url(std::env::var(UPDATE_URL_ENV).ok())
        .or_else(|| normalize_url(BUILD_TIME_UPDATE_URL.map(ToOwned::to_owned)))
}

/// 更新下载页地址，规则与 `one_core::config::UpdateConfig` 一致。
fn configured_download_url() -> Option<String> {
    normalize_url(std::env::var(UPDATE_DOWNLOAD_URL_ENV).ok())
        .or_else(|| normalize_url(BUILD_TIME_DOWNLOAD_URL.map(ToOwned::to_owned)))
}

fn normalize_url(value: Option<String>) -> Option<String> {
    value
        .map(|url| url.trim().to_string())
        .filter(|url| !url.is_empty())
}

/// 产品自己的更新清单地址。
///
/// 全量产品保持 `{base}/updates/latest.json`；独立产品带上产品名与发布通道，
/// 三者的 feed 因此互不覆盖。
fn feed_url(base: &str, identity: &AppIdentity) -> Option<String> {
    let base = base.trim().trim_end_matches('/');
    if base.is_empty() {
        return None;
    }
    if is_legacy_full(identity) {
        return Some(update_url_from_public_base(base));
    }
    let channel = identity.mcp_update_channel.trim();
    Some(if channel.is_empty() {
        format!(
            "{base}/{UPDATE_ROOT}/{}/{UPDATE_MANIFEST}",
            identity.binary_name
        )
    } else {
        format!(
            "{base}/{UPDATE_ROOT}/{}/{channel}/{UPDATE_MANIFEST}",
            identity.binary_name
        )
    })
}

fn is_legacy_full(identity: &AppIdentity) -> bool {
    identity.app_id == LEGACY_FULL_APP_ID
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = "https://cdn.example.test";

    #[test]
    fn runtime_override_wins_over_the_product_feed() {
        let config = derive_update_config(
            Some("https://mirror.example.test/latest.json"),
            Some(BASE),
            &AppIdentity::FULL,
        );
        assert_eq!(config.update_url, "https://mirror.example.test/latest.json");
        assert_eq!(config.download_url, None);
    }

    #[test]
    fn blank_runtime_override_falls_back_to_the_product_feed() {
        let config = derive_update_config(Some("  "), Some(BASE), &AppIdentity::FULL);
        assert_eq!(
            config.update_url,
            "https://cdn.example.test/updates/latest.json"
        );
    }

    #[test]
    fn full_product_keeps_the_legacy_feed_path() {
        let config =
            derive_update_config(None, Some("https://cdn.example.test/"), &AppIdentity::FULL);
        assert_eq!(
            config.update_url,
            "https://cdn.example.test/updates/latest.json"
        );
    }

    #[test]
    fn split_products_use_their_own_feed() {
        let full = derive_update_config(None, Some(BASE), &AppIdentity::FULL);
        let shell = derive_update_config(None, Some(BASE), &AppIdentity::SHELL);
        let data = derive_update_config(None, Some(BASE), &AppIdentity::DATA);

        assert_eq!(
            shell.update_url,
            "https://cdn.example.test/updates/navop-shell/shell/latest.json"
        );
        assert_eq!(
            data.update_url,
            "https://cdn.example.test/updates/navop-data/data/latest.json"
        );
        assert_ne!(shell.update_url, data.update_url);
        assert_ne!(shell.update_url, full.update_url);
        assert_ne!(data.update_url, full.update_url);
    }

    #[test]
    fn missing_base_url_disables_the_feed_without_panicking() {
        let config = derive_update_config(None, None, &AppIdentity::SHELL);
        assert!(config.update_url.is_empty());
        assert!(!config.is_valid());
        assert!(!update_check_available(&config));
    }

    #[test]
    fn current_version_can_only_be_overridden_once() {
        assert_eq!(current_version(), env!("CARGO_PKG_VERSION"));
        set_current_version("9.9.9");
        set_current_version("1.0.0");
        assert_eq!(current_version(), "9.9.9");
    }
}
