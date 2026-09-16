// https://github.com/ClickHouse/clickhouse-rs/issues/392
// Variant is implicitly nullable: 0xFF discriminator = NULL.

use serde::{Deserialize, Serialize};

use clickhouse::Row;

#[tokio::test]
async fn variant_null_values() {
    let client = prepare_database!();

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    enum SimpleVariant {
        String(String),
        UInt64(u64),
    }

    #[derive(Debug, PartialEq, Row, Serialize, Deserialize)]
    struct MyRow {
        id: u32,
        var: Option<SimpleVariant>,
    }

    client
        .query(
            "
            CREATE OR REPLACE TABLE test_var_null
            (
                `id` UInt32,
                `var` Variant(String, UInt64)
            )
            ENGINE = MergeTree
            ORDER BY id",
        )
        .with_setting("allow_experimental_variant_type", "1")
        .execute()
        .await
        .unwrap();

    // insert via SQL so we get actual NULLs in the Variant column
    client
        .query("INSERT INTO test_var_null VALUES (1, 'hello'), (2, NULL), (3, 42), (4, NULL)")
        .execute()
        .await
        .unwrap();

    let rows = client
        .query("SELECT ?fields FROM test_var_null ORDER BY id")
        .fetch_all::<MyRow>()
        .await
        .unwrap();

    assert_eq!(
        rows,
        vec![
            MyRow {
                id: 1,
                var: Some(SimpleVariant::String("hello".to_string())),
            },
            MyRow { id: 2, var: None },
            MyRow {
                id: 3,
                var: Some(SimpleVariant::UInt64(42)),
            },
            MyRow { id: 4, var: None },
        ]
    );
}

// verifies that a NULL Variant value can be read without a table,
// using an inline cast expression (e.g. `NULL::Variant(String, UInt64)`).
#[tokio::test]
async fn variant_null_cast() {
    let client = prepare_database!();

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    enum SimpleVariant {
        String(String),
        UInt64(u64),
    }

    #[derive(Debug, PartialEq, Row, Deserialize)]
    struct Wrapper {
        #[serde(rename = "v")]
        v: Option<SimpleVariant>,
    }

    let result = client
        .query("SELECT NULL::Variant(String, UInt64) AS v")
        .with_setting("allow_experimental_variant_type", "1")
        .fetch_one::<Wrapper>()
        .await
        .unwrap();

    assert_eq!(result.v, None);
}

/// Regression test for nested Variant inside an Array.
/// Before the inner-validator `null_encoding` impl, `Vec<Option<MyVariant>>`
/// over `Array(Variant(...))` would consume a discriminator byte as a
/// Nullable flag and corrupt the input stream.
#[tokio::test]
async fn variant_null_in_array() {
    let client = prepare_database!();

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    enum SimpleVariant {
        String(String),
        UInt64(u64),
    }

    #[derive(Debug, PartialEq, Row, Deserialize)]
    struct MyRow {
        id: u32,
        vars: Vec<Option<SimpleVariant>>,
    }

    client
        .query(
            "
            CREATE OR REPLACE TABLE test_var_null_array
            (
                `id` UInt32,
                `vars` Array(Variant(String, UInt64))
            )
            ENGINE = MergeTree
            ORDER BY id",
        )
        .with_setting("allow_experimental_variant_type", "1")
        .execute()
        .await
        .unwrap();

    // Mix NULL and non-NULL discriminators in the same array so the
    // deserializer has to switch paths element-by-element.
    client
        .query(
            "INSERT INTO test_var_null_array VALUES \
             (1, ['hello', NULL, 42, NULL, 'world'])",
        )
        .execute()
        .await
        .unwrap();

    let rows = client
        .query("SELECT ?fields FROM test_var_null_array ORDER BY id")
        .fetch_all::<MyRow>()
        .await
        .unwrap();

    assert_eq!(
        rows,
        vec![MyRow {
            id: 1,
            vars: vec![
                Some(SimpleVariant::String("hello".to_string())),
                None,
                Some(SimpleVariant::UInt64(42)),
                None,
                Some(SimpleVariant::String("world".to_string())),
            ],
        }]
    );
}

/// Regression test for nested Variant inside a Tuple.
/// Exercises the `Tuple(elements)` branch of the inner validator's
/// `null_encoding`.
#[tokio::test]
async fn variant_null_in_tuple() {
    let client = prepare_database!();

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    enum SimpleVariant {
        String(String),
        UInt64(u64),
    }

    #[derive(Debug, PartialEq, Row, Deserialize)]
    struct MyRow {
        id: u32,
        pair: (u32, Option<SimpleVariant>),
    }

    client
        .query(
            "
            CREATE OR REPLACE TABLE test_var_null_tuple
            (
                `id` UInt32,
                `pair` Tuple(UInt32, Variant(String, UInt64))
            )
            ENGINE = MergeTree
            ORDER BY id",
        )
        .with_setting("allow_experimental_variant_type", "1")
        .execute()
        .await
        .unwrap();

    client
        .query(
            "INSERT INTO test_var_null_tuple VALUES \
             (1, (10, 'hello')), (2, (20, NULL)), (3, (30, 42))",
        )
        .execute()
        .await
        .unwrap();

    let rows = client
        .query("SELECT ?fields FROM test_var_null_tuple ORDER BY id")
        .fetch_all::<MyRow>()
        .await
        .unwrap();

    assert_eq!(
        rows,
        vec![
            MyRow {
                id: 1,
                pair: (10, Some(SimpleVariant::String("hello".to_string()))),
            },
            MyRow {
                id: 2,
                pair: (20, None),
            },
            MyRow {
                id: 3,
                pair: (30, Some(SimpleVariant::UInt64(42))),
            },
        ]
    );
}
