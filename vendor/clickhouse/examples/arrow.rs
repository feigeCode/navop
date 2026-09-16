use std::sync::Arc;

use arrow::array::{Int32Array, RecordBatch, StringArray};
use arrow::datatypes::{DataType, Field, Schema};

use clickhouse::{Client, error::Result};
use clickhouse_ext_arrow::{ArrowClientExt, ArrowQueryExt};

// This example demonstrates the `clickhouse-ext-arrow` extension traits:
//
//   - `ArrowClientExt::insert_arrow()`: write Arrow `RecordBatch`es to a table.
//   - `ArrowQueryExt::fetch_arrow()`: stream the result of a `SELECT` as Arrow.
//
// See also: tests/it/arrow.rs

async fn ddl(client: &Client) -> Result<()> {
    client
        .query(
            "CREATE OR REPLACE TABLE chrs_arrow_example (bar Int32, baz String) \
             ENGINE = MergeTree ORDER BY bar",
        )
        .execute()
        .await
}

// `insert_arrow()` writes Arrow `RecordBatch`es to a target table.
//
// The Arrow field names are used as the target table's column list for the
// generated `INSERT ... FORMAT ArrowStream` query.
async fn insert(client: &Client) -> Result<()> {
    let schema = Arc::new(Schema::new(vec![
        Field::new("bar", DataType::Int32, false),
        Field::new("baz", DataType::Utf8, false),
    ]));

    let batches = [
        RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(Int32Array::from(vec![1, 2])),
                Arc::new(StringArray::from(vec!["one", "two"])),
            ],
        )
        .unwrap(),
        RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Int32Array::from(vec![3])),
                Arc::new(StringArray::from(vec!["three"])),
            ],
        )
        .unwrap(),
    ];

    let mut insert = client.insert_arrow("chrs_arrow_example")?;
    for batch in &batches {
        insert.write(batch).await?;
    }
    insert.end().await?;

    Ok(())
}

// `fetch_arrow()` streams the response as a sequence of Arrow `RecordBatch`es.
async fn select(client: &Client) -> Result<()> {
    let mut cursor = client
        .query("SELECT bar, baz FROM chrs_arrow_example ORDER BY bar")
        .fetch_arrow()?;

    while let Some(batch) = cursor.next().await? {
        println!("batch with {} row(s):\n{batch:?}", batch.num_rows());
    }

    Ok(())
}

// Convenience helper: collect the whole response into a single `RecordBatch`.
// Allocates a fresh batch to concatenate the streamed pieces; use `select()`
// above when you want to process batches as they arrive instead.
async fn select_merged(client: &Client) -> Result<()> {
    let batch = client
        .query("SELECT bar, baz FROM chrs_arrow_example ORDER BY bar")
        .fetch_arrow()?
        .collect_merged()
        .await?;

    println!("merged batch with {} row(s):\n{batch:?}", batch.num_rows());

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let client = Client::default().with_url("http://localhost:8123");

    ddl(&client).await?;
    insert(&client).await?;
    select(&client).await?;
    select_merged(&client).await?;

    Ok(())
}
