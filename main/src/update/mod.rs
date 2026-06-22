use std::path::PathBuf;

use gpui::{App, AppContext, Window};
use gpui_component::WindowExt;
use one_core::gpui_tokio::Tokio;
use rust_i18n::t;

use crate::setting_tab::AppSettings;
use one_core::config::UpdateConfig;

mod custom_api;
mod dialog;
mod download;
mod extract;
mod github_release;
mod install;
mod network;
mod util;

use custom_api::{
    fetch_update_info, select_download_url, select_fallback_download_url, select_sha256,
};
use dialog::show_update_dialog;
use github_release::{fetch_github_release, github_release_to_dialog_info};
use install::{apply_update_helper, cleanup_stale_update_backups};
use network::check_network_connectivity;
use util::parse_version;

const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
const APPLY_UPDATE_FLAG: &str = "--apply-update";

/// 当前使用的更新源。默认走 R2 JSON；启用 `github-updates` feature 时切回 GitHub Releases。
#[cfg(test)]
#[cfg(feature = "github-updates")]
const ACTIVE_UPDATE_SOURCE: UpdateSource = UpdateSource::GitHub;
#[cfg(test)]
#[cfg(not(feature = "github-updates"))]
const ACTIVE_UPDATE_SOURCE: UpdateSource = UpdateSource::CustomApi;
#[cfg(feature = "github-updates")]
const GITHUB_UPDATE_SOURCES: &[UpdateSource] = &[UpdateSource::GitHub];
#[cfg(not(feature = "github-updates"))]
const GITHUB_UPDATE_SOURCES: &[UpdateSource] = &[UpdateSource::GitHub];
#[cfg(not(feature = "github-updates"))]
const CUSTOM_API_UPDATE_SOURCES: &[UpdateSource] = &[UpdateSource::CustomApi, UpdateSource::GitHub];

/// 更新源类型
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UpdateSource {
    /// 通过 GitHub Releases 检查更新
    #[cfg_attr(not(feature = "github-updates"), allow(dead_code))]
    GitHub,
    /// 通过 R2 或自定义 JSON API 检查更新
    #[allow(dead_code)]
    CustomApi,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UpdateCheckTrigger {
    Automatic,
    Manual,
}

enum UpdateCheckOutcome {
    ShowDialog(UpdateDialogInfo),
    NotifyNoUpdate,
    NotifyFailure(String),
    Silent,
}

#[derive(Clone, Debug)]
pub(crate) struct UpdateDialogInfo {
    current_version: String,
    latest_version: String,
    download_url: Option<String>,
    fallback_download_url: Option<String>,
    expected_sha256: Option<String>,
}

impl UpdateDialogInfo {
    fn download_urls(&self) -> Vec<String> {
        let mut urls = Vec::new();
        push_unique_url(&mut urls, self.download_url.clone());
        push_unique_url(&mut urls, self.fallback_download_url.clone());
        urls
    }
}

fn push_unique_url(urls: &mut Vec<String>, url: Option<String>) {
    let Some(url) = url.filter(|url| !url.trim().is_empty()) else {
        return;
    };
    if !urls.contains(&url) {
        urls.push(url);
    }
}

pub fn handle_update_command() -> bool {
    let mut args = std::env::args().skip(1);
    let Some(flag) = args.next() else {
        cleanup_stale_update_backups();
        return false;
    };

    if flag != APPLY_UPDATE_FLAG {
        cleanup_stale_update_backups();
        return false;
    }

    let Some(download_path) = args.next().map(PathBuf::from) else {
        eprintln!("缺少更新包路径");
        return true;
    };

    let target_path = args
        .next()
        .map(PathBuf::from)
        .or_else(|| std::env::current_exe().ok())
        .unwrap_or_else(|| download_path.clone());

    if let Err(err) = apply_update_helper(&download_path, &target_path) {
        eprintln!("更新失败: {}", err);
    }

    true
}

pub fn schedule_update_check(window: &mut Window, cx: &mut App) {
    if !should_run_update_check(
        UpdateCheckTrigger::Automatic,
        AppSettings::global(cx).auto_update,
    ) {
        return;
    }

    run_update_check(window, cx, UpdateCheckTrigger::Automatic);
}

pub fn check_for_updates_manually(window: &mut Window, cx: &mut App) {
    run_update_check(window, cx, UpdateCheckTrigger::Manual);
}

fn run_update_check(window: &mut Window, cx: &mut App, trigger: UpdateCheckTrigger) {
    let config = UpdateConfig::get();
    let http_client = cx.http_client();
    let current_version = CURRENT_VERSION.to_string();
    let update_task = Tokio::spawn(cx, async move {
        perform_update_check(config, http_client, current_version, trigger).await
    });

    window
        .spawn(cx, async move |cx| {
            let outcome = match update_task.await {
                Ok(outcome) => outcome,
                Err(err) => {
                    let message = format!("更新检查任务执行失败: {}", err);
                    tracing::warn!("{}", message);
                    notify_failure_if_needed(trigger, message, cx);
                    return;
                }
            };

            match outcome {
                UpdateCheckOutcome::ShowDialog(info) => {
                    show_update_dialog_on_active_window(info, cx)
                }
                UpdateCheckOutcome::NotifyNoUpdate => notify_no_update_if_needed(trigger, cx),
                UpdateCheckOutcome::NotifyFailure(err) => {
                    notify_failure_if_needed(trigger, err, cx)
                }
                UpdateCheckOutcome::Silent => {}
            }
        })
        .detach();
}

async fn perform_update_check(
    config: UpdateConfig,
    http_client: std::sync::Arc<dyn gpui::http_client::HttpClient>,
    current_version: String,
    trigger: UpdateCheckTrigger,
) -> UpdateCheckOutcome {
    let mut last_error = None;
    let mut saw_no_update = false;

    for source in active_update_sources(&config) {
        match fetch_dialog_info_from_source(*source, &config, http_client.clone(), &current_version)
            .await
        {
            Ok(Some(info)) => return UpdateCheckOutcome::ShowDialog(info),
            Ok(None) => saw_no_update = true,
            Err(err) => {
                tracing::warn!("更新源 {:?} 检查失败: {}", source, err);
                last_error = Some(err);
            }
        }
    }

    if saw_no_update {
        return no_update_outcome(trigger);
    }

    let err = last_error.unwrap_or_else(|| "没有可用的更新源".to_string());
    failure_outcome(trigger, err)
}

#[cfg(feature = "github-updates")]
fn active_update_sources(_config: &UpdateConfig) -> &'static [UpdateSource] {
    GITHUB_UPDATE_SOURCES
}

#[cfg(not(feature = "github-updates"))]
fn active_update_sources(config: &UpdateConfig) -> &'static [UpdateSource] {
    if config.is_valid() {
        CUSTOM_API_UPDATE_SOURCES
    } else {
        GITHUB_UPDATE_SOURCES
    }
}

async fn fetch_dialog_info_from_source(
    source: UpdateSource,
    config: &UpdateConfig,
    http_client: std::sync::Arc<dyn gpui::http_client::HttpClient>,
    current_version: &str,
) -> Result<Option<UpdateDialogInfo>, String> {
    // 网络连通性检查：直接探测对应更新源，而非第三方地址
    let connectivity_url = match source {
        UpdateSource::GitHub => network::GITHUB_API_HOST,
        UpdateSource::CustomApi => config.update_url.as_str(),
    };
    if let Err(err) = check_network_connectivity(http_client.clone(), connectivity_url).await {
        tracing::warn!("{}: {}", t!("Update.network_check_failed"), err);
        return Err(err);
    }

    match source {
        UpdateSource::GitHub => fetch_github_dialog_info(http_client, current_version).await,
        UpdateSource::CustomApi => {
            fetch_custom_dialog_info(config, http_client, current_version).await
        }
    }
}

fn should_run_update_check(trigger: UpdateCheckTrigger, auto_update_enabled: bool) -> bool {
    matches!(trigger, UpdateCheckTrigger::Manual) || auto_update_enabled
}

fn notify_no_update_if_needed(trigger: UpdateCheckTrigger, cx: &mut gpui::AsyncApp) {
    if trigger == UpdateCheckTrigger::Manual {
        push_notification_on_active_window(t!("Update.already_up_to_date").to_string(), cx);
    }
}

fn notify_failure_if_needed(trigger: UpdateCheckTrigger, err: String, cx: &mut gpui::AsyncApp) {
    if trigger == UpdateCheckTrigger::Manual {
        push_notification_on_active_window(format!("{}: {}", t!("Update.check_failed"), err), cx);
    }
}

fn no_update_outcome(trigger: UpdateCheckTrigger) -> UpdateCheckOutcome {
    if trigger == UpdateCheckTrigger::Manual {
        UpdateCheckOutcome::NotifyNoUpdate
    } else {
        UpdateCheckOutcome::Silent
    }
}

fn failure_outcome(trigger: UpdateCheckTrigger, err: String) -> UpdateCheckOutcome {
    if trigger == UpdateCheckTrigger::Manual {
        UpdateCheckOutcome::NotifyFailure(err)
    } else {
        UpdateCheckOutcome::Silent
    }
}

async fn fetch_github_dialog_info(
    http_client: std::sync::Arc<dyn gpui::http_client::HttpClient>,
    current_version: &str,
) -> Result<Option<UpdateDialogInfo>, String> {
    let release = fetch_github_release(http_client).await?;
    let latest_version = parse_version(&release.tag_name)
        .ok_or_else(|| format!("版本号无法解析 {}", release.tag_name))?;
    let current_semver = parse_version(current_version)
        .ok_or_else(|| format!("当前版本号无法解析 {}", current_version))?;

    if latest_version <= current_semver {
        return Ok(None);
    }

    github_release_to_dialog_info(&release, current_version)
        .map(Some)
        .map_err(|err| format!("转换 GitHub Release 失败: {}", err))
}

async fn fetch_custom_dialog_info(
    config: &UpdateConfig,
    http_client: std::sync::Arc<dyn gpui::http_client::HttpClient>,
    current_version: &str,
) -> Result<Option<UpdateDialogInfo>, String> {
    if !config.is_valid() {
        return Err("缺少更新接口地址，无法检查更新".to_string());
    }

    let response = fetch_update_info(http_client, &config.update_url).await?;
    let latest_version = parse_version(&response.version)
        .ok_or_else(|| format!("版本号无法解析 {}", response.version))?;
    let current_semver = parse_version(current_version)
        .ok_or_else(|| format!("当前版本号无法解析 {}", current_version))?;

    if latest_version <= current_semver {
        return Ok(None);
    }

    Ok(Some(UpdateDialogInfo {
        current_version: current_version.to_string(),
        latest_version: response.version.clone(),
        download_url: select_download_url(&response, config.download_url.clone()),
        fallback_download_url: select_fallback_download_url(&response),
        expected_sha256: select_sha256(&response),
    }))
}

fn show_update_dialog_on_active_window(info: UpdateDialogInfo, cx: &mut gpui::AsyncApp) {
    let _ = cx.update(|cx| {
        show_update_dialog(info.clone(), cx);
    });
}

fn push_notification_on_active_window(message: String, cx: &mut gpui::AsyncApp) {
    let _ = cx.update(|cx| {
        if let Some(window_id) = cx.active_window() {
            let _ = cx.update_window(window_id, |_, window, cx| {
                window.push_notification(message.clone(), cx);
            });
        }
    });
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::collections::VecDeque;
    use std::sync::Mutex;

    use anyhow::{Result, anyhow};
    use futures::FutureExt;
    use gpui::http_client::{self, AsyncBody, HttpClient, Url, http};

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub(crate) struct CapturedRequest {
        pub method: http::Method,
        pub uri: String,
        pub user_agent: Option<String>,
    }

    pub(crate) struct FakeHttpClient {
        responses: Mutex<VecDeque<Result<http_client::Response<AsyncBody>>>>,
        requests: Mutex<Vec<CapturedRequest>>,
    }

    impl FakeHttpClient {
        pub(crate) fn new(responses: Vec<Result<http_client::Response<AsyncBody>>>) -> Self {
            Self {
                responses: Mutex::new(VecDeque::from(responses)),
                requests: Mutex::new(Vec::new()),
            }
        }

        pub(crate) fn take_requests(&self) -> Vec<CapturedRequest> {
            self.requests.lock().expect("requests 锁失败").clone()
        }

        pub(crate) fn response(
            status: u16,
            body: &str,
        ) -> Result<http_client::Response<AsyncBody>> {
            http::Response::builder()
                .status(status)
                .body(AsyncBody::from(body.as_bytes().to_vec()))
                .map_err(|err| anyhow!("构建响应失败: {}", err))
        }
    }

    impl HttpClient for FakeHttpClient {
        fn user_agent(&self) -> Option<&http::HeaderValue> {
            None
        }

        fn proxy(&self) -> Option<&Url> {
            None
        }

        fn send(
            &self,
            req: http::Request<AsyncBody>,
        ) -> futures::future::BoxFuture<'static, Result<http_client::Response<AsyncBody>>> {
            let captured = CapturedRequest {
                method: req.method().clone(),
                uri: req.uri().to_string(),
                user_agent: req
                    .headers()
                    .get(http::header::USER_AGENT)
                    .and_then(|value| value.to_str().ok())
                    .map(ToOwned::to_owned),
            };
            self.requests
                .lock()
                .expect("requests 锁失败")
                .push(captured);

            let result = self
                .responses
                .lock()
                .expect("responses 锁失败")
                .pop_front()
                .unwrap_or_else(|| Err(anyhow!("缺少 fake response")));

            async move { result }.boxed()
        }
    }
}

#[cfg(test)]
mod tests {
    #[cfg(not(feature = "github-updates"))]
    use std::sync::Arc;

    #[cfg(not(feature = "github-updates"))]
    use anyhow::anyhow;
    #[cfg(not(feature = "github-updates"))]
    use gpui::http_client::HttpClient;
    #[cfg(not(feature = "github-updates"))]
    use one_core::config::update_url_from_public_base;

    use super::{
        ACTIVE_UPDATE_SOURCE, UpdateCheckTrigger, UpdateDialogInfo, UpdateSource,
        should_run_update_check,
    };
    #[cfg(not(feature = "github-updates"))]
    use super::{UpdateCheckOutcome, perform_update_check};
    #[cfg(not(feature = "github-updates"))]
    use crate::update::test_support::FakeHttpClient;

    #[test]
    fn manual_check_bypasses_auto_update_switch() {
        assert!(should_run_update_check(UpdateCheckTrigger::Manual, false));
    }

    #[test]
    fn automatic_check_still_respects_auto_update_switch() {
        assert!(!should_run_update_check(
            UpdateCheckTrigger::Automatic,
            false
        ));
        assert!(should_run_update_check(UpdateCheckTrigger::Automatic, true));
    }

    #[cfg(not(feature = "github-updates"))]
    #[test]
    fn default_update_source_uses_r2_custom_api() {
        assert_eq!(UpdateSource::CustomApi, ACTIVE_UPDATE_SOURCE);
    }

    #[cfg(feature = "github-updates")]
    #[test]
    fn github_updates_feature_uses_github_source() {
        assert_eq!(UpdateSource::GitHub, ACTIVE_UPDATE_SOURCE);
    }

    #[test]
    fn update_dialog_info_download_urls_keep_primary_then_fallback() {
        let info = UpdateDialogInfo {
            current_version: "0.1.0".to_string(),
            latest_version: "9.9.9".to_string(),
            download_url: Some("https://onetcli.pdyyds.cn/update.tar.gz".to_string()),
            fallback_download_url: Some("https://github.example.test/update.tar.gz".to_string()),
            expected_sha256: None,
        };

        assert_eq!(
            vec![
                "https://onetcli.pdyyds.cn/update.tar.gz".to_string(),
                "https://github.example.test/update.tar.gz".to_string(),
            ],
            info.download_urls()
        );
    }

    #[test]
    fn update_dialog_info_download_urls_deduplicate_repeated_fallback() {
        let info = UpdateDialogInfo {
            current_version: "0.1.0".to_string(),
            latest_version: "9.9.9".to_string(),
            download_url: Some("https://github.example.test/update.tar.gz".to_string()),
            fallback_download_url: Some("https://github.example.test/update.tar.gz".to_string()),
            expected_sha256: None,
        };

        assert_eq!(
            vec!["https://github.example.test/update.tar.gz".to_string()],
            info.download_urls()
        );
    }

    #[cfg(not(feature = "github-updates"))]
    #[tokio::test]
    async fn cloudflare_connectivity_failure_falls_back_to_github_release() {
        let update_url = update_url_from_public_base("https://onetcli.pdyyds.cn");
        let client = Arc::new(FakeHttpClient::new(vec![
            Err(anyhow!("r2 unavailable")),
            FakeHttpClient::response(200, ""),
            FakeHttpClient::response(200, &github_release_body("v9.9.9")),
        ]));
        let http_client: Arc<dyn HttpClient> = client.clone();
        let config = one_core::config::UpdateConfig {
            update_url: update_url.clone(),
            download_url: None,
        };

        let outcome = perform_update_check(
            config,
            http_client,
            "0.1.0".to_string(),
            UpdateCheckTrigger::Manual,
        )
        .await;

        let UpdateCheckOutcome::ShowDialog(info) = outcome else {
            panic!("R2 失败后应回退到 GitHub Release");
        };
        assert_eq!(info.latest_version, "v9.9.9");
        assert_eq!(
            info.download_url.as_deref(),
            Some("https://github.example.test/update")
        );

        let requests = client.take_requests();
        assert_eq!(requests[0].uri, update_url);
        assert_eq!(requests[1].uri, "https://api.github.com/");
        assert_eq!(
            requests[2].uri,
            "https://api.github.com/repos/feigeCode/onetcli/releases/latest"
        );
    }

    #[cfg(not(feature = "github-updates"))]
    #[tokio::test]
    async fn missing_cloudflare_update_url_uses_github_directly() {
        let client = Arc::new(FakeHttpClient::new(vec![
            FakeHttpClient::response(200, ""),
            FakeHttpClient::response(200, &github_release_body("v9.9.9")),
        ]));
        let http_client: Arc<dyn HttpClient> = client.clone();
        let config = one_core::config::UpdateConfig {
            update_url: String::new(),
            download_url: None,
        };

        let outcome = perform_update_check(
            config,
            http_client,
            "0.1.0".to_string(),
            UpdateCheckTrigger::Manual,
        )
        .await;

        assert!(matches!(outcome, UpdateCheckOutcome::ShowDialog(_)));
        let requests = client.take_requests();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].uri, "https://api.github.com/");
        assert_eq!(
            requests[1].uri,
            "https://api.github.com/repos/feigeCode/onetcli/releases/latest"
        );
    }

    #[cfg(not(feature = "github-updates"))]
    #[tokio::test]
    async fn stale_cloudflare_manifest_falls_back_to_github_release() {
        let update_url = update_url_from_public_base("https://onetcli.pdyyds.cn");
        let client = Arc::new(FakeHttpClient::new(vec![
            FakeHttpClient::response(200, ""),
            FakeHttpClient::response(200, r#"{"version":"0.1.0"}"#),
            FakeHttpClient::response(200, ""),
            FakeHttpClient::response(200, &github_release_body("v9.9.9")),
        ]));
        let http_client: Arc<dyn HttpClient> = client.clone();
        let config = one_core::config::UpdateConfig {
            update_url: update_url.clone(),
            download_url: None,
        };

        let outcome = perform_update_check(
            config,
            http_client,
            "0.1.0".to_string(),
            UpdateCheckTrigger::Manual,
        )
        .await;

        let UpdateCheckOutcome::ShowDialog(info) = outcome else {
            panic!("R2 manifest 未更新时应继续回退到 GitHub Release");
        };
        assert_eq!(info.latest_version, "v9.9.9");

        let requests = client.take_requests();
        assert_eq!(requests[0].uri, update_url);
        assert_eq!(requests[1].uri, update_url);
        assert_eq!(requests[2].uri, "https://api.github.com/");
        assert_eq!(
            requests[3].uri,
            "https://api.github.com/repos/feigeCode/onetcli/releases/latest"
        );
    }

    #[cfg(not(feature = "github-updates"))]
    #[tokio::test]
    async fn cloudflare_manifest_can_provide_github_download_fallback() {
        let update_url = update_url_from_public_base("https://onetcli.pdyyds.cn");
        let client = Arc::new(FakeHttpClient::new(vec![
            FakeHttpClient::response(200, ""),
            FakeHttpClient::response(200, &custom_update_body_with_fallback("9.9.9")),
        ]));
        let http_client: Arc<dyn HttpClient> = client.clone();
        let config = one_core::config::UpdateConfig {
            update_url,
            download_url: None,
        };

        let outcome = perform_update_check(
            config,
            http_client,
            "0.1.0".to_string(),
            UpdateCheckTrigger::Manual,
        )
        .await;

        let UpdateCheckOutcome::ShowDialog(info) = outcome else {
            panic!("R2 manifest 有新版本时应展示更新对话框");
        };
        assert_eq!(
            info.download_url.as_deref(),
            Some(
                format!(
                    "https://onetcli.pdyyds.cn/releases/v9.9.9/{}",
                    expected_archive_name()
                )
                .as_str()
            )
        );
        assert_eq!(
            info.fallback_download_url.as_deref(),
            Some(
                format!(
                    "https://github.com/feigeCode/onetcli/releases/download/v9.9.9/{}",
                    expected_archive_name()
                )
                .as_str()
            )
        );
    }

    #[cfg(not(feature = "github-updates"))]
    fn github_release_body(version: &str) -> String {
        format!(
            r#"{{
                "tag_name": "{version}",
                "assets": [{{
                    "name": "{}",
                    "browser_download_url": "https://github.example.test/update"
                }}]
            }}"#,
            expected_archive_name()
        )
    }

    #[cfg(not(feature = "github-updates"))]
    fn custom_update_body_with_fallback(version: &str) -> String {
        format!(
            r#"{{
                "version": "{version}",
                "downloads": {{
                    "{}": "https://onetcli.pdyyds.cn/releases/v{version}/{}"
                }},
                "fallback_downloads": {{
                    "{}": "https://github.com/feigeCode/onetcli/releases/download/v{version}/{}"
                }}
            }}"#,
            current_target_key(),
            expected_archive_name(),
            current_target_key(),
            expected_archive_name()
        )
    }

    #[cfg(not(feature = "github-updates"))]
    fn current_target_key() -> &'static str {
        super::custom_api::platform_download_keys_for(std::env::consts::OS, std::env::consts::ARCH)
            .first()
            .copied()
            .unwrap_or("unsupported-target")
    }

    #[cfg(not(feature = "github-updates"))]
    fn expected_archive_name() -> &'static str {
        let archive = super::github_release::expected_archive_name_for(
            std::env::consts::OS,
            std::env::consts::ARCH,
        );
        if archive.is_empty() {
            "unsupported-target"
        } else {
            archive
        }
    }
}
