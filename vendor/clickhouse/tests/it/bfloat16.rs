use clickhouse::Row;
use clickhouse::types::BFloat16;
use serde::{Deserialize, Serialize};

#[tokio::test]
async fn bfloat16() {
    let client = prepare_database!();

    #[derive(Debug, PartialEq, Serialize, Deserialize, Row, Clone)]
    struct MyRow {
        val: BFloat16,
        val_opt: Option<BFloat16>,
    }

    client
        .query(
            "
            CREATE TABLE test(
                val     BFloat16,
                val_opt Nullable(BFloat16)
            ) ENGINE = MergeTree ORDER BY val
        ",
        )
        .execute()
        .await
        .unwrap();

    // NaN values are intentionally omitted from this test.
    // IEEE 754 defines NaN as unequal to itself (NaN != NaN),
    let values = [
        0.0f32,
        -0.0,
        1.0,
        -1.0,
        -1.5,
        0.5,
        100.0,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::MIN_POSITIVE,
        BFloat16::MAX.to_f32(),
        BFloat16::MIN.to_f32(),
    ];

    let original_rows = values
        .into_iter()
        .map(|v| MyRow {
            val: BFloat16::from_f32(v),
            val_opt: Some(BFloat16::from_f32(v)),
        })
        .collect::<Vec<_>>();

    let mut insert = client.insert::<MyRow>("test").await.unwrap();
    for row in &original_rows {
        insert.write(row).await.unwrap();
    }
    insert.end().await.unwrap();

    // Fetch and compare by bits to avoid float comparison pitfalls
    let mut rows = client
        .query("SELECT ?fields FROM test WHERE val_opt IS NOT NULL ORDER BY val")
        .fetch_all::<MyRow>()
        .await
        .unwrap();

    let mut expected = original_rows.clone();
    rows.sort_by_key(|r| r.val.to_bits());
    expected.sort_by_key(|r| r.val.to_bits());

    assert_eq!(rows.len(), expected.len(), "row count mismatch");

    for (got, exp) in rows.iter().zip(expected.iter()) {
        assert_eq!(
            got.val.to_bits(),
            exp.val.to_bits(),
            "val bits mismatch: got {got:?}, expected {exp:?}"
        );
        assert_eq!(
            got.val_opt.map(|v| v.to_bits()),
            exp.val_opt.map(|v| v.to_bits()),
            "val_opt bits mismatch"
        );
    }

    // NULL handling: insert a row with val_opt = NULL
    let null_val = BFloat16::from_f32(42.0);
    let mut insert = client.insert::<MyRow>("test").await.unwrap();
    insert
        .write(&MyRow {
            val: null_val,
            val_opt: None,
        })
        .await
        .unwrap();
    insert.end().await.unwrap();

    let null_row = client
        .query("SELECT ?fields FROM test WHERE val_opt IS NULL")
        .fetch_one::<MyRow>()
        .await
        .unwrap();

    assert_eq!(null_row.val_opt, None);
    assert_eq!(null_row.val.to_bits(), null_val.to_bits());

    // Verify ClickHouse agrees on the value via toString() for a few values
    #[derive(Deserialize, Row)]
    struct ValStr {
        s: String,
    }

    for (input, expected_f32) in [(1.0f32, 1.0f32), (-1.0, -1.0), (0.5, 0.5)] {
        let ValStr { s } = client
            .query("SELECT toString(val) AS s FROM test WHERE val = toBFloat16(?)")
            .bind(input)
            .fetch_one::<ValStr>()
            .await
            .unwrap();

        let ch_val: f32 = s.parse().unwrap();
        assert!(
            (ch_val - expected_f32).abs() < 1e-2,
            "ClickHouse toString mismatch for {input}: got {ch_val}"
        );
    }

    // from_f64 round-trip
    let bf_from_f64 = BFloat16::from_f64(1.0f64);
    assert_eq!(bf_from_f64.to_bits(), BFloat16::from_f32(1.0f32).to_bits());
}
