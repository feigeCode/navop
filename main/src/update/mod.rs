use std::path::PathBuf;

use gpui::{App, AppContext, Window};
use rust_i18n::t;

use crate::setting_tab::AppSettings;
use one_core::config::UpdateConfig;

mod custom_api;
mod dialog;
mod download;
mod github_release;
mod install;
mod network;
mod util;

use custom_api::{fetch_update_info, select_download_url};
use dialog::show_update_dialog;
use github_release::{fetch_github_release, github_release_to_dialog_info};
use install::apply_update_helper;
use network::check_network_connectivity;
use util::parse_version;

const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
const APPLY_UPDATE_FLAG: &str = "--apply-update";

#[derive(Clone, Debug)]
pub(crate) struct UpdateDialogInfo {
    current_version: String,
    latest_version: String,
    download_url: Option<String>,
    release_notes: Option<String>,
}

pub fn handle_update_command() -> bool {
    let mut args = std::env::args().skip(1);
    let Some(flag) = args.next() else {
        return false;
    };

    if flag != APPLY_UPDATE_FLAG {
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
    if !AppSettings::global(cx).auto_update {
        return;
    }

    let config = UpdateConfig::get();
    let http_client = cx.http_client();
    let current_version = CURRENT_VERSION.to_string();

    window
        .spawn(cx, async move |cx| {
            if let Err(err) = check_network_connectivity(http_client.clone()).await {
                tracing::warn!("{}: {}", t!("Update.network_check_failed"), err);
                return;
            }

            match fetch_github_dialog_info(http_client.clone(), &current_version).await {
                Ok(Some(info)) => {
                    show_update_dialog_on_active_window(info, cx);
                    return;
                }
                Ok(None) => return,
                Err(err) => {
                    tracing::warn!("GitHub Release 检查失败: {}", err);
                }
            }

            match fetch_custom_dialog_info(&config, http_client, &current_version).await {
                Ok(Some(info)) => show_update_dialog_on_active_window(info, cx),
                Ok(None) => {}
                Err(err) => {
                    tracing::warn!("自定义更新检查失败: {}", err);
                }
            }
        })
        .detach();
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
        return Err("缺少 ONETCLI_UPDATE_URL，无法使用自定义更新接口兜底".to_string());
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
        release_notes: response.release_notes.clone(),
    }))
}

fn show_update_dialog_on_active_window(info: UpdateDialogInfo, cx: &mut gpui::AsyncApp) {
    let _ = cx.update(|cx| {
        if let Some(window_id) = cx.active_window() {
            let _ = cx.update_window(window_id, |_, window, cx| {
                show_update_dialog(window, info.clone(), cx);
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
        fn proxy(&self) -> Option<&Url> {
            None
        }

        fn user_agent(&self) -> Option<&http::HeaderValue> {
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
