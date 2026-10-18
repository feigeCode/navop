use std::path::PathBuf;
use std::time::Duration;

use gpui::{App, AppContext};
use gpui_component::WindowExt;
use one_core::gpui_tokio::Tokio;
use rust_i18n::t;

use crate::setting_tab::AppSettings;
use one_core::config::UpdateConfig;

mod check_state;
mod custom_api;
mod dialog;
mod download;
mod extract;
mod github_release;
mod install;
mod local_source;
mod network;
mod util;

use check_state::{CheckReport, begin_check, finish_check};
pub(crate) use check_state::{UpdateCheckState, update_check_state};
use custom_api::{
    fetch_update_info, select_download_url, select_fallback_download_url, select_sha256,
};
use dialog::show_update_dialog;
use github_release::{fetch_github_release, github_release_to_dialog_info};
use install::{apply_update_helper, cleanup_stale_update_backups};
use local_source::{is_local_file_source, resolve_local_reference};
use network::check_network_connectivity;
use util::parse_version;

const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
const APPLY_UPDATE_FLAG: &str = "--apply-update";

/// 一次更新检查的总时长上限。多个更新源是串行检查的，连通性探测与拉取又都没有各自的
/// 超时，代理或弱网下会长时间不返回，因此整体限时并给出明确提示。
const UPDATE_CHECK_TIMEOUT: Duration = Duration::from_secs(20);

/// 当前使用的更新源。默认走 R2 JSON；启用 `github-updates` feature 时切回 GitHub Releases。
#[cfg(test)]
#[cfg(feature = "github-updates")]
const ACTIVE_UPDATE_SOURCE: UpdateSource = UpdateSource::GitHub;
#[cfg(test)]
#[cfg(not(feature = "github-updates"))]
const ACTIVE_UPDATE_SOURCE: UpdateSource = UpdateSource::CustomApi;
#[cfg(feature = "github-updates")]
const GITHUB_UPDATE_SOURCES: &[UpdateSource] = &[UpdateSource::GitHub];
const LOCAL_UPDATE_SOURCES: &[UpdateSource] = &[UpdateSource::CustomApi];
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

/// 检查结果本身与呈现方式分开：这里只描述「查到了什么」，是否通知用户由 trigger 决定。
enum UpdateCheckOutcome {
    ShowDialog(UpdateDialogInfo),
    NoUpdate,
    Failure(String),
    TimedOut,
}

#[derive(Clone, Debug)]
pub(crate) struct UpdateDialogInfo {
    current_version: String,
    latest_version: String,
    release_notes: Option<String>,
    download_url: Option<String>,
    fallback_download_url: Option<String>,
    expected_sha256: Option<String>,
    is_local_simulation: bool,
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

pub fn schedule_update_check(cx: &mut App) {
    if !should_run_update_check(
        UpdateCheckTrigger::Automatic,
        AppSettings::global(cx).auto_update,
        one_core::app_paths::is_portable(),
    ) {
        return;
    }

    run_update_check(cx, UpdateCheckTrigger::Automatic);
}

pub fn check_for_updates_manually(cx: &mut App) {
    run_update_check(cx, UpdateCheckTrigger::Manual);
}

fn run_update_check(cx: &mut App, trigger: UpdateCheckTrigger) {
    // 自动与手动检查共用同一份状态：启动时的自动检查可能还在跑，此时按钮已经是加载态，
    // 这里直接拒绝，避免同一时刻发两轮网络请求。
    if !begin_check(cx) {
        return;
    }

    let config = UpdateConfig::get();
    let http_client = cx.http_client();
    let current_version = CURRENT_VERSION.to_string();
    let skipped_version = AppSettings::global(cx).skipped_update_version.clone();
    let update_task = Tokio::spawn(cx, async move {
        match tokio::time::timeout(
            UPDATE_CHECK_TIMEOUT,
            perform_update_check(config, http_client, current_version),
        )
        .await
        {
            Ok(outcome) => outcome,
            Err(_elapsed) => UpdateCheckOutcome::TimedOut,
        }
    });

    // 收尾挂在 app 而不是某个窗口上：窗口在检查期间被关闭时，窗口级 future 会被丢弃，
    // 「检查中」就永远落不回来，按钮会一直转圈并拒绝再次检查。
    cx.spawn(async move |cx| {
        let outcome = match update_task.await {
            Ok(outcome) => outcome,
            Err(err) => {
                let message = format!("更新检查任务执行失败: {}", err);
                tracing::warn!("{}", message);
                finish_check(
                    cx,
                    CheckReport::Failed {
                        reason: message.clone(),
                    },
                );
                notify_failure_if_needed(trigger, message, cx);
                return;
            }
        };

        match outcome {
            UpdateCheckOutcome::ShowDialog(info) => {
                let latest_version = info.latest_version.clone();
                finish_check(
                    cx,
                    CheckReport::Available {
                        version: latest_version,
                    },
                );
                if should_show_available_update(
                    trigger,
                    &info.latest_version,
                    skipped_version.as_deref(),
                ) {
                    show_update_dialog_on_active_window(info, cx);
                }
            }
            UpdateCheckOutcome::NoUpdate => {
                finish_check(cx, CheckReport::UpToDate);
                notify_no_update_if_needed(trigger, cx);
            }
            UpdateCheckOutcome::Failure(err) => {
                finish_check(
                    cx,
                    CheckReport::Failed {
                        reason: err.clone(),
                    },
                );
                notify_failure_if_needed(trigger, err, cx);
            }
            UpdateCheckOutcome::TimedOut => {
                finish_check(cx, CheckReport::TimedOut);
                notify_timeout_if_needed(trigger, cx);
            }
        }
    })
    .detach();
}

async fn perform_update_check(
    config: UpdateConfig,
    http_client: std::sync::Arc<dyn gpui::http_client::HttpClient>,
    current_version: String,
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
        return UpdateCheckOutcome::NoUpdate;
    }

    let err = last_error.unwrap_or_else(|| "没有可用的更新源".to_string());
    UpdateCheckOutcome::Failure(err)
}

#[cfg(feature = "github-updates")]
fn active_update_sources(config: &UpdateConfig) -> &'static [UpdateSource] {
    if is_local_file_source(&config.update_url) {
        LOCAL_UPDATE_SOURCES
    } else {
        GITHUB_UPDATE_SOURCES
    }
}

#[cfg(not(feature = "github-updates"))]
fn active_update_sources(config: &UpdateConfig) -> &'static [UpdateSource] {
    if is_local_file_source(&config.update_url) {
        LOCAL_UPDATE_SOURCES
    } else if config.is_valid() {
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
    if source != UpdateSource::CustomApi || !is_local_file_source(&config.update_url) {
        // 网络连通性检查：直接探测对应更新源，而非第三方地址
        let connectivity_url = match source {
            UpdateSource::GitHub => network::GITHUB_API_HOST,
            UpdateSource::CustomApi => config.update_url.as_str(),
        };
        if let Err(err) = check_network_connectivity(http_client.clone(), connectivity_url).await {
            tracing::warn!("{}: {}", t!("Update.network_check_failed"), err);
            return Err(err);
        }
    }

    match source {
        UpdateSource::GitHub => fetch_github_dialog_info(http_client, current_version).await,
        UpdateSource::CustomApi => {
            fetch_custom_dialog_info(config, http_client, current_version).await
        }
    }
}

fn should_run_update_check(
    trigger: UpdateCheckTrigger,
    auto_update_enabled: bool,
    portable: bool,
) -> bool {
    matches!(trigger, UpdateCheckTrigger::Manual) || auto_update_enabled && !portable
}

fn should_show_available_update(
    trigger: UpdateCheckTrigger,
    latest_version: &str,
    skipped_version: Option<&str>,
) -> bool {
    if trigger == UpdateCheckTrigger::Manual {
        return true;
    }

    let Some(skipped_version) = skipped_version else {
        return true;
    };

    match (
        parse_version(latest_version),
        parse_version(skipped_version),
    ) {
        (Some(latest), Some(skipped)) => latest != skipped,
        _ => latest_version.trim() != skipped_version.trim(),
    }
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

fn notify_timeout_if_needed(trigger: UpdateCheckTrigger, cx: &mut gpui::AsyncApp) {
    if trigger == UpdateCheckTrigger::Manual {
        push_notification_on_active_window(t!("Update.check_timeout").to_string(), cx);
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

    let download_url = select_download_url(&response, config.download_url.clone())
        .map(|url| resolve_local_reference(&config.update_url, &url));
    let fallback_download_url = select_fallback_download_url(&response)
        .map(|url| resolve_local_reference(&config.update_url, &url));

    Ok(Some(UpdateDialogInfo {
        current_version: current_version.to_string(),
        latest_version: response.version.clone(),
        release_notes: response.release_notes.clone(),
        download_url,
        fallback_download_url,
        expected_sha256: select_sha256(&response),
        is_local_simulation: is_local_file_source(&config.update_url),
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
    use std::sync::Arc;
    use std::time::Duration;

    #[cfg(not(feature = "github-updates"))]
    use anyhow::anyhow;
    use gpui::http_client::HttpClient;
    #[cfg(not(feature = "github-updates"))]
    use one_core::config::update_url_from_public_base;

    use super::{
        ACTIVE_UPDATE_SOURCE, UPDATE_CHECK_TIMEOUT, UpdateCheckTrigger, UpdateDialogInfo,
        UpdateSource, should_run_update_check, should_show_available_update,
    };
    use super::{UpdateCheckOutcome, perform_update_check};
    use crate::update::test_support::FakeHttpClient;

    #[test]
    fn update_check_is_bounded_and_goes_through_the_shared_state() {
        assert_eq!(Duration::from_secs(20), UPDATE_CHECK_TIMEOUT);

        let source = include_str!("mod.rs");
        let run_check = source
            .split("fn run_update_check(")
            .nth(1)
            .expect("run_update_check 存在")
            .split("async fn perform_update_check(")
            .next()
            .expect("run_update_check 有结尾");

        // 同一时刻只允许一轮检查，按钮的加载态才不会被并发请求打乱。
        assert!(
            run_check.contains("if !begin_check(cx)"),
            "检查必须先进入共享的「检查中」状态"
        );
        assert!(
            run_check.contains("tokio::time::timeout("),
            "整轮检查要有总时长上限"
        );
        // 结果先落到共享状态，再决定弹窗或通知，设置页才能就地展示。
        assert!(run_check.contains("finish_check(cx, CheckReport::UpToDate)"));
        assert!(run_check.contains("notify_timeout_if_needed(trigger, cx)"));
    }

    #[test]
    fn manual_check_bypasses_auto_update_switch() {
        assert!(should_run_update_check(
            UpdateCheckTrigger::Manual,
            false,
            true
        ));
    }

    #[test]
    fn automatic_check_still_respects_auto_update_switch() {
        assert!(!should_run_update_check(
            UpdateCheckTrigger::Automatic,
            false,
            false
        ));
        assert!(should_run_update_check(
            UpdateCheckTrigger::Automatic,
            true,
            false
        ));
    }

    #[test]
    fn portable_mode_skips_automatic_update_checks() {
        assert!(!should_run_update_check(
            UpdateCheckTrigger::Automatic,
            true,
            true
        ));
    }

    #[test]
    fn automatic_check_hides_the_skipped_version() {
        assert!(!should_show_available_update(
            UpdateCheckTrigger::Automatic,
            "v1.2.3",
            Some("1.2.3")
        ));
    }

    #[test]
    fn automatic_check_shows_a_newer_version_than_the_skipped_one() {
        assert!(should_show_available_update(
            UpdateCheckTrigger::Automatic,
            "1.2.4",
            Some("1.2.3")
        ));
    }

    #[test]
    fn manual_check_can_reopen_the_skipped_version() {
        assert!(should_show_available_update(
            UpdateCheckTrigger::Manual,
            "1.2.3",
            Some("v1.2.3")
        ));
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
            release_notes: None,
            download_url: Some("https://onetcli.pdyyds.cn/update.tar.gz".to_string()),
            fallback_download_url: Some("https://github.example.test/update.tar.gz".to_string()),
            expected_sha256: None,
            is_local_simulation: false,
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
            release_notes: None,
            download_url: Some("https://github.example.test/update.tar.gz".to_string()),
            fallback_download_url: Some("https://github.example.test/update.tar.gz".to_string()),
            expected_sha256: None,
            is_local_simulation: false,
        };

        assert_eq!(
            vec!["https://github.example.test/update.tar.gz".to_string()],
            info.download_urls()
        );
    }

    #[tokio::test]
    async fn local_manifest_skips_network_check_and_enables_safe_simulation() {
        let temp_dir = tempfile::TempDir::new().expect("创建临时目录失败");
        let package_path = temp_dir.path().join("navop-local-update.tar.gz");
        let manifest_path = temp_dir.path().join("latest.json");
        std::fs::write(&package_path, b"local-package").expect("写入本地更新包失败");
        std::fs::write(
            &manifest_path,
            r###"{
                "version": "99.0.0",
                "release_notes": "## 本地更新模拟",
                "download_url": "navop-local-update.tar.gz"
            }"###,
        )
        .expect("写入本地 manifest 失败");
        let client = Arc::new(FakeHttpClient::new(Vec::new()));
        let http_client: Arc<dyn HttpClient> = client.clone();
        let config = one_core::config::UpdateConfig {
            update_url: manifest_path.to_string_lossy().into_owned(),
            download_url: None,
        };

        let outcome = perform_update_check(config, http_client, "0.10.0".to_string()).await;

        let UpdateCheckOutcome::ShowDialog(info) = outcome else {
            panic!("本地 manifest 应展示更新模拟窗口");
        };
        assert_eq!(info.latest_version, "99.0.0");
        assert_eq!(info.release_notes.as_deref(), Some("## 本地更新模拟"));
        assert_eq!(
            info.download_url.as_deref(),
            Some(package_path.to_string_lossy().as_ref())
        );
        assert!(info.is_local_simulation);
        assert!(client.take_requests().is_empty());
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

        let outcome = perform_update_check(config, http_client, "0.1.0".to_string()).await;

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
            "https://api.github.com/repos/feigeCode/navop/releases/latest"
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

        let outcome = perform_update_check(config, http_client, "0.1.0".to_string()).await;

        assert!(matches!(outcome, UpdateCheckOutcome::ShowDialog(_)));
        let requests = client.take_requests();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].uri, "https://api.github.com/");
        assert_eq!(
            requests[1].uri,
            "https://api.github.com/repos/feigeCode/navop/releases/latest"
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

        let outcome = perform_update_check(config, http_client, "0.1.0".to_string()).await;

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
            "https://api.github.com/repos/feigeCode/navop/releases/latest"
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

        let outcome = perform_update_check(config, http_client, "0.1.0".to_string()).await;

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
                    "https://github.com/feigeCode/navop/releases/download/v9.9.9/{}",
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
                    "{}": "https://github.com/feigeCode/navop/releases/download/v{version}/{}"
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
