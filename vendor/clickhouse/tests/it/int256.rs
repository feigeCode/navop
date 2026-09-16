use serde::{Deserialize, Serialize};

use clickhouse::Row;
use clickhouse::types::{Int256, UInt256};

#[tokio::test]
async fn uint_256() {
    let client = prepare_database!();

    #[derive(Debug, PartialEq, Eq, Serialize, Deserialize, Row)]
    struct MyRow {
        id: UInt256,
        value: String,
    }

    client
        .query(
            "
            CREATE TABLE test(
                id UInt256,
                value String,
            ) ENGINE = MergeTree ORDER BY id
        ",
        )
        .execute()
        .await
        .unwrap();

    let ids = [
        UInt256::ZERO,
        1u32.into(),
        1234567890u32.into(),
        UInt256::MAX,
    ];

    let original_rows = ids
        .into_iter()
        .enumerate()
        .map(|(i, id)| MyRow {
            id,
            value: format!("test_{i}"),
        })
        .collect::<Vec<_>>();

    let mut insert = client.insert::<MyRow>("test").await.unwrap();
    for row in &original_rows {
        insert.write(row).await.unwrap();
    }
    insert.end().await.unwrap();

    // Test binding individual values
    for original in &original_rows {
        let row = client
            .query("SELECT ?fields FROM test WHERE id = ?")
            .bind(original.id)
            .fetch_one::<MyRow>()
            .await
            .unwrap();

        assert_eq!(row, *original);

        let row = client
            .query("SELECT ?fields FROM test WHERE id = {id:UInt256}")
            .param("id", original.id)
            .fetch_one::<MyRow>()
            .await
            .unwrap();

        assert_eq!(row, *original);
    }

    // Test binding arrays
    let rows = client
        .query("SELECT ?fields FROM test WHERE id IN ? ORDER BY value")
        .bind(ids)
        .fetch_all::<MyRow>()
        .await
        .unwrap();

    assert_eq!(rows, original_rows);

    // https://github.com/ClickHouse/clickhouse-rs/issues/290
    let rows = client
        .query("SELECT ?fields FROM test WHERE id IN {ids:Array(UInt256)} ORDER BY value")
        // FIXME(?): `impl Serialize for [T; N]` uses `serialize_tuple()`
        // but `Vec<T>` and `&[T]` use `serialize_seq()`
        .param("ids", &ids[..])
        .fetch_all::<MyRow>()
        .await
        .unwrap();

    assert_eq!(rows, original_rows);
}

#[tokio::test]
async fn int_256() {
    let client = prepare_database!();

    #[derive(Debug, PartialEq, Eq, Serialize, Deserialize, Row)]
    struct MyRow {
        id: Int256,
        value: String,
    }

    client
        .query(
            "
            CREATE TABLE test(
                id Int256,
                value String,
            ) ENGINE = MergeTree ORDER BY id
        ",
        )
        .execute()
        .await
        .unwrap();

    let ids = [
        Int256::MIN,
        (-1234567890).into(),
        (-1).into(),
        0.into(),
        1.into(),
        1234567890.into(),
        Int256::MAX,
    ];

    let original_rows = ids
        .into_iter()
        .enumerate()
        .map(|(i, id)| MyRow {
            id,
            value: format!("test_{i}"),
        })
        .collect::<Vec<_>>();

    let mut insert = client.insert::<MyRow>("test").await.unwrap();
    for row in &original_rows {
        insert.write(row).await.unwrap();
    }
    insert.end().await.unwrap();

    // Test binding individual values
    for original in &original_rows {
        let row = client
            .query("SELECT ?fields FROM test WHERE id = ?")
            .bind(original.id)
            .fetch_one::<MyRow>()
            .await
            .unwrap();

        assert_eq!(row, *original);

        let row = client
            .query("SELECT ?fields FROM test WHERE id = {id:Int256}")
            .param("id", original.id)
            .fetch_one::<MyRow>()
            .await
            .unwrap();

        assert_eq!(row, *original);
    }

    // Test binding arrays
    let rows = client
        .query("SELECT ?fields FROM test WHERE id IN ? ORDER BY value")
        .bind(ids)
        .fetch_all::<MyRow>()
        .await
        .unwrap();

    assert_eq!(rows, original_rows);

    // https://github.com/ClickHouse/clickhouse-rs/issues/290
    let rows = client
        .query("SELECT ?fields FROM test WHERE id IN {ids:Array(Int256)} ORDER BY value")
        // FIXME(?): `impl Serialize for [T; N]` uses `serialize_tuple()`
        // but `Vec<T>` and `&[T]` use `serialize_seq()`
        .param("ids", &ids[..])
        .fetch_all::<MyRow>()
        .await
        .unwrap();

    assert_eq!(rows, original_rows);
}
