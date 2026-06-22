use std::{collections::HashMap, sync::Arc};

use futures::AsyncReadExt;
use gpui::http_client::{AsyncBody, HttpClient, Method, Request};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub(crate) struct UpdateResponse {
    pub(crate) version: String,
    #[serde(default)]
    download_url: Option<String>,
    #[serde(default)]
    downloads: Option<HashMap<String, String>>,
    #[serde(default, alias = "github_download_url")]
    fallback_download_url: Option<String>,
    #[serde(default, alias = "github_downloads")]
    fallback_downloads: Option<HashMap<String, String>>,
    #[serde(default)]
    pub(crate) sha256: Option<String>,
    #[serde(default)]
    sha256s: Option<HashMap<String, String>>,
}

pub(crate) async fn fetch_update_info(
    http_client: Arc<dyn HttpClient>,
    update_url: &str,
) -> Result<UpdateResponse, String> {
    let request = Request::builder()
        .method(Method::GET)
        .uri(update_url)
        .header("Accept", "application/json")
        .body(AsyncBody::empty())
        .map_err(|err| format!("构建更新请求失败: {}", err))?;

    let response = http_client
        .send(request)
        .await
        .map_err(|err| format!("发送更新请求失败: {}", err))?;

    let status = response.status();
    let mut body = response.into_body();
    let mut bytes = Vec::new();
    body.read_to_end(&mut bytes)
        .await
        .map_err(|err| format!("读取更新响应失败: {}", err))?;

    if !status.is_success() {
        return Err(format!("更新接口返回异常状态码: {}", status));
    }

    serde_json::from_slice::<UpdateResponse>(&bytes)
        .map_err(|err| format!("解析更新响应失败: {}", err))
}

pub(crate) fn select_download_url(
    response: &UpdateResponse,
    default_download_url: Option<String>,
) -> Option<String> {
    select_download_url_for_keys(response, default_download_url, platform_download_keys())
}

pub(crate) fn select_sha256(response: &UpdateResponse) -> Option<String> {
    select_sha256_for_keys(response, platform_download_keys())
}

pub(crate) fn select_fallback_download_url(response: &UpdateResponse) -> Option<String> {
    select_fallback_download_url_for_keys(response, platform_download_keys())
}

pub(crate) fn platform_download_keys_for(os: &str, arch: &str) -> &'static [&'static str] {
    match (os, arch) {
        ("macos", "aarch64") => &["aarch64-apple-darwin", "macos"],
        ("macos", "x86_64") => &["x86_64-apple-darwin", "macos"],
        ("linux", "x86_64") => &["x86_64-unknown-linux-gnu", "linux"],
        ("linux", "aarch64") => &["aarch64-unknown-linux-gnu", "linux"],
        ("windows", "x86_64") => &["x86_64-pc-windows-msvc", "windows"],
        _ => &[],
    }
}

fn select_download_url_for_keys(
    response: &UpdateResponse,
    default_download_url: Option<String>,
    keys: &[&str],
) -> Option<String> {
    select_keyed_value(&response.downloads, keys)
        .or_else(|| response.download_url.clone())
        .or(default_download_url)
}

fn select_sha256_for_keys(response: &UpdateResponse, keys: &[&str]) -> Option<String> {
    select_keyed_value(&response.sha256s, keys).or_else(|| response.sha256.clone())
}

fn select_fallback_download_url_for_keys(
    response: &UpdateResponse,
    keys: &[&str],
) -> Option<String> {
    select_keyed_value(&response.fallback_downloads, keys)
        .or_else(|| response.fallback_download_url.clone())
}

fn select_keyed_value(values: &Option<HashMap<String, String>>, keys: &[&str]) -> Option<String> {
    values
        .as_ref()
        .and_then(|values| keys.iter().find_map(|key| values.get(*key).cloned()))
}

fn platform_download_keys() -> &'static [&'static str] {
    platform_download_keys_for(std::env::consts::OS, std::env::consts::ARCH)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn select_download_url_prefers_target_triple_before_os_fallback() {
        let response = serde_json::from_str::<UpdateResponse>(
            r#"{
                "version": "1.2.3",
                "downloads": {
                    "aarch64-apple-darwin": "https://example.test/arm64.tar.gz",
                    "macos": "https://example.test/macos.tar.gz"
                }
            }"#,
        )
        .unwrap();

        assert_eq!(
            Some("https://example.test/arm64.tar.gz".to_string()),
            select_download_url_for_keys(&response, None, &["aarch64-apple-darwin", "macos"])
        );
    }

    #[test]
    fn select_sha256_prefers_target_triple_before_global_fallback() {
        let response = serde_json::from_str::<UpdateResponse>(
            r#"{
                "version": "1.2.3",
                "sha256": "global",
                "sha256s": {
                    "x86_64-unknown-linux-gnu": "linux-sha"
                }
            }"#,
        )
        .unwrap();

        assert_eq!(
            Some("linux-sha".to_string()),
            select_sha256_for_keys(&response, &["x86_64-unknown-linux-gnu", "linux"])
        );
    }

    #[test]
    fn select_fallback_download_url_prefers_target_triple_before_global_fallback() {
        let response = serde_json::from_str::<UpdateResponse>(
            r#"{
                "version": "1.2.3",
                "fallback_download_url": "https://github.example.test/global.tar.gz",
                "fallback_downloads": {
                    "x86_64-unknown-linux-gnu": "https://github.example.test/linux.tar.gz"
                }
            }"#,
        )
        .unwrap();

        assert_eq!(
            Some("https://github.example.test/linux.tar.gz".to_string()),
            select_fallback_download_url_for_keys(
                &response,
                &["x86_64-unknown-linux-gnu", "linux"]
            )
        );
    }

    #[test]
    fn platform_download_keys_include_linux_arm64() {
        assert_eq!(
            &["aarch64-unknown-linux-gnu", "linux"],
            platform_download_keys_for("linux", "aarch64")
        );
    }
}
