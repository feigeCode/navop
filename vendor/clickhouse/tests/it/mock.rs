#![cfg(feature = "test-util")]

use crate::SimpleRow;
use clickhouse::{Client, test};
use std::time::Duration;

async fn test_provide() {
    let mock = test::Mock::new();
    let client = Client::default().with_mock(&mock);
    let expected = vec![SimpleRow::new(1, "one"), SimpleRow::new(2, "two")];

    // FIXME: &expected is not allowed due to new trait bounds
    mock.add(test::handlers::provide(expected.clone()));

    let actual = crate::fetch_rows::<SimpleRow>(&client, "doesn't matter").await;
    assert_eq!(actual, expected);
}

#[tokio::test]
async fn provide() {
    test_provide().await;

    // Same but with the advanced time.
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(100_000)).await;
    test_provide().await;
}

#[tokio::test]
async fn summary_header() {
    let mock = test::Mock::new();
    let client = Client::default().with_mock(&mock);
    let rows = vec![SimpleRow::new(1, "one"), SimpleRow::new(2, "two")];
    let summary_json = r#"{"read_rows":"2","read_bytes":"16","written_rows":"0","written_bytes":"0","total_rows_to_read":"2","result_rows":"3","result_bytes":"24","elapsed_ns":"12345","memory_usage":"1024"}"#;

    mock.add(test::handlers::provide_with_summary(
        rows.clone(),
        summary_json,
    ));

    let mut cursor = client.query("doesn't matter").fetch::<SimpleRow>().unwrap();

    // Summary is not available before headers are received.
    assert!(cursor.summary().is_none());

    let mut actual = Vec::new();
    while let Some(row) = cursor.next().await.unwrap() {
        actual.push(row);
    }

    assert_eq!(actual, rows);

    let summary = cursor.summary().expect("summary should be present");
    assert_eq!(summary.read_rows(), Some(2));
    assert_eq!(summary.read_bytes(), Some(16));
    assert_eq!(summary.written_rows(), Some(0));
    assert_eq!(summary.written_bytes(), Some(0));
    assert_eq!(summary.total_rows_to_read(), Some(2));
    assert_eq!(summary.result_rows(), Some(3));
    assert_eq!(summary.result_bytes(), Some(24));
    assert_eq!(summary.elapsed_ns(), Some(12345));
    assert_eq!(summary.memory_usage(), Some(1024));
}

#[tokio::test]
async fn summary_header_absent() {
    let mock = test::Mock::new();
    let client = Client::default().with_mock(&mock);
    let rows = vec![SimpleRow::new(1, "one")];

    mock.add(test::handlers::provide(rows.clone()));

    let mut cursor = client.query("doesn't matter").fetch::<SimpleRow>().unwrap();

    while cursor.next().await.unwrap().is_some() {}

    assert!(cursor.summary().is_none());
}

#[tokio::test]
async fn summary_header_malformed() {
    let mock = test::Mock::new();
    let client = Client::default().with_mock(&mock);
    let rows = vec![SimpleRow::new(1, "one")];

    mock.add(test::handlers::provide_with_summary(
        rows.clone(),
        "not valid json",
    ));

    let mut cursor = client.query("doesn't matter").fetch::<SimpleRow>().unwrap();

    while cursor.next().await.unwrap().is_some() {}

    // Malformed summary should be silently ignored.
    assert!(cursor.summary().is_none());
}

#[tokio::test]
async fn summary_header_unknown_fields() {
    let mock = test::Mock::new();
    let client = Client::default().with_mock(&mock);
    let rows = vec![SimpleRow::new(1, "one")];

    // Include a known field and an unknown future field.
    let summary_json = r#"{"read_rows":"42","some_future_field":"hello"}"#;

    mock.add(test::handlers::provide_with_summary(
        rows.clone(),
        summary_json,
    ));

    let mut cursor = client.query("doesn't matter").fetch::<SimpleRow>().unwrap();

    while cursor.next().await.unwrap().is_some() {}

    let summary = cursor.summary().expect("summary should be present");
    // Known field is accessible via typed getter.
    assert_eq!(summary.read_rows(), Some(42));
    // Missing known fields return None instead of failing.
    assert_eq!(summary.written_rows(), None);
    // Unknown fields are accessible via the generic getter.
    assert_eq!(summary.get("some_future_field"), Some("hello"));
    assert_eq!(summary.get("nonexistent"), None);
}

#[tokio::test]
async fn client_with_url() {
    let mock = test::Mock::new();

    // Existing usages before `with_mock()` was introduced should not silently break.
    let client = Client::default().with_url(mock.url());
    let expected = vec![SimpleRow::new(1, "one"), SimpleRow::new(2, "two")];

    // FIXME: &expected is not allowed due to new trait bounds
    mock.add(test::handlers::provide(expected.clone()));

    let actual = crate::fetch_rows::<SimpleRow>(&client, "doesn't matter").await;
    assert_eq!(actual, expected);
}
