use std::sync::Arc;

use futures::AsyncReadExt;
use gpui::http_client::{AsyncBody, HttpClient, Method, Request};
use serde::Deserialize;

use super::UpdateDialogInfo;

const GITHUB_OWNER: &str = "feigeCode";
const GITHUB_REPO: &str = "onetcli";
const GITHUB_API_URL: &str = "https://api.github.com/repos/feigeCode/onetcli/releases/latest";
const GITHUB_USER_AGENT: &str = "onetcli-updater";

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const EXPECTED_ARCHIVE_NAME: &str = "onetcli-aarch64-apple-darwin.tar.gz";
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
const EXPECTED_ARCHIVE_NAME: &str = "onetcli-x86_64-apple-darwin.tar.gz";
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const EXPECTED_ARCHIVE_NAME: &str = "onetcli-x86_64-unknown-linux-gnu.tar.gz";
#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
const EXPECTED_ARCHIVE_NAME: &str = "onetcli-x86_64-pc-windows-msvc.zip";
#[cfg(not(any(
    all(target_os = "macos", target_arch = "aarch64"),
    all(target_os = "macos", target_arch = "x86_64"),
    all(target_os = "linux", target_arch = "x86_64"),
    all(target_os = "windows", target_arch = "x86_64")
)))]
const EXPECTED_ARCHIVE_NAME: &str = "";

#[derive(Debug, Deserialize)]
pub(crate) struct GithubReleaseAsset {
    pub(crate) name: String,
    pub(crate) browser_download_url: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GithubRelease {
    pub(crate) tag_name: String,
    pub(crate) assets: Vec<GithubReleaseAsset>,
}

pub(crate) async fn fetch_github_release(
    http_client: Arc<dyn HttpClient>,
) -> Result<GithubRelease, String> {
    let request = Request::builder()
        .method(Method::GET)
        .uri(GITHUB_API_URL)
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", GITHUB_USER_AGENT)
        .body(AsyncBody::empty())
        .map_err(|err| format!("构建 GitHub Release 请求失败: {}", err))?;

    let response = http_client
        .send(request)
        .await
        .map_err(|err| format!("发送 GitHub Release 请求失败: {}", err))?;

    let status = response.status();
    let mut body = response.into_body();
    let mut bytes = Vec::new();
    body.read_to_end(&mut bytes)
        .await
        .map_err(|err| format!("读取 GitHub Release 响应失败: {}", err))?;

    if !status.is_success() {
        return Err(format!(
            "GitHub Release 接口返回异常状态码: {} ({}/{})",
            status, GITHUB_OWNER, GITHUB_REPO
        ));
    }

    serde_json::from_slice::<GithubRelease>(&bytes)
        .map_err(|err| format!("解析 GitHub Release 响应失败: {}", err))
}

pub(crate) fn select_github_asset(release: &GithubRelease) -> Option<&GithubReleaseAsset> {
    if EXPECTED_ARCHIVE_NAME.is_empty() {
        return None;
    }

    release
        .assets
        .iter()
        .find(|asset| asset.name == EXPECTED_ARCHIVE_NAME)
}

pub(crate) fn github_release_to_dialog_info(
    release: &GithubRelease,
    current_version: &str,
) -> Result<UpdateDialogInfo, String> {
    let asset = select_github_asset(release)
        .ok_or_else(|| format!("未找到当前平台的发布资产: {}", EXPECTED_ARCHIVE_NAME))?;

    let release_page_url = format!(
        "https://github.com/{}/{}/releases/latest",
        GITHUB_OWNER, GITHUB_REPO
    );

    Ok(UpdateDialogInfo {
        current_version: current_version.to_string(),
        latest_version: release.tag_name.clone(),
        download_url: Some(asset.browser_download_url.clone()),
        expected_sha256: None,
        release_page_url: Some(release_page_url),
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use anyhow::anyhow;
    use gpui::http_client::{HttpClient, http};

    use super::*;
    use crate::update::test_support::FakeHttpClient;

    #[tokio::test]
    async fn fetch_github_release_sends_expected_request() {
        let client = Arc::new(FakeHttpClient::new(vec![FakeHttpClient::response(
            200,
            r#"{
                "tag_name":"v1.2.3",
                "body":"release notes",
                "assets":[]
            }"#,
        )]));
        let http_client: Arc<dyn HttpClient> = client.clone();

        let release = fetch_github_release(http_client)
            .await
            .expect("GitHub Release 请求应成功");

        assert_eq!(release.tag_name, "v1.2.3");

        let requests = client.take_requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].method, http::Method::GET);
        assert_eq!(requests[0].uri, GITHUB_API_URL);
        assert_eq!(requests[0].user_agent.as_deref(), Some(GITHUB_USER_AGENT));
    }

    #[test]
    fn github_release_to_dialog_info_uses_matching_asset() {
        let release = GithubRelease {
            tag_name: "v1.2.3".to_string(),
            assets: vec![
                GithubReleaseAsset {
                    name: "sha256sums.txt".to_string(),
                    browser_download_url: "https://example.com/sha256".to_string(),
                },
                GithubReleaseAsset {
                    name: EXPECTED_ARCHIVE_NAME.to_string(),
                    browser_download_url: "https://example.com/update".to_string(),
                },
            ],
        };

        let info = github_release_to_dialog_info(&release, "0.1.0").expect("应选择当前平台资产");

        assert_eq!(info.latest_version, "v1.2.3");
        assert_eq!(info.current_version, "0.1.0");
        assert_eq!(
            info.download_url.as_deref(),
            Some("https://example.com/update")
        );
    }

    #[tokio::test]
    async fn fetch_github_release_returns_error_on_transport_failure() {
        let client = Arc::new(FakeHttpClient::new(vec![Err(anyhow!("network down"))]));
        let http_client: Arc<dyn HttpClient> = client;

        let err = fetch_github_release(http_client)
            .await
            .expect_err("传输失败应返回错误");

        assert!(err.contains("发送 GitHub Release 请求失败"));
    }
}
