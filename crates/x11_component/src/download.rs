use futures_util::AsyncReadExt as _;
use http_client::{AsyncBody, HttpClient as _, Method, Request};
use reqwest_client::ReqwestClient;

use crate::X11ComponentError;

const HTTP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

pub async fn fetch(
    client: &ReqwestClient,
    url: &str,
    limit: usize,
) -> Result<Vec<u8>, X11ComponentError> {
    with_download_timeout(HTTP_TIMEOUT, fetch_unbounded(client, url, limit)).await
}

async fn with_download_timeout<F>(
    duration: std::time::Duration,
    operation: F,
) -> Result<Vec<u8>, X11ComponentError>
where
    F: std::future::Future<Output = Result<Vec<u8>, X11ComponentError>>,
{
    tokio::time::timeout(duration, operation)
        .await
        .map_err(|_| X11ComponentError::Download("HTTP 请求超过 60 秒".into()))?
}

async fn fetch_unbounded(
    client: &ReqwestClient,
    url: &str,
    limit: usize,
) -> Result<Vec<u8>, X11ComponentError> {
    let request = Request::builder()
        .method(Method::GET)
        .uri(url)
        .body(AsyncBody::empty())
        .map_err(|error| X11ComponentError::Download(error.to_string()))?;
    let response = client
        .send(request)
        .await
        .map_err(|error| X11ComponentError::Download(error.to_string()))?;
    if !response.status().is_success() {
        return Err(X11ComponentError::Download(format!(
            "HTTP {}：{url}",
            response.status()
        )));
    }
    let mut body = response.into_body().take(limit as u64 + 1);
    let mut bytes = Vec::new();
    body.read_to_end(&mut bytes)
        .await
        .map_err(|error| X11ComponentError::Download(error.to_string()))?;
    if bytes.len() > limit {
        return Err(X11ComponentError::Download("响应超过大小上限".into()));
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests;
