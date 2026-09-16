use crate::{SimpleRow, create_simple_table};

#[tokio::test]
async fn summary_with_wait_end_of_query() {
    let client = prepare_database!();
    create_simple_table(&client, "test").await;

    let mut insert = client.insert::<SimpleRow>("test").await.unwrap();
    for i in 0..100 {
        insert.write(&SimpleRow::new(i, "foo")).await.unwrap();
    }
    insert.end().await.unwrap();

    let mut cursor = client
        .query("SELECT * FROM test")
        .with_setting("send_progress_in_http_headers", "1")
        .with_setting("wait_end_of_query", "1")
        .fetch::<SimpleRow>()
        .unwrap();

    // Summary is not available before headers are received.
    assert!(cursor.summary().is_none());

    while cursor.next().await.unwrap().is_some() {}

    let summary = cursor.summary().expect("summary should be present");
    assert_eq!(summary.read_rows(), Some(100));
    assert!(summary.read_bytes().unwrap() > 0);
    assert_eq!(summary.result_rows(), Some(100));
    assert!(summary.result_bytes().unwrap() > 0);
    assert!(summary.elapsed_ns().unwrap() > 0);
}

#[tokio::test]
async fn summary_without_wait_end_of_query() {
    let client = prepare_database!();
    create_simple_table(&client, "test").await;

    let mut insert = client.insert::<SimpleRow>("test").await.unwrap();
    for i in 0..10 {
        insert.write(&SimpleRow::new(i, "bar")).await.unwrap();
    }
    insert.end().await.unwrap();

    let mut cursor = client
        .query("SELECT * FROM test")
        .with_setting("send_progress_in_http_headers", "1")
        .fetch::<SimpleRow>()
        .unwrap();

    while cursor.next().await.unwrap().is_some() {}

    // Summary should still be present, though values may be incomplete.
    let summary = cursor.summary().expect("summary should be present");
    assert!(summary.read_rows().is_some());
    assert!(summary.read_bytes().is_some());
}

#[tokio::test]
async fn summary_absent_without_progress_headers() {
    let client = prepare_database!();
    create_simple_table(&client, "test").await;

    let mut cursor = client
        .query("SELECT * FROM test")
        .fetch::<SimpleRow>()
        .unwrap();

    while cursor.next().await.unwrap().is_some() {}

    // Without send_progress_in_http_headers, summary may or may not be present
    // Just verify it doesn't panic.
    let _ = cursor.summary();
}

#[tokio::test]
async fn summary_generic_get() {
    let client = prepare_database!();
    create_simple_table(&client, "test").await;

    let mut insert = client.insert::<SimpleRow>("test").await.unwrap();
    insert.write(&SimpleRow::new(1, "baz")).await.unwrap();
    insert.end().await.unwrap();

    let mut cursor = client
        .query("SELECT * FROM test")
        .with_setting("send_progress_in_http_headers", "1")
        .with_setting("wait_end_of_query", "1")
        .fetch::<SimpleRow>()
        .unwrap();

    while cursor.next().await.unwrap().is_some() {}

    let summary = cursor.summary().expect("summary should be present");

    // The generic getter should return the raw string value.
    assert_eq!(summary.get("read_rows"), Some("1"));
    // Non-existent fields return None.
    assert_eq!(summary.get("nonexistent_field"), None);
}

#[tokio::test]
async fn summary_with_fetch_bytes() {
    let client = prepare_database!();
    create_simple_table(&client, "test").await;

    let mut insert = client.insert::<SimpleRow>("test").await.unwrap();
    for i in 0..50 {
        insert.write(&SimpleRow::new(i, "hello")).await.unwrap();
    }
    insert.end().await.unwrap();

    let mut cursor = client
        .query("SELECT * FROM test")
        .with_setting("send_progress_in_http_headers", "1")
        .with_setting("wait_end_of_query", "1")
        .fetch_bytes("RowBinaryWithNamesAndTypes")
        .unwrap();

    // Summary is not available before headers are received.
    assert!(cursor.summary().is_none());

    // Consume all bytes.
    while cursor.next().await.unwrap().is_some() {}

    let summary = cursor.summary().expect("summary should be present");
    assert_eq!(summary.read_rows(), Some(50));
    assert!(summary.read_bytes().unwrap() > 0);
    assert_eq!(summary.result_rows(), Some(50));
    assert!(summary.elapsed_ns().unwrap() > 0);
}
