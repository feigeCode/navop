use std::path::PathBuf;
use std::time::Duration;

use gpui::{App, AppContext};
use gpui_component::WindowExt;
use one_core::config::UpdateConfig;
use one_core::gpui_tokio::Tokio;
use one_core::settings::{AppSettings, UpdateChannel, UpdateMirror};
use rust_i18n::t;

mod check_state;
mod config;
mod custom_api;
mod dialog;
mod download;
mod extract;
mod github_release;
mod host;
mod install;
mod local_source;
mod mirror;
mod network;
mod util;

pub use crate::file_association::registered_extensions;
pub use config::{
    current_version, derive_update_config, effective_channel, set_current_version,
    update_check_available, update_config,
};
pub use host::{icon_asset_path, set_icon_asset_path, set_shutdown_hook};

use check_state::{CheckReport, begin_check, finish_check};
pub use check_state::{UpdateCheckState, update_check_state};
use custom_api::{
    fetch_update_info, select_download_url, select_fallback_download_url, select_sha256,
};
use dialog::show_update_dialog;
use github_release::{fetch_github_release, github_release_to_dialog_info};
use install::{apply_update_helper, cleanup_stale_update_backups};
use local_source::{is_local_file_source, resolve_local_reference};
use network::check_network_connectivity;
use util::parse_version;

const APPLY_UPDATE_FLAG: &str = "--apply-update";

/// 一次更新检查的总时长上限。多个更新源是串行检查的，连通性探测与拉取又都没有各自的
/// 超时，代理或弱网下会长时间不返回，因此整体限时并给出明确提示。
const UPDATE_CHECK_TIMEOUT: Duration = Duration::from_secs(20);

/// 一轮检查里的一次取数尝试。
///
/// 「镜像源」只决定这些尝试的先后顺序，具体走哪种形态由这里区分：R2/本地 fixture 是
/// 一份 JSON 清单，CNB 要先发现 tag 再取 Release 资产，GitHub 走 Releases API。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UpdateAttempt {
    /// 直接读一份 JSON 清单；括号里只用于日志与下载地址改写
    Manifest(UpdateMirror),
    /// CNB 镜像：先读 git refs 拿到 tag，再取该 tag 的 feed 资产
    Cnb,
    /// GitHub Releases API，只在稳定通道使用
    Github,
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
    /// 目标版本是否为预发布（测试通道），弹窗据此显示 Beta 标记与提示。
    is_prerelease: bool,
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
    let channel = effective_channel(AppSettings::global(cx).update_channel);
    // 镜像源只影响「先问谁」，取默认值就是内置顺序（R2 → CNB → GitHub）。
    let mirror = AppSettings::global(cx).update_mirror.unwrap_or_default();
    let config = update_config(channel);
    if !update_check_available(&config) {
        // 没有可用 feed 的产品什么都不做：既不能弹失败提示，也不能 panic。
        tracing::debug!("no update feed configured; skipping update check");
        return;
    }

    // 自动与手动检查共用同一份状态：启动时的自动检查可能还在跑，此时按钮已经是加载态，
    // 这里直接拒绝，避免同一时刻发两轮网络请求。
    if !begin_check(cx) {
        return;
    }

    let http_client = cx.http_client();
    let current_version = current_version().to_string();
    let skipped_version = skipped_version_for_channel(
        channel,
        AppSettings::global(cx).skipped_update_version.as_deref(),
    );
    let update_task = Tokio::spawn(cx, async move {
        match tokio::time::timeout(
            UPDATE_CHECK_TIMEOUT,
            perform_update_check(config, http_client, current_version, channel, mirror),
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
    channel: UpdateChannel,
    mirror: UpdateMirror,
) -> UpdateCheckOutcome {
    let attempts = update_attempts(&config, channel, mirror);
    if attempts.is_empty() {
        return UpdateCheckOutcome::Failure("没有可用的更新源".to_string());
    }

    let mut last_error = None;
    let mut saw_up_to_date = false;
    let mut saw_missing_manifest = false;

    for attempt in attempts {
        match fetch_dialog_info_from_attempt(
            attempt,
            &config,
            http_client.clone(),
            &current_version,
            channel,
        )
        .await
        {
            Ok(AttemptOutcome::Found(info)) => return UpdateCheckOutcome::ShowDialog(info),
            // 「已是最新」不立刻收尾：R2 的清单可能比实际 Release 慢一拍，继续问下一个源
            // 才能把「清单还没更新」和「真的没有新版本」区分开。
            Ok(AttemptOutcome::UpToDate) => saw_up_to_date = true,
            Ok(AttemptOutcome::MissingManifest) => saw_missing_manifest = true,
            Err(err) => {
                tracing::warn!("更新尝试 {:?} 检查失败: {}", attempt, err);
                last_error = Some(err);
            }
        }
    }

    if saw_up_to_date {
        return UpdateCheckOutcome::NoUpdate;
    }
    if let Some(err) = last_error {
        return UpdateCheckOutcome::Failure(err);
    }
    if saw_missing_manifest {
        // 所有源都说「这份清单不存在」：测试通道在首个 beta 发布前就是这种状态，
        // 对稳定通道则确实是配置/发布出了问题。
        return match channel {
            UpdateChannel::Beta => UpdateCheckOutcome::NoUpdate,
            UpdateChannel::Stable => {
                UpdateCheckOutcome::Failure("更新接口返回异常状态码: 404 Not Found".to_string())
            }
        };
    }
    UpdateCheckOutcome::NoUpdate
}

/// 单次取数尝试的结果。
enum AttemptOutcome {
    /// 找到了比当前更新的版本
    Found(UpdateDialogInfo),
    /// 清单有效，但没有更新版本
    UpToDate,
    /// 清单不存在（HTTP 404）
    #[cfg_attr(feature = "github-updates", allow(dead_code))]
    MissingManifest,
}

/// 按镜像源偏好排出本轮要尝试的源。
///
/// 本地 fixture 不参与镜像回退：它只该读本地文件，不该把请求发到网络上。
#[cfg(feature = "github-updates")]
fn update_attempts(
    config: &UpdateConfig,
    _channel: UpdateChannel,
    _mirror: UpdateMirror,
) -> Vec<UpdateAttempt> {
    if is_local_file_source(&config.update_url) {
        vec![UpdateAttempt::Manifest(UpdateMirror::Auto)]
    } else {
        vec![UpdateAttempt::Github]
    }
}

#[cfg(not(feature = "github-updates"))]
fn update_attempts(
    config: &UpdateConfig,
    channel: UpdateChannel,
    mirror: UpdateMirror,
) -> Vec<UpdateAttempt> {
    if is_local_file_source(&config.update_url) {
        return vec![UpdateAttempt::Manifest(UpdateMirror::Auto)];
    }

    let mut attempts = Vec::new();
    for candidate in mirror::mirror_order(mirror) {
        match candidate {
            // 没有配置 feed 的产品（未注入 public base 的开发构建）跳过 R2。
            UpdateMirror::Cloudflare if config.is_valid() => {
                attempts.push(UpdateAttempt::Manifest(UpdateMirror::Cloudflare));
            }
            UpdateMirror::Cnb => attempts.push(UpdateAttempt::Cnb),
            // GitHub 的 `releases/latest` 天然排除预发布，放进测试通道只会把通道悄悄
            // 降级成稳定通道，所以测试通道不做 GitHub 兜底。
            UpdateMirror::Github if channel == UpdateChannel::Stable => {
                attempts.push(UpdateAttempt::Github);
            }
            _ => {}
        }
    }
    attempts
}

async fn fetch_dialog_info_from_attempt(
    attempt: UpdateAttempt,
    config: &UpdateConfig,
    http_client: std::sync::Arc<dyn gpui::http_client::HttpClient>,
    current_version: &str,
    channel: UpdateChannel,
) -> Result<AttemptOutcome, String> {
    match attempt {
        UpdateAttempt::Manifest(mirror) => {
            if !is_local_file_source(&config.update_url) {
                probe_connectivity(http_client.clone(), mirror, &config.update_url).await?;
            }
            fetch_manifest_outcome(config, http_client, current_version, channel).await
        }
        UpdateAttempt::Cnb => {
            let refs_url = mirror::cnb_refs_url();
            probe_connectivity(http_client.clone(), UpdateMirror::Cnb, &refs_url).await?;
            // CNB 既没有 JSON 清单的公开直链，也没有 `latest` 别名：先读 refs 找 tag，
            // 再取该 tag 的 feed 资产，等价于「从清单里拿最新版本 + 下载地址」。
            let tags = mirror::fetch_cnb_tags(http_client.clone()).await?;
            let tag = mirror::latest_tag_for_channel(&tags, channel)
                .ok_or_else(|| "CNB 镜像还没有可用版本".to_string())?;
            let product = mirror::feed_product_slug();
            let feed_url =
                mirror::cnb_release_asset_url(&tag, &mirror::feed_asset_name(product, channel));
            let feed_config = UpdateConfig {
                update_url: feed_url,
                download_url: None,
            };
            match fetch_manifest_outcome(&feed_config, http_client, current_version, channel)
                .await?
            {
                AttemptOutcome::Found(info) => {
                    Ok(AttemptOutcome::Found(rewrite_downloads_for_cnb(info, &tag)))
                }
                other => Ok(other),
            }
        }
        UpdateAttempt::Github => {
            probe_connectivity(
                http_client.clone(),
                UpdateMirror::Github,
                network::GITHUB_API_HOST,
            )
            .await?;
            let info = fetch_github_dialog_info(http_client, current_version, channel).await?;
            Ok(match info {
                Some(info) => AttemptOutcome::Found(info),
                None => AttemptOutcome::UpToDate,
            })
        }
    }
}

/// 先探测目标是否可达，失败就直接跳到下一个源。
async fn probe_connectivity(
    http_client: std::sync::Arc<dyn gpui::http_client::HttpClient>,
    mirror: UpdateMirror,
    target: &str,
) -> Result<(), String> {
    if let Err(err) = check_network_connectivity(http_client, target).await {
        tracing::warn!(
            "{} ({:?}): {}",
            t!("Update.network_check_failed"),
            mirror,
            err
        );
        return Err(err);
    }
    Ok(())
}

/// 把 CNB 清单里的下载地址改到 CNB 的 Release 资产上，改不动就保留原地址。
fn rewrite_downloads_for_cnb(mut info: UpdateDialogInfo, tag: &str) -> UpdateDialogInfo {
    if let Some(url) = info
        .download_url
        .as_deref()
        .and_then(|url| mirror::cnb_download_url(tag, url))
    {
        info.download_url = Some(url);
    }
    if let Some(url) = info
        .fallback_download_url
        .as_deref()
        .and_then(|url| mirror::cnb_download_url(tag, url))
    {
        info.fallback_download_url = Some(url);
    }
    info
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

/// 「跳过此版本」的存储值带上通道前缀（例如 `beta:0.23.0-beta.2`）。
///
/// 两个通道的版本号可能重号（`0.23.0` 与 `0.23.0-beta.2`），不带前缀时在一个通道里
/// 跳过的版本会连带跳过另一个通道的同号版本。
fn encode_skipped_update_version(channel: UpdateChannel, version: &str) -> String {
    format!("{}:{}", channel.as_str(), version.trim())
}

/// 取出属于当前通道的跳过版本；老版本写下的裸版本号只对稳定通道生效。
fn skipped_version_for_channel(channel: UpdateChannel, stored: Option<&str>) -> Option<String> {
    let stored = stored?.trim();
    if stored.is_empty() {
        return None;
    }

    match stored.split_once(':') {
        Some((prefix, version))
            if prefix == UpdateChannel::Stable.as_str()
                || prefix == UpdateChannel::Beta.as_str() =>
        {
            (prefix == channel.as_str()).then(|| version.trim().to_string())
        }
        // 兼容历史数据：没有可识别的通道前缀，只当作稳定通道的跳过记录。
        _ => (channel == UpdateChannel::Stable).then(|| stored.to_string()),
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

/// 该版本是否值得提示。
///
/// 稳定通道只接受正式版本：semver 里 `0.23.0-beta.1 > 0.22.0`，不显式排除预发布就会
/// 把测试版推给只想用稳定版的用户。测试通道接受任何更高的版本（含预发布），但同样不降级。
fn should_offer_update(
    latest: &semver::Version,
    current: &semver::Version,
    channel: UpdateChannel,
) -> bool {
    if latest <= current {
        return false;
    }

    match channel {
        UpdateChannel::Stable => latest.pre.is_empty(),
        UpdateChannel::Beta => true,
    }
}

async fn fetch_github_dialog_info(
    http_client: std::sync::Arc<dyn gpui::http_client::HttpClient>,
    current_version: &str,
    channel: UpdateChannel,
) -> Result<Option<UpdateDialogInfo>, String> {
    let release = fetch_github_release(http_client).await?;
    let latest_version = parse_version(&release.tag_name)
        .ok_or_else(|| format!("版本号无法解析 {}", release.tag_name))?;
    let current_semver = parse_version(current_version)
        .ok_or_else(|| format!("当前版本号无法解析 {}", current_version))?;

    if !should_offer_update(&latest_version, &current_semver, channel) {
        return Ok(None);
    }

    github_release_to_dialog_info(&release, current_version, !latest_version.pre.is_empty())
        .map(Some)
        .map_err(|err| format!("转换 GitHub Release 失败: {}", err))
}

async fn fetch_manifest_outcome(
    config: &UpdateConfig,
    http_client: std::sync::Arc<dyn gpui::http_client::HttpClient>,
    current_version: &str,
    channel: UpdateChannel,
) -> Result<AttemptOutcome, String> {
    if !config.is_valid() {
        return Err("缺少更新接口地址，无法检查更新".to_string());
    }

    let Some(response) = fetch_update_info(http_client, &config.update_url).await? else {
        // 清单不存在只说明「这份 feed 还没有内容」，由调用方结合通道和其余源决定结论：
        // 测试通道在首个 beta 之前就是 404，稳定通道则说明配置/发布出了问题。
        return Ok(AttemptOutcome::MissingManifest);
    };
    let latest_version = parse_version(&response.version)
        .ok_or_else(|| format!("版本号无法解析 {}", response.version))?;
    let current_semver = parse_version(current_version)
        .ok_or_else(|| format!("当前版本号无法解析 {}", current_version))?;

    if !should_offer_update(&latest_version, &current_semver, channel) {
        return Ok(AttemptOutcome::UpToDate);
    }

    let download_url = select_download_url(&response, config.download_url.clone())
        .map(|url| resolve_local_reference(&config.update_url, &url));
    let fallback_download_url = select_fallback_download_url(&response)
        .map(|url| resolve_local_reference(&config.update_url, &url));

    Ok(AttemptOutcome::Found(UpdateDialogInfo {
        current_version: current_version.to_string(),
        latest_version: response.version.clone(),
        release_notes: response.release_notes.clone(),
        download_url,
        fallback_download_url,
        expected_sha256: select_sha256(&response),
        is_local_simulation: is_local_file_source(&config.update_url),
        is_prerelease: !latest_version.pre.is_empty(),
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
    use one_core::settings::{UpdateChannel, UpdateMirror};

    use super::{
        UPDATE_CHECK_TIMEOUT, UpdateAttempt, UpdateCheckTrigger, UpdateDialogInfo,
        encode_skipped_update_version, parse_version, should_offer_update, should_run_update_check,
        should_show_available_update, skipped_version_for_channel, update_attempts,
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
    fn auto_mirror_order_tries_r2_then_cnb_then_github() {
        let config = one_core::config::UpdateConfig {
            update_url: update_url_from_public_base("https://onetcli.pdyyds.cn"),
            download_url: None,
        };

        assert_eq!(
            vec![
                UpdateAttempt::Manifest(UpdateMirror::Cloudflare),
                UpdateAttempt::Cnb,
                UpdateAttempt::Github,
            ],
            update_attempts(&config, UpdateChannel::Stable, UpdateMirror::Auto)
        );
        assert_eq!(
            vec![
                UpdateAttempt::Manifest(UpdateMirror::Cloudflare),
                UpdateAttempt::Cnb,
                UpdateAttempt::Github,
            ],
            update_attempts(&config, UpdateChannel::Stable, UpdateMirror::Cloudflare)
        );
    }

    #[cfg(not(feature = "github-updates"))]
    #[test]
    fn choosing_a_mirror_moves_it_first_without_dropping_the_fallbacks() {
        let config = one_core::config::UpdateConfig {
            update_url: update_url_from_public_base("https://onetcli.pdyyds.cn"),
            download_url: None,
        };

        assert_eq!(
            vec![
                UpdateAttempt::Cnb,
                UpdateAttempt::Manifest(UpdateMirror::Cloudflare),
                UpdateAttempt::Github,
            ],
            update_attempts(&config, UpdateChannel::Stable, UpdateMirror::Cnb)
        );
        assert_eq!(
            vec![
                UpdateAttempt::Github,
                UpdateAttempt::Manifest(UpdateMirror::Cloudflare),
                UpdateAttempt::Cnb,
            ],
            update_attempts(&config, UpdateChannel::Stable, UpdateMirror::Github)
        );
    }

    #[cfg(not(feature = "github-updates"))]
    #[test]
    fn beta_channel_never_falls_back_to_github_releases() {
        let config = one_core::config::UpdateConfig {
            update_url: "https://cdn.example.test/updates/beta/latest.json".to_string(),
            download_url: None,
        };

        // GitHub 的 releases/latest 只给稳定版，beta 回退过去等于把通道降级。
        let attempts = update_attempts(&config, UpdateChannel::Beta, UpdateMirror::Github);
        assert_eq!(
            vec![
                UpdateAttempt::Manifest(UpdateMirror::Cloudflare),
                UpdateAttempt::Cnb
            ],
            attempts
        );
        assert!(!attempts.contains(&UpdateAttempt::Github));
    }

    #[cfg(not(feature = "github-updates"))]
    #[test]
    fn missing_feed_still_leaves_the_mirror_attempts_available() {
        let config = one_core::config::UpdateConfig {
            update_url: String::new(),
            download_url: None,
        };

        assert_eq!(
            vec![UpdateAttempt::Cnb, UpdateAttempt::Github],
            update_attempts(&config, UpdateChannel::Stable, UpdateMirror::Auto)
        );
        // 没有 R2 清单时测试通道只剩 CNB，仍然不会把稳定版当 beta 推给用户。
        assert_eq!(
            vec![UpdateAttempt::Cnb],
            update_attempts(&config, UpdateChannel::Beta, UpdateMirror::Auto)
        );
    }

    #[cfg(feature = "github-updates")]
    #[test]
    fn github_updates_feature_ignores_the_mirror_preference() {
        let config = one_core::config::UpdateConfig {
            update_url: "https://cdn.example.test/updates/latest.json".to_string(),
            download_url: None,
        };

        assert_eq!(
            vec![UpdateAttempt::Github],
            update_attempts(&config, UpdateChannel::Stable, UpdateMirror::Cnb)
        );
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
            is_prerelease: false,
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
            is_prerelease: false,
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

        let outcome = perform_update_check(
            config,
            http_client,
            "0.10.0".to_string(),
            UpdateChannel::Stable,
            UpdateMirror::Auto,
        )
        .await;

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
        assert!(!info.is_prerelease, "本地模拟的正式版本不带预发布标记");
        assert!(client.take_requests().is_empty());
    }

    #[cfg(not(feature = "github-updates"))]
    #[tokio::test]
    async fn cloudflare_connectivity_failure_falls_back_to_github_release() {
        let update_url = update_url_from_public_base("https://onetcli.pdyyds.cn");
        let client = Arc::new(FakeHttpClient::new(vec![
            Err(anyhow!("r2 unavailable")),
            Err(anyhow!("cnb unavailable")),
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
            UpdateChannel::Stable,
            UpdateMirror::Auto,
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
        assert_eq!(requests[1].uri, super::mirror::cnb_refs_url());
        assert_eq!(requests[2].uri, "https://api.github.com/");
        assert_eq!(
            requests[3].uri,
            "https://api.github.com/repos/feigeCode/navop/releases/latest"
        );
    }

    #[cfg(not(feature = "github-updates"))]
    #[tokio::test]
    async fn missing_cloudflare_update_url_uses_the_next_mirrors() {
        let client = Arc::new(FakeHttpClient::new(vec![
            Err(anyhow!("cnb unavailable")),
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
            UpdateChannel::Stable,
            UpdateMirror::Auto,
        )
        .await;

        assert!(matches!(outcome, UpdateCheckOutcome::ShowDialog(_)));
        let requests = client.take_requests();
        assert_eq!(requests.len(), 3);
        assert_eq!(requests[0].uri, super::mirror::cnb_refs_url());
        assert_eq!(requests[1].uri, "https://api.github.com/");
        assert_eq!(
            requests[2].uri,
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
            Err(anyhow!("cnb unavailable")),
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
            UpdateChannel::Stable,
            UpdateMirror::Auto,
        )
        .await;

        let UpdateCheckOutcome::ShowDialog(info) = outcome else {
            panic!("R2 manifest 未更新时应继续回退到 GitHub Release");
        };
        assert_eq!(info.latest_version, "v9.9.9");

        let requests = client.take_requests();
        assert_eq!(requests[0].uri, update_url);
        assert_eq!(requests[1].uri, update_url);
        assert_eq!(requests[2].uri, super::mirror::cnb_refs_url());
        assert_eq!(requests[3].uri, "https://api.github.com/");
        assert_eq!(
            requests[4].uri,
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

        let outcome = perform_update_check(
            config,
            http_client,
            "0.1.0".to_string(),
            UpdateChannel::Stable,
            UpdateMirror::Auto,
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

    #[test]
    fn stable_channel_never_offers_prerelease_manifests() {
        let current = parse_version("0.22.0").expect("版本号合法");
        let beta = parse_version("0.23.0-beta.1").expect("版本号合法");
        let stable = parse_version("0.23.0").expect("版本号合法");

        // semver 认为 0.23.0-beta.1 > 0.22.0，稳定通道必须显式排除它。
        assert!(!should_offer_update(&beta, &current, UpdateChannel::Stable));
        assert!(should_offer_update(
            &stable,
            &current,
            UpdateChannel::Stable
        ));
        assert!(should_offer_update(&beta, &current, UpdateChannel::Beta));
        // 同版本不提示，测试通道也不接受降级。
        assert!(!should_offer_update(
            &current,
            &current,
            UpdateChannel::Beta
        ));
        assert!(!should_offer_update(&beta, &stable, UpdateChannel::Beta));
    }

    #[test]
    fn skipped_versions_are_scoped_per_channel() {
        let stored = encode_skipped_update_version(UpdateChannel::Beta, "0.23.0-beta.2");
        assert_eq!("beta:0.23.0-beta.2", stored);
        assert_eq!(
            Some("0.23.0-beta.2".to_string()),
            skipped_version_for_channel(UpdateChannel::Beta, Some(&stored))
        );
        // 在测试通道跳过的版本不能连带跳过稳定通道的同号版本。
        assert_eq!(
            None,
            skipped_version_for_channel(UpdateChannel::Stable, Some(&stored))
        );

        let stable_key = encode_skipped_update_version(UpdateChannel::Stable, "0.22.1");
        assert_eq!("stable:0.22.1", stable_key);
        assert_eq!(
            Some("0.22.1".to_string()),
            skipped_version_for_channel(UpdateChannel::Stable, Some(&stable_key))
        );
        assert_eq!(
            None,
            skipped_version_for_channel(UpdateChannel::Beta, Some(&stable_key))
        );

        // 历史数据是没有前缀的裸版本号，只对稳定通道生效。
        assert_eq!(
            Some("0.22.1".to_string()),
            skipped_version_for_channel(UpdateChannel::Stable, Some("0.22.1"))
        );
        assert_eq!(
            None,
            skipped_version_for_channel(UpdateChannel::Beta, Some("0.22.1"))
        );
        assert_eq!(
            None,
            skipped_version_for_channel(UpdateChannel::Beta, Some("  "))
        );
    }

    #[cfg(not(feature = "github-updates"))]
    #[tokio::test]
    async fn cnb_mirror_resolves_the_tag_and_rewrites_download_urls() {
        let refs_body = format!(
            "001e# service=git-upload-pack\n0000{:04x}4444444444444444444444444444444444444444 refs/tags/v9.9.9\n",
            4 + "4444444444444444444444444444444444444444 refs/tags/v9.9.9\n".len()
        );
        let feed_body = format!(
            r#"{{"version":"9.9.9","downloads":{{"{}":"https://r2.example.test/releases/v9.9.9/{}"}}}}"#,
            current_target_key(),
            expected_archive_name()
        );
        let client = Arc::new(FakeHttpClient::new(vec![
            FakeHttpClient::response(200, ""),
            FakeHttpClient::response(200, &refs_body),
            FakeHttpClient::response(200, &feed_body),
        ]));
        let http_client: Arc<dyn HttpClient> = client.clone();
        // 没有 R2 feed 时 CNB 就是第一个可用的源。
        let config = one_core::config::UpdateConfig {
            update_url: String::new(),
            download_url: None,
        };

        let outcome = perform_update_check(
            config,
            http_client,
            "0.1.0".to_string(),
            UpdateChannel::Stable,
            UpdateMirror::Cnb,
        )
        .await;

        let UpdateCheckOutcome::ShowDialog(info) = outcome else {
            panic!("CNB 镜像应给出更新");
        };
        assert_eq!(info.latest_version, "9.9.9");
        // 清单里指向 R2 的地址要被改写到 CNB 的同名资产上，否则国内用户依然慢。
        assert_eq!(
            Some(super::mirror::cnb_release_asset_url("v9.9.9", expected_archive_name()).as_str()),
            info.download_url.as_deref()
        );

        let requests = client.take_requests();
        assert_eq!(requests.len(), 3);
        assert_eq!(requests[0].uri, super::mirror::cnb_refs_url());
        assert_eq!(requests[1].uri, super::mirror::cnb_refs_url());
        assert_eq!(
            requests[2].uri,
            super::mirror::cnb_release_asset_url(
                "v9.9.9",
                &super::mirror::feed_asset_name("navop", UpdateChannel::Stable)
            )
        );
    }
}
