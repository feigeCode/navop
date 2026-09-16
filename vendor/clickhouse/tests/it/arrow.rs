use crate::get_client;
use arrow::array::types::Int32Type;
use arrow::array::{PrimitiveArray, RecordBatch, StringArray, create_array, record_batch};
use arrow::datatypes::{DataType, Field, Schema};
use arrow_schema::ArrowError;
use clickhouse::error::Error;
use clickhouse_ext_arrow::_priv::CARGO_PKG_VERSION as ARROW_EXT_VER;
use clickhouse_ext_arrow::{ArrowClientExt, ArrowQueryExt};
use std::env::consts::OS;
use std::sync::Arc;

use crate::user_agent::{PKG_VER, RUST_VER};

#[tokio::test]
async fn basic_query() {
    // `record_batch!()` sets all columns to be nullable
    let expected = RecordBatch::try_new(
        Schema::new(vec![
            Field::new("number", DataType::UInt64, false),
            Field::new("name", DataType::Utf8, false),
        ])
        .into(),
        vec![
            create_array!(UInt64, [0, 1, 2, 3, 4, 5, 6, 7, 8, 9]),
            create_array!(
                Utf8,
                [
                    "test_0", "test_1", "test_2", "test_3", "test_4", "test_5", "test_6", "test_7",
                    "test_8", "test_9"
                ]
            ),
        ],
    )
    .unwrap();

    let client = get_client();

    let mut cursor = client
        .query("SELECT number, 'test_' || number as name FROM system.numbers LIMIT 10")
        .fetch_arrow()
        .unwrap();

    let actual = cursor.collect_merged().await.unwrap();

    assert_eq!(actual, expected);

    // An empty result should still include the schema.
    let mut cursor = client
        .query("SELECT number, 'test_' || number as name FROM system.numbers LIMIT 0")
        .fetch_arrow()
        .unwrap();

    let actual = cursor.collect_merged().await.unwrap();

    assert_eq!(actual, RecordBatch::new_empty(expected.schema()));
}

#[tokio::test]
async fn insert() {
    let client = prepare_database!();

    client
        .query(
            "CREATE TABLE arrow_insert_test(bar Int32, baz String) ENGINE = MergeTree ORDER BY bar",
        )
        .execute()
        .await
        .unwrap();

    let batch_size = 100;
    let num_batches = 100;
    let mut next_id = 1..;

    let mut batches = Vec::new();
    let schema = Arc::new(Schema::new(vec![
        Field::new("bar", DataType::Int32, false),
        Field::new("baz", DataType::Utf8, false),
    ]));

    for batch in 1..=num_batches {
        let bars: PrimitiveArray<Int32Type> = (0..batch_size)
            .zip(&mut next_id)
            .map(|(_, id)| id)
            .collect();

        let bazzes: StringArray = bars
            .iter()
            .filter_map(|bar| {
                let bar = bar?;
                Some(format!("batch_{batch}_bar_{bar}"))
            })
            .collect::<Vec<String>>()
            .into();

        batches.push(
            RecordBatch::try_new(schema.clone(), vec![Arc::new(bars), Arc::new(bazzes)]).unwrap(),
        );
    }

    let mut insert = client.insert_arrow("arrow_insert_test").unwrap();

    for batch in batches {
        insert.write(&batch).await.unwrap();
    }

    insert.end().await.unwrap();

    let result = client
        .query(
            "SELECT \
         count(*) AS row_count, \
         first_value(bar) AS min_bar, \
         first_value(baz) AS min_baz,
         last_value(bar) AS max_bar, \
         last_value(baz) AS max_baz \
         FROM (SELECT * FROM arrow_insert_test ORDER BY bar)",
        )
        .fetch_arrow()
        .unwrap()
        .collect_merged()
        .await
        .unwrap();

    let expected_count = batch_size * num_batches;

    let expected = RecordBatch::try_new(
        Schema::new(vec![
            Field::new("row_count", DataType::UInt64, false),
            Field::new("min_bar", DataType::Int32, false),
            Field::new("min_baz", DataType::Utf8, false),
            Field::new("max_bar", DataType::Int32, false),
            Field::new("max_baz", DataType::Utf8, false),
        ])
        .into(),
        vec![
            create_array!(UInt64, [expected_count]),
            create_array!(Int32, [1]),
            create_array!(Utf8, ["batch_1_bar_1"]),
            create_array!(Int32, [expected_count as i32]),
            create_array!(Utf8, ["batch_100_bar_10000"]),
        ],
    )
    .unwrap();

    assert_eq!(result, expected);
}

#[tokio::test]
async fn insert_custom() {
    let client = prepare_database!();

    client
        .query(
            "CREATE TABLE arrow_custom_insert_test(bar Int32, baz String) ENGINE = MergeTree ORDER BY bar",
        )
        .execute()
        .await
        .unwrap();

    let batch_size = 100;
    let num_batches = 100;
    let mut next_id = 1..;

    let mut batches = Vec::new();
    let schema = Arc::new(Schema::new(vec![
        Field::new("bar", DataType::Int32, false),
        Field::new("baz", DataType::Utf8, false),
    ]));

    for batch in 1..=num_batches {
        let bars: PrimitiveArray<Int32Type> = (0..batch_size)
            .zip(&mut next_id)
            .map(|(_, id)| id)
            .collect();

        let bazzes: StringArray = bars
            .iter()
            .filter_map(|bar| {
                let bar = bar?;
                Some(format!("batch_{batch}_bar_{bar}"))
            })
            .collect::<Vec<String>>()
            .into();

        batches.push(
            RecordBatch::try_new(schema.clone(), vec![Arc::new(bars), Arc::new(bazzes)]).unwrap(),
        );
    }

    let mut insert = client
        .insert_arrow_with("INSERT INTO arrow_custom_insert_test(bar, baz) FORMAT ArrowStream");

    for batch in batches {
        insert.write(&batch).await.unwrap();
    }

    insert.end().await.unwrap();

    let result = client
        .query(
            "SELECT \
         count(*) AS row_count, \
         first_value(bar) AS min_bar, \
         first_value(baz) AS min_baz,
         last_value(bar) AS max_bar, \
         last_value(baz) AS max_baz \
         FROM (SELECT * FROM arrow_custom_insert_test ORDER BY bar)",
        )
        .fetch_arrow()
        .unwrap()
        .collect_merged()
        .await
        .unwrap();

    let expected_count = batch_size * num_batches;

    let expected = RecordBatch::try_new(
        Schema::new(vec![
            Field::new("row_count", DataType::UInt64, false),
            Field::new("min_bar", DataType::Int32, false),
            Field::new("min_baz", DataType::Utf8, false),
            Field::new("max_bar", DataType::Int32, false),
            Field::new("max_baz", DataType::Utf8, false),
        ])
        .into(),
        vec![
            create_array!(UInt64, [expected_count]),
            create_array!(Int32, [1]),
            create_array!(Utf8, ["batch_1_bar_1"]),
            create_array!(Int32, [expected_count as i32]),
            create_array!(Utf8, ["batch_100_bar_10000"]),
        ],
    )
    .unwrap();

    assert_eq!(result, expected);
}

#[tokio::test]
async fn query_empty_response() {
    let client = get_client();

    // Don't error if the response doesn't return anything
    let mut cursor = client.query("SYSTEM FLUSH LOGS").fetch_arrow().unwrap();

    let batch = cursor.next().await.unwrap();
    assert_eq!(batch, None);
    assert_eq!(cursor.schema(), None);
}

#[tokio::test]
async fn ext_arrow_adds_user_agent() {
    let client = prepare_database!();

    client.query("CREATE TABLE arrow_product_info_test(foo Int32, bar String) ENGINE = MergeTree ORDER BY foo")
        .execute()
        .await
        .unwrap();

    client
        .query(
            "INSERT INTO arrow_product_info_test(foo, bar)\n\
         SELECT number, 'test_' || number FROM system.numbers LIMIT 10",
        )
        .execute()
        .await
        .unwrap();

    let query = "SELECT * FROM arrow_product_info_test";

    let mut cursor = client.query(query).fetch_arrow().unwrap();

    let records = cursor.collect_merged().await.unwrap();

    assert_eq!(records.num_rows(), 10);

    crate::flush_query_log(&client).await;

    let recorded_user_agents = client
        .query(
            "
            SELECT http_user_agent
            FROM system.query_log
            WHERE type = 'QueryFinish'
            AND (
              query LIKE {query:String}
            )
            ORDER BY event_time_microseconds DESC
            LIMIT 1
            ",
        )
        .param("query", query)
        .fetch_all::<String>()
        .await
        .unwrap();

    let expected_user_agent = format!(
        "clickhouse-ext-arrow/{ARROW_EXT_VER} clickhouse-rs/{PKG_VER} (lv:rust/{RUST_VER}; os:{OS})"
    );

    assert_eq!(recorded_user_agents.len(), 1);
    assert_eq!(recorded_user_agents[0], expected_user_agent);
}

#[tokio::test]
async fn schema_mismatch_throws_error() {
    let client = prepare_database!();

    client.query("CREATE TABLE arrow_schema_mismatch_throws_error(foo Int32, bar String) ENGINE = MergeTree ORDER BY foo")
        .execute()
        .await
        .unwrap();

    let record_batch = record_batch!(
        ("alpha", UInt64, [0, 1, 2, 3, 4]),
        ("beta", Float32, [0.0, 1.1, 2.2, 3.3, 4.4])
    )
    .unwrap();

    let mut insert = client
        .insert_arrow("arrow_schema_mismatch_throws_error")
        .unwrap();

    insert.write(&record_batch).await.unwrap();

    let res = insert.end().await;

    let err = res.unwrap_err().to_string();

    assert!(
        err.contains("alpha"),
        "expected error to mention column name, got {err:?}"
    );
}

#[tokio::test]
async fn query_non_utf8() {
    let client = get_client();

    let mut cursor = client
        .query("SELECT 'invalid UTF-8: \\xC0\\xC1' AS test_text")
        .fetch_arrow()
        .unwrap();

    let err = cursor.collect_merged().await.unwrap_err();

    let Error::Other(e) = err else {
        panic!("unexpected error kind: {err:?}");
    };

    let e = e
        .downcast::<ArrowError>()
        .expect("unexpected Other error type");

    assert!(
        matches!(*e, ArrowError::InvalidArgumentError(_)),
        "unexpected ArrowError kind: {e:?}"
    );
}
