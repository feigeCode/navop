use std::sync::Arc;

use gpui::http_client::{AsyncBody, HttpClient, Method, Request};
use rust_i18n::t;

const GITHUB_CONNECTIVITY_URL: &str = "https://www.gstatic.com/generate_204";
const GITHUB_USER_AGENT: &str = "onetcli-updater";

pub(crate) async fn check_network_connectivity(
    http_client: Arc<dyn HttpClient>,
) -> Result<(), String> {
    let request = Request::builder()
        .method(Method::HEAD)
        .uri(GITHUB_CONNECTIVITY_URL)
        .header("User-Agent", GITHUB_USER_AGENT)
        .body(AsyncBody::empty())
        .map_err(|err| format!("构建网络检查请求失败: {}", err))?;

    let response = http_client
        .send(request)
        .await
        .map_err(|_err| t!("Update.network_unreachable").to_string())?;

    if response.status().is_server_error() {
        return Err(t!("Update.github_service_unavailable").to_string());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use anyhow::anyhow;
    use gpui::http_client::{HttpClient, http};

    use super::*;
    use crate::update::test_support::FakeHttpClient;

    #[tokio::test]
    async fn check_network_connectivity_uses_head_with_user_agent() {
        let client = Arc::new(FakeHttpClient::new(vec![FakeHttpClient::response(200, "")]));
        let http_client: Arc<dyn HttpClient> = client.clone();

        check_network_connectivity(http_client)
            .await
            .expect("网络检查应成功");

        let requests = client.take_requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].method, http::Method::HEAD);
        assert_eq!(requests[0].uri, GITHUB_CONNECTIVITY_URL);
        assert_eq!(requests[0].user_agent.as_deref(), Some(GITHUB_USER_AGENT));
    }

    #[tokio::test]
    async fn check_network_connectivity_maps_server_errors() {
        let client = Arc::new(FakeHttpClient::new(vec![FakeHttpClient::response(503, "")]));
        let http_client: Arc<dyn HttpClient> = client;

        let err = check_network_connectivity(http_client)
            .await
            .expect_err("5xx 应映射为服务不可用");

        assert_eq!(err, t!("Update.github_service_unavailable").to_string());
    }

    #[tokio::test]
    async fn check_network_connectivity_maps_transport_errors() {
        let client = Arc::new(FakeHttpClient::new(vec![Err(anyhow!("dns failed"))]));
        let http_client: Arc<dyn HttpClient> = client;

        let err = check_network_connectivity(http_client)
            .await
            .expect_err("传输失败应映射为网络不可达");

        assert_eq!(err, t!("Update.network_unreachable").to_string());
    }
}
