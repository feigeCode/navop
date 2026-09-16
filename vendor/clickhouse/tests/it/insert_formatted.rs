use crate::{SimpleRow, create_simple_table, fetch_rows};
use bytes::{Buf, Bytes};
use clickhouse::error::Error;
use clickhouse::{Client, Compression};
use clickhouse_macros::Row;
use serde::Deserialize;
use std::cmp;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;

/// First 1000 records of the [NYC taxi dataset] in `TabSeparated` format.
///
/// [NYC taxi dataset]: https://clickhouse.com/docs/getting-started/example-datasets/nyc-taxi#create-the-table-trips
const TAXI_DATA_TSV: &[u8] = include_bytes!("fixtures/nyc-taxi_trips_0_head_1000.tsv");

#[tokio::test]
async fn empty_insert() {
    let table_name = "insert_empty";
    let query_id = uuid::Uuid::new_v4().to_string();

    let client = prepare_database!();
    create_simple_table(&client, table_name).await;

    let insert = client
        .insert_formatted_with(format!("INSERT INTO {table_name} FORMAT TabSeparated"))
        .with_setting("query_id", query_id);
    insert.end().await.unwrap();

    let rows = fetch_rows::<SimpleRow>(&client, table_name).await;
    assert!(rows.is_empty())
}

#[tokio::test]
async fn insert() {
    let client = prepare_database!()
        // Separate test for compression
        .with_compression(Compression::None);

    create_table(&client).await;

    let bytes = Bytes::from_static(TAXI_DATA_TSV);

    let mut insert =
        client.insert_formatted_with("INSERT INTO nyc_taxi_trips_small FORMAT TabSeparated");

    insert.send(bytes).await.unwrap();

    insert.end().await.unwrap();

    verify_insert(&client).await;
}

#[tokio::test]
async fn insert_small_chunks() {
    let client = prepare_database!()
        // Separate test for compression
        .with_compression(Compression::None);

    create_table(&client).await;

    let mut bytes = Bytes::from_static(TAXI_DATA_TSV);

    let mut insert =
        client.insert_formatted_with("INSERT INTO nyc_taxi_trips_small FORMAT TabSeparated");

    while !bytes.is_empty() {
        let chunk = bytes.split_to(cmp::min(16, bytes.len()));
        insert.send(chunk).await.unwrap();
    }

    insert.end().await.unwrap();

    verify_insert(&client).await;
}

#[tokio::test]
#[cfg(feature = "lz4")]
async fn insert_compressed_lz4() {
    use clickhouse::insert_formatted::CompressedData;

    let client = prepare_database!()
        // `test-util` turns compression off
        .with_compression(Compression::Lz4);

    create_table(&client).await;

    let data = CompressedData::new(TAXI_DATA_TSV, Compression::Lz4).unwrap();

    let mut insert =
        client.insert_formatted_with("INSERT INTO nyc_taxi_trips_small FORMAT TabSeparated");

    insert.send_compressed(data).await.unwrap();

    insert.end().await.unwrap();

    verify_insert(&client).await;
}

#[tokio::test]
#[cfg(feature = "zstd")]
async fn insert_compressed_zstd() {
    use clickhouse::insert_formatted::CompressedData;

    let client = prepare_database!()
        // `test-util` turns compression off
        .with_compression(Compression::zstd());

    create_table(&client).await;

    let data = CompressedData::new(TAXI_DATA_TSV, Compression::zstd()).unwrap();

    let mut insert =
        client.insert_formatted_with("INSERT INTO nyc_taxi_trips_small FORMAT TabSeparated");

    insert.send_compressed(data).await.unwrap();

    insert.end().await.unwrap();

    verify_insert(&client).await;
}

#[tokio::test]
async fn insert_send_timeout() {
    // Simulate a timeout by attempting to send a request to a server
    // that's listening but not answering.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let local_addr = listener.local_addr().unwrap();

    let client = Client::default().with_url(format!("http://{local_addr}"));

    let bytes = Bytes::from_static(TAXI_DATA_TSV);

    let send_timeout = Duration::from_millis(100);

    let mut insert = client
        .insert_formatted_with("INSERT INTO nyc_taxi_trips_small FORMAT TabSeparated")
        .with_timeouts(Some(send_timeout), None);

    for _ in 0..1024 {
        // First send actually initiates the request,
        // then it might take a few more sends to fill up the TCP send window.
        if let Err(e) = insert.send(bytes.clone()).await {
            assert!(
                matches!(e, Error::TimedOut),
                "expected `Error::TimedOut`, got {e:?}"
            );
            return;
        }
    }

    unreachable!("BUG: `send_timeout` should have triggered by now!");
}

#[tokio::test]
async fn insert_end_timeout() {
    // Simulate a timeout by attempting to send a request to a server
    // that's listening but not answering.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let local_addr = listener.local_addr().unwrap();

    let client = Client::default().with_url(format!("http://{local_addr}"));

    let bytes = Bytes::from_static(TAXI_DATA_TSV);

    let end_timeout = Duration::from_millis(100);

    let mut insert = client
        .insert_formatted_with("INSERT INTO nyc_taxi_trips_small FORMAT TabSeparated")
        .with_timeouts(None, Some(end_timeout));

    // First send actually initiates the request.
    if let Err(e) = insert.send(bytes.clone()).await {
        assert!(
            matches!(e, Error::TimedOut),
            "expected `Err(TimedOut)`, got {e:?}"
        );
    }

    let res = insert.end().await;

    assert!(
        matches!(res, Err(Error::TimedOut)),
        "expected `Err(TimedOut)`, got {res:?}"
    );
}

#[tokio::test]
async fn insert_buffered() {
    let client = prepare_database!();

    create_table(&client).await;

    let mut data = TAXI_DATA_TSV;
    let capacity = 8192;

    let mut insert = client
        .insert_formatted_with("INSERT INTO nyc_taxi_trips_small FORMAT TabSeparated")
        .buffered_with_capacity(capacity);

    // Cycle different read sizes in an attempt to break the buffer
    let read_sizes = [1, 10, 100, 1000, 1024, capacity];

    while !data.is_empty() {
        for size in read_sizes {
            if data.is_empty() {
                break;
            }

            let written = insert
                .write(&data[..cmp::min(size, data.len())])
                .await
                .unwrap();
            assert_ne!(written, 0);
            data = &data[written..];
        }
    }

    insert.end().await.unwrap();

    verify_insert(&client).await;
}

#[tokio::test]
async fn insert_buffered_zero_capacity() {
    let client = prepare_database!();

    create_table(&client).await;

    let mut data = TAXI_DATA_TSV;

    let mut insert = client
        .insert_formatted_with("INSERT INTO nyc_taxi_trips_small FORMAT TabSeparated")
        .buffered_with_capacity(0);

    // Cycle different read sizes in an attempt to break the buffer
    let read_sizes = [1, 10, 100, 1000, 1024];

    while !data.is_empty() {
        for size in read_sizes {
            if data.is_empty() {
                break;
            }

            let written = insert
                .write(&data[..cmp::min(size, data.len())])
                .await
                .unwrap();
            assert_ne!(written, 0);
            data = &data[written..];
        }
    }

    insert.end().await.unwrap();

    verify_insert(&client).await;
}

#[tokio::test]
async fn insert_write_buffered() {
    let client = prepare_database!();

    create_table(&client).await;

    let mut data = TAXI_DATA_TSV;

    let capacity = 8192;

    let mut insert = client
        .insert_formatted_with("INSERT INTO nyc_taxi_trips_small FORMAT TabSeparated")
        .buffered_with_capacity(capacity);

    // Cycle different read sizes in an attempt to break the buffer
    let read_sizes = [1, 10, 100, 1000, 1024, capacity];

    while !data.is_empty() {
        for size in read_sizes {
            if data.is_empty() {
                break;
            }

            let write_len = cmp::min(size, data.len());

            insert.write_buffered(&data[..write_len]);

            data = &data[write_len..];
        }

        insert.flush().await.unwrap();
    }

    insert.end().await.unwrap();

    verify_insert(&client).await;
}

#[tokio::test]
async fn insert_buffered_async_write() {
    let client = prepare_database!();

    create_table(&client).await;

    let mut data = TAXI_DATA_TSV;
    let capacity = 8192;

    let mut insert = client
        .insert_formatted_with("INSERT INTO nyc_taxi_trips_small FORMAT TabSeparated")
        .buffered_with_capacity(capacity);

    // Cycle different read sizes in an attempt to break the buffer
    let read_sizes = [1, 10, 100, 1000, 1024, capacity];

    while !data.is_empty() {
        for size in read_sizes {
            if data.is_empty() {
                break;
            }

            insert
                .write_buf(&mut Buf::take(&mut data, size))
                .await
                .unwrap();
        }
    }

    AsyncWriteExt::flush(&mut insert).await.unwrap();
    AsyncWriteExt::shutdown(&mut insert).await.unwrap();

    verify_insert(&client).await;
}

async fn create_table(client: &Client) {
    client
        .query(
            r#"
            CREATE TABLE IF NOT EXISTS nyc_taxi_trips_small (
                trip_id             UInt32,
                pickup_datetime     DateTime,
                dropoff_datetime    DateTime,
                pickup_longitude    Nullable(Float64),
                pickup_latitude     Nullable(Float64),
                dropoff_longitude   Nullable(Float64),
                dropoff_latitude    Nullable(Float64),
                passenger_count     UInt8,
                trip_distance       Float32,
                fare_amount         Float32,
                extra               Float32,
                tip_amount          Float32,
                tolls_amount        Float32,
                total_amount        Float32,
                payment_type        Enum('CSH' = 1, 'CRE' = 2, 'NOC' = 3, 'DIS' = 4, 'UNK' = 5),
                pickup_ntaname      LowCardinality(String),
                dropoff_ntaname     LowCardinality(String)
            )
            ENGINE = MergeTree
            PRIMARY KEY (pickup_datetime, dropoff_datetime)
            "#,
        )
        .execute()
        .await
        .unwrap();
}

async fn verify_insert(client: &Client) {
    #[derive(Row, Deserialize)]
    struct Results {
        min_trip_id: u32,
        max_trip_id: u32,
        median_trip_distance: f64,
        count: u64,
    }

    let results = client
        .query(
            "SELECT \
                min(trip_id) min_trip_id, \
                max(trip_id) max_trip_id, \
                median(trip_distance) median_trip_distance,\
                count(*) count \
            FROM nyc_taxi_trips_small",
        )
        .fetch_one::<Results>()
        .await
        .unwrap();

    assert_eq!(results.min_trip_id, 1199999902);
    assert_eq!(results.max_trip_id, 1200019742);
    assert_eq!(results.median_trip_distance, 1.7999999523162842);
    assert_eq!(results.count, 1000);
}
