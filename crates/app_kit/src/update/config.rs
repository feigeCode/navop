//! 更新 feed 与当前版本的运行时配置。
//!
//! 独立产品各自使用自己那份更新清单；全量产品沿用历史路径，保证
//! 已发布的老版本还能收到升级提示。
//!
//! 稳定通道与测试通道各有一份清单（测试通道在 `updates/` 下多一层 `beta/`），
//! 稳定清单的路径一个字都没改，所以 beta 机制上线不会让老版本收到预发布版本。

use std::sync::OnceLock;

use domain_api::AppIdentity;
use one_core::config::{UpdateConfig, public_base_url, update_url_from_public_base};
use one_core::settings::UpdateChannel;

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
/// 测试通道清单所在的子目录。
const BETA_CHANNEL_SEGMENT: &str = "beta";
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

/// 当前实际生效的更新通道。
///
/// 用户显式选了通道就听用户的；没选过则跟随当前构建：预发布构建（`0.23.0-beta.2`）
/// 自动走测试通道，正式构建走稳定通道。
pub fn effective_channel(configured: Option<UpdateChannel>) -> UpdateChannel {
    UpdateChannel::resolve(configured, current_version())
}

/// 当前产品在指定通道上使用的更新配置。
pub fn update_config(channel: UpdateChannel) -> UpdateConfig {
    let runtime_url = override_update_url();
    let base_url = public_base_url();
    let mut config = match crate::identity() {
        Some(identity) => derive_update_config(
            runtime_url.as_deref(),
            base_url.as_deref(),
            identity,
            channel,
        ),
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

/// 纯函数版本：由覆盖地址、站点根地址、产品身份与更新通道派生更新配置。
pub fn derive_update_config(
    runtime: Option<&str>,
    base: Option<&str>,
    identity: &AppIdentity,
    channel: UpdateChannel,
) -> UpdateConfig {
    let update_url = normalize_url(runtime.map(ToOwned::to_owned))
        .or_else(|| base.and_then(|base| feed_url(base, identity, channel)))
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

/// 拼接 `{base}/updates/<segments…>/latest.json`，空段自动跳过。
fn updates_url(base: &str, segments: &[&str]) -> String {
    let mut url = format!("{base}/{UPDATE_ROOT}");
    for segment in segments
        .iter()
        .copied()
        .filter(|segment| !segment.is_empty())
    {
        url.push('/');
        url.push_str(segment);
    }
    url.push('/');
    url.push_str(UPDATE_MANIFEST);
    url
}

/// 产品在指定通道上的更新清单地址。
///
/// 稳定通道保持历史路径（全量产品 `{base}/updates/latest.json`，独立产品带上产品名
/// 与发布通道）；测试通道在其后多一层 `beta/`，因此两份清单互不覆盖。
fn feed_url(base: &str, identity: &AppIdentity, channel: UpdateChannel) -> Option<String> {
    let base = base.trim().trim_end_matches('/');
    if base.is_empty() {
        return None;
    }
    if channel == UpdateChannel::Stable {
        if is_legacy_full(identity) {
            return Some(update_url_from_public_base(base));
        }
        return Some(updates_url(
            base,
            &[identity.binary_name, identity.mcp_update_channel.trim()],
        ));
    }
    if is_legacy_full(identity) {
        return Some(updates_url(base, &[BETA_CHANNEL_SEGMENT]));
    }
    Some(updates_url(
        base,
        &[
            identity.binary_name,
            identity.mcp_update_channel.trim(),
            BETA_CHANNEL_SEGMENT,
        ],
    ))
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
            UpdateChannel::Stable,
        );
        assert_eq!(config.update_url, "https://mirror.example.test/latest.json");
        assert_eq!(config.download_url, None);
    }

    #[test]
    fn blank_runtime_override_falls_back_to_the_product_feed() {
        let config = derive_update_config(
            Some("  "),
            Some(BASE),
            &AppIdentity::FULL,
            UpdateChannel::Stable,
        );
        assert_eq!(
            config.update_url,
            "https://cdn.example.test/updates/latest.json"
        );
    }

    #[test]
    fn full_product_keeps_the_legacy_feed_path() {
        let config = derive_update_config(
            None,
            Some("https://cdn.example.test/"),
            &AppIdentity::FULL,
            UpdateChannel::Stable,
        );
        assert_eq!(
            config.update_url,
            "https://cdn.example.test/updates/latest.json"
        );
    }

    #[test]
    fn split_products_use_their_own_feed() {
        let full =
            derive_update_config(None, Some(BASE), &AppIdentity::FULL, UpdateChannel::Stable);
        let shell =
            derive_update_config(None, Some(BASE), &AppIdentity::SHELL, UpdateChannel::Stable);
        let data =
            derive_update_config(None, Some(BASE), &AppIdentity::DATA, UpdateChannel::Stable);

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
        let config = derive_update_config(None, None, &AppIdentity::SHELL, UpdateChannel::Stable);
        assert!(config.update_url.is_empty());
        assert!(!config.is_valid());
        assert!(!update_check_available(&config));
    }

    #[test]
    fn beta_feed_sits_beside_the_stable_one_without_shadowing_it() {
        let stable =
            derive_update_config(None, Some(BASE), &AppIdentity::FULL, UpdateChannel::Stable);
        let beta = derive_update_config(None, Some(BASE), &AppIdentity::FULL, UpdateChannel::Beta);

        assert_eq!(
            stable.update_url,
            "https://cdn.example.test/updates/latest.json"
        );
        assert_eq!(
            beta.update_url,
            "https://cdn.example.test/updates/beta/latest.json"
        );
    }

    #[test]
    fn split_product_beta_feed_appends_the_channel_segment() {
        let beta = derive_update_config(None, Some(BASE), &AppIdentity::SHELL, UpdateChannel::Beta);

        assert_eq!(
            beta.update_url,
            "https://cdn.example.test/updates/navop-shell/shell/beta/latest.json"
        );
    }

    #[test]
    fn effective_channel_prefers_the_explicit_choice() {
        // 这里只验证显式选择优先：全局 current_version 会被别的测试覆盖，不在此断言默认值。
        assert_eq!(
            UpdateChannel::Stable,
            effective_channel(Some(UpdateChannel::Stable))
        );
        assert_eq!(
            UpdateChannel::Beta,
            effective_channel(Some(UpdateChannel::Beta))
        );
    }

    #[test]
    fn current_version_can_only_be_overridden_once() {
        assert_eq!(current_version(), env!("CARGO_PKG_VERSION"));
        set_current_version("9.9.9");
        set_current_version("1.0.0");
        assert_eq!(current_version(), "9.9.9");
    }
}
