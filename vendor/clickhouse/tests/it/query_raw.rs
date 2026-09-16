//! Tests for `Client::query_raw()`, which sends SQL to the server verbatim,
//! without client-side bind parameter parsing.
//!
//! See https://github.com/ClickHouse/adbc_clickhouse/issues/53.

use serde::Deserialize;

use clickhouse::{Row, error::Error};

use crate::{flush_query_log, get_client};

#[tokio::test]
async fn keeps_question_marks_verbatim() {
    let client = get_client();

    #[derive(Debug, Row, Deserialize)]
    struct MyRow {
        s: String,
        n: u8,
    }

    // `?` is not treated as a bind placeholder.
    let row = client
        .query_raw("SELECT 'a?b' AS s, 1 AS n")
        .fetch_one::<MyRow>()
        .await
        .unwrap();
    assert_eq!(row.s, "a?b");
    assert_eq!(row.n, 1);

    // `??` is not unescaped to `?`.
    let value = client
        .query_raw("SELECT 'a??b'")
        .fetch_one::<String>()
        .await
        .unwrap();
    assert_eq!(value, "a??b");
}

#[tokio::test]
async fn question_marks_in_comments() {
    let client = get_client();

    let value = client
        .query_raw("SELECT 1 /* What? How?? Why??? */ WHERE 1 = 1")
        .fetch_one::<u8>()
        .await
        .unwrap();
    assert_eq!(value, 1);
}

#[tokio::test]
async fn rejects_bind() {
    let client = get_client();

    let err = client
        .query_raw("SELECT ?")
        .bind(42)
        .execute()
        .await
        .unwrap_err();
    assert!(matches!(err, Error::InvalidParams(_)));
    assert!(
        err.to_string()
            .contains("bind() is not supported for raw queries")
    );
}

#[tokio::test]
async fn with_server_side_param() {
    let client = get_client();

    let value = client
        .query_raw("SELECT concat('foo?', {suffix: String}) AS result")
        .param("suffix", "bar")
        .fetch_one::<String>()
        .await
        .unwrap();
    assert_eq!(value, "foo?bar");
}

/// Checks via `system.query_log` that the server received
/// exactly the SQL that was passed to `query_raw()`.
#[tokio::test]
async fn sql_arrives_verbatim() {
    let client = get_client();
    let query_id = uuid::Uuid::new_v4().to_string();

    const SQL: &str = "SELECT 'a?b' /* What? How?? Why??? */ AS s";

    let value = client
        .query_raw(SQL)
        .with_setting("query_id", &query_id)
        .fetch_one::<String>()
        .await
        .unwrap();
    assert_eq!(value, "a?b");

    flush_query_log(&client).await;

    let logged = client
        .query(
            "SELECT query FROM system.query_log \
             WHERE query_id = ? AND type = 'QueryFinish'",
        )
        .bind(&query_id)
        .fetch_one::<String>()
        .await
        .unwrap();
    assert_eq!(logged, SQL);
}
