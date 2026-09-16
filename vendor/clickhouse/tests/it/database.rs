//! Tests for the `database` URL query parameter.
//!
//! See: <https://github.com/ClickHouse/clickhouse-rs/issues/448>

use clickhouse::Client;

/// Build a client for the current test environment with the given URL query string.
fn client_with_url_query(query: &str) -> Client {
    match crate::test_env() {
        crate::TestEnv::Local => {
            Client::default().with_url(format!("http://localhost:8123?{query}"))
        }
        crate::TestEnv::Cloud => Client::default()
            .with_url(format!("{}?{query}", crate::get_cloud_url()))
            .with_user("default")
            .with_password(crate::require_env_var("CLICKHOUSE_CLOUD_PASSWORD")),
    }
}

async fn current_database(client: &Client) -> String {
    client
        .query("SELECT currentDatabase()")
        .fetch_one::<String>()
        .await
        .unwrap()
}

#[tokio::test]
async fn query_uses_url_database_param() {
    let client = client_with_url_query("database=system");
    assert_eq!(current_database(&client).await, "system");
}

#[tokio::test]
async fn last_configured_database_wins() {
    let client = client_with_url_query("database=system").with_database("default");
    assert_eq!(current_database(&client).await, "default");
}
