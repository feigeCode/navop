use crate::get_client_with_session;
use clickhouse::native::builder::BlockBuilder;
use std::collections::HashMap;
use std::time::Duration;

#[tokio::test]
async fn mixed_types_1() {
    mixed_types(1).await
}

#[tokio::test]
async fn mixed_types_10() {
    mixed_types(10).await
}

#[tokio::test]
async fn mixed_types_100() {
    mixed_types(100).await
}

#[tokio::test]
async fn mixed_types_1000() {
    mixed_types(1000).await
}

async fn mixed_types(num_rows: u64) {
    let client = get_client_with_session()
        // Note: only in ClickHouse 26.6
        .with_setting("enable_nullable_tuple_type", "1");

    client
        .query(
            "CREATE TEMPORARY TABLE foo(
                number Int32,
                text String,
                nullable_number Nullable(UInt64),
                nullable_text Nullable(String),
                number_tuple Tuple(UInt32, Int64),
                number_text_tuple Tuple(Int64, String),
                nullable_tuple Tuple(Nullable(Int64), Nullable(String)),
                tuple_nullable Nullable(Tuple(Int32, String)),
                low_cardinality_text LowCardinality(String),
                number_array Array(Int32),
                text_array Array(String),
                nullable_text_array Array(Nullable(String)),
                number_text_map Map(Int32, String),
            )",
        )
        .execute()
        .await
        .unwrap();

    let numbers = (0..(num_rows as i32)).collect::<Vec<_>>();
    let texts = (0..(num_rows as usize))
        .map(|i| {
            if i == 0 {
                return "".to_string();
            }

            // Left-pad with dots to `i` width
            format!("{i:.<i$}")
        })
        .collect::<Vec<_>>();

    let number_arrays = (0..(num_rows as usize))
        .map(|len| (0..len as i32).collect::<Vec<_>>())
        .collect::<Vec<_>>();

    let text_arrays = (0..(num_rows as usize))
        .map(|len| {
            (0..len)
                .map(|i| {
                    if i == 0 {
                        return "".to_string();
                    }

                    // Right-pad with asterisks to `i` width
                    format!("{i:*>i$}")
                })
                .collect::<Vec<String>>()
        })
        .collect::<Vec<_>>();

    let maps = (0..(num_rows as usize))
        .map(|len| {
            numbers[..len]
                .iter()
                .zip(&texts)
                .map(|(&n, t)| (n, t.clone()))
                .collect::<HashMap<_, _>>()
        })
        .collect::<Vec<_>>();

    let mut builder = BlockBuilder::new();

    builder
        .upsert_column("number")
        .unwrap()
        .add_all(&numbers)
        .unwrap();

    builder
        .upsert_column("text")
        .unwrap()
        .add_all(&texts)
        .unwrap();

    builder
        .upsert_column("nullable_number")
        .unwrap()
        .add_all(numbers.iter().map(|&i| (i % 2 == 0).then_some(i)))
        .unwrap();

    builder
        .upsert_column("nullable_text")
        .unwrap()
        .add_all(
            numbers
                .iter()
                .zip(&texts)
                .map(|(&number, text)| (number % 2 != 0).then_some(text)),
        )
        .unwrap();

    builder
        .upsert_column::<(i32, i64)>("number_tuple")
        .unwrap()
        .add_all(numbers.iter().map(|&i| (i, i as i64)))
        .unwrap();

    builder
        .upsert_column("number_text_tuple")
        .unwrap()
        .add_all(
            numbers
                .iter()
                .zip(&texts)
                .map(|(&number, text)| (number as i64, text)),
        )
        .unwrap();

    builder
        .upsert_column("nullable_tuple")
        .unwrap()
        .add_all(numbers.iter().zip(&texts).map(|(&number, text)| {
            (
                (number % 2 == 0).then_some(number as i64),
                (number % 2 != 0).then_some(text),
            )
        }))
        .unwrap();

    // Note: `Nullable(Tuple(...))` is experimental
    builder
        .upsert_column("tuple_nullable")
        .unwrap()
        .add_all(
            numbers
                .iter()
                .zip(&texts)
                .map(|(&number, text)| (number % 2 == 0).then_some((number, text))),
        )
        .unwrap();

    // Verifying the assumption that `LowCardinality` can accept a regular data stream
    builder
        .upsert_column("low_cardinality_text")
        .unwrap()
        .add_all(&texts)
        .unwrap();

    builder
        .upsert_column("number_array")
        .unwrap()
        .add_all(&number_arrays)
        .unwrap();

    builder
        .upsert_column("text_array")
        .unwrap()
        .add_all(&text_arrays)
        .unwrap();

    builder
        .upsert_column("nullable_text_array")
        .unwrap()
        .add_all(text_arrays.iter().map(|array| {
            array
                .iter()
                .enumerate()
                .map(|(i, text)| (i % 2 == 0).then_some(text))
                .collect::<Vec<_>>()
        }))
        .unwrap();

    builder
        .upsert_column("number_text_map")
        .unwrap()
        .add_all(&maps)
        .unwrap();

    let block_in = builder.build().unwrap();

    let mut insert = client
        .insert_native("foo")
        // Generates coverage for `InsertNative::with_timeouts()`
        // and also tests the case when *both* timeouts are set (which `insert_formatted.rs` misses)
        .with_timeouts(
            Some(Duration::from_secs(600)),
            Some(Duration::from_secs(600)),
        );

    insert.write(&block_in).await.unwrap();

    insert.end().await.unwrap();

    // This is going to be similar to the `fetch_native` test,
    // but we need to make sure the data actually got inserted correctly.
    let mut cursor = client.query("SELECT * FROM foo").fetch_native().unwrap();

    let Some(block_out) = cursor.next().await.unwrap() else {
        assert_eq!(num_rows, 0, "expected block, got none");
        return;
    };

    assert_eq!(block_out.num_rows(), block_in.num_rows());
    assert_eq!(block_out.columns().len(), block_in.columns().len());

    let mut number_iter = block_out["number"].iter::<i32>().unwrap();

    for (res, &expected) in number_iter.by_ref().zip(&numbers) {
        let actual = res.unwrap();
        assert_eq!(actual, expected);
    }

    if let Some(res) = number_iter.next() {
        panic!("unexpected value {res:?}");
    }

    let mut text_iter = block_out["text"].iter::<String>().unwrap();

    for (res, expected) in text_iter.by_ref().zip(&texts) {
        let actual = res.unwrap();
        assert_eq!(actual, *expected);
    }

    if let Some(res) = text_iter.next() {
        panic!("unexpected value {res:?}");
    }

    let mut nullable_text_iter = block_out["nullable_text"].iter::<Option<String>>().unwrap();

    for (i, (res, expected)) in nullable_text_iter.by_ref().zip(&texts).enumerate() {
        let actual = res.unwrap();

        assert_eq!(actual.as_ref(), (i % 2 != 0).then_some(expected));
    }

    if let Some(res) = nullable_text_iter.next() {
        panic!("unexpected value {res:?}");
    }

    let mut number_tuple_iter = block_out["number_tuple"].iter::<(u32, i64)>().unwrap();

    for (res, &expected) in number_tuple_iter.by_ref().zip(&numbers) {
        let (actual_uint32, actual_int64) = res.unwrap();
        assert_eq!(actual_uint32, expected as u32);
        assert_eq!(actual_int64, expected as i64);
    }

    if let Some(res) = number_tuple_iter.next() {
        panic!("unexpected value {res:?}");
    }

    let mut number_text_tuple_iter = block_out["number_text_tuple"]
        .iter::<(i64, String)>()
        .unwrap();

    for ((res, &expected_number), expected_text) in
        number_text_tuple_iter.by_ref().zip(&numbers).zip(&texts)
    {
        let (actual_number, actual_text) = res.unwrap();
        assert_eq!(actual_number, expected_number as i64);
        assert_eq!(actual_text, *expected_text);
    }

    if let Some(res) = number_text_tuple_iter.next() {
        panic!("unexpected value {res:?}");
    }

    let mut nullable_tuple_iter = block_out["nullable_tuple"]
        .iter::<(Option<i64>, Option<String>)>()
        .unwrap();

    for ((res, &expected_number), expected_text) in
        nullable_tuple_iter.by_ref().zip(&numbers).zip(&texts)
    {
        let (actual_number, actual_text) = res.unwrap();

        assert_eq!(
            actual_number,
            (expected_number % 2 == 0).then_some(expected_number as i64)
        );
        assert_eq!(
            actual_text.as_ref(),
            (expected_number % 2 != 0).then_some(expected_text)
        );
    }

    if let Some(res) = nullable_tuple_iter.next() {
        panic!("unexpected value {res:?}");
    }

    let mut tuple_nullable_iter = block_out["tuple_nullable"]
        .iter::<Option<(i32, String)>>()
        .unwrap();

    for ((res, &expected_number), expected_text) in
        tuple_nullable_iter.by_ref().zip(&numbers).zip(&texts)
    {
        let actual = res.unwrap();

        if expected_number % 2 == 0 {
            let (actual_number, actual_text) = actual.unwrap();

            assert_eq!(actual_number, expected_number);
            assert_eq!(actual_text, *expected_text);
        } else {
            assert_eq!(actual, None);
        }
    }

    if let Some(res) = tuple_nullable_iter.next() {
        panic!("unexpected value {res:?}");
    }

    let mut lc_text_iter = block_out["low_cardinality_text"].iter::<String>().unwrap();

    for (res, expected) in lc_text_iter.by_ref().zip(&texts) {
        let actual = res.unwrap();
        assert_eq!(actual, *expected);
    }

    if let Some(res) = lc_text_iter.next() {
        panic!("unexpected value {res:?}");
    }

    let mut number_array_iter = block_out["number_array"].iter::<Vec<i32>>().unwrap();

    for (res, expected) in number_array_iter.by_ref().zip(&number_arrays) {
        let actual = res.unwrap();
        assert_eq!(actual, *expected);
    }

    if let Some(res) = number_array_iter.next() {
        panic!("unexpected value {res:?}");
    }

    let mut text_array_iter = block_out["text_array"].iter::<Vec<String>>().unwrap();

    for (res, expected) in text_array_iter.by_ref().zip(&text_arrays) {
        let actual = res.unwrap();
        assert_eq!(actual, *expected);
    }

    if let Some(res) = text_array_iter.next() {
        panic!("unexpected value {res:?}");
    }

    let mut nullable_text_array_iter = block_out["nullable_text_array"]
        .iter::<Vec<Option<String>>>()
        .unwrap();

    for (res, expected) in nullable_text_array_iter.by_ref().zip(&text_arrays) {
        let actual = res.unwrap();

        for (i, (actual, expected)) in actual.iter().zip(expected).enumerate() {
            assert_eq!(actual.as_ref(), (i % 2 == 0).then_some(expected));
        }
    }

    if let Some(res) = nullable_text_array_iter.next() {
        panic!("unexpected value {res:?}");
    }

    let mut map_iter = block_out["number_text_map"]
        .iter::<HashMap<i32, String>>()
        .unwrap();

    for (res, expected) in map_iter.by_ref().zip(&maps) {
        let actual = res.unwrap();

        assert_eq!(actual, *expected);
    }

    if let Some(res) = map_iter.next() {
        panic!("unexpected value {res:?}");
    }
}
