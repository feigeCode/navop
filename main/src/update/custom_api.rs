use std::sync::Arc;

use futures::AsyncReadExt;
use gpui::http_client::{AsyncBody, HttpClient, Method, Request};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub(crate) struct UpdateDownloads {
    #[serde(default)]
    #[allow(dead_code)]
    windows: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    macos: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    linux: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct UpdateResponse {
    pub(crate) version: String,
    #[serde(default)]
    download_url: Option<String>,
    #[serde(default)]
    downloads: Option<UpdateDownloads>,
    #[serde(default)]
    pub(crate) sha256: Option<String>,
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
    let platform_url = response.downloads.as_ref().and_then(|downloads| {
        #[cfg(target_os = "windows")]
        {
            return downloads.windows.clone();
        }
        #[cfg(target_os = "macos")]
        {
            return downloads.macos.clone();
        }
        #[cfg(target_os = "linux")]
        {
            return downloads.linux.clone();
        }
        #[allow(unreachable_code)]
        None
    });

    platform_url
        .or_else(|| response.download_url.clone())
        .or(default_download_url)
}
