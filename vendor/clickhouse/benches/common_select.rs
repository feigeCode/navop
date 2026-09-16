#![allow(dead_code)]

use clickhouse::query::RowCursor;
use clickhouse::{Client, Compression, Row, RowRead};
use std::time::{Duration, Instant};
use tracing_subscriber::util::SubscriberInitExt;

pub(crate) trait WithId {
    fn id(&self) -> u64;
}
pub(crate) trait WithAccessType {
    const ACCESS_TYPE: &'static str;
}
pub(crate) trait BenchmarkRow: for<'a> RowRead<Value<'a>: WithId> + WithAccessType {}

#[macro_export]
macro_rules! impl_benchmark_row {
    ($type:ty, $id_field:ident, $access_type:literal) => {
        impl WithId for $type {
            fn id(&self) -> u64 {
                self.$id_field as u64
            }
        }

        impl WithAccessType for $type {
            const ACCESS_TYPE: &'static str = $access_type;
        }

        impl BenchmarkRow for $type {}
    };
}

#[macro_export]
macro_rules! impl_benchmark_row_no_access_type {
    ($type:ty, $id_field:ident) => {
        impl WithId for $type {
            fn id(&self) -> u64 {
                self.$id_field as u64
            }
        }

        impl WithAccessType for $type {
            const ACCESS_TYPE: &'static str = "";
        }

        impl BenchmarkRow for $type {}
    };
}

pub(crate) fn print_header(add: Option<&str>) {
    let add = add.unwrap_or("");
    println!("compress  validation  opentelemetry    elapsed  throughput  received{add}");
}

pub(crate) fn print_results<T: WithAccessType>(stats: &BenchmarkStats<u64>, opts: BenchmarkOpts) {
    fn flag_as_str(flag: bool) -> &'static str {
        if flag { "enabled" } else { "disabled" }
    }

    let BenchmarkStats {
        throughput_mbytes_sec,
        received_mbytes,
        elapsed,
        ..
    } = stats;
    let validation_mode = flag_as_str(opts.validation);
    let otel_mode = flag_as_str(opts.otel);

    let compression = match opts.compression {
        Compression::None => "none".to_string(),
        #[cfg(feature = "lz4")]
        Compression::Lz4 => "lz4".to_string(),
        #[cfg(feature = "zstd")]
        Compression::Zstd(level) => format!("zstd({level})"),
        _ => panic!("Unexpected compression mode"),
    };
    let access = if T::ACCESS_TYPE.is_empty() {
        ""
    } else {
        let access_type = T::ACCESS_TYPE;
        &format!("  {access_type:>6}")
    };
    println!(
        "{compression:>8}  {validation_mode:>10} {otel_mode:>14}  {elapsed:>9.3?}  {throughput_mbytes_sec:>4.0} MiB/s  {received_mbytes:>4.0} MiB{access}"
    );
}

pub(crate) async fn fetch_cursor<T: Row>(
    compression: Compression,
    validation: bool,
    query: &str,
) -> RowCursor<T> {
    Client::default()
        .with_compression(compression)
        .with_url("http://localhost:8123")
        .with_validation(validation)
        .query(query)
        .fetch::<T>()
        .unwrap()
}

pub(crate) async fn do_select_bench<T: BenchmarkRow>(
    query: &str,
    opts: BenchmarkOpts,
) -> BenchmarkStats<u64> {
    let _guard = match opts.otel {
        #[cfg(feature = "opentelemetry")]
        true => {
            use tracing_subscriber::layer::SubscriberExt;

            tracing_subscriber::registry()
                .with(tracing_opentelemetry::layer().with_tracer(get_tracer()))
                .set_default()
        }
        // Use a subscriber that does non-trivial work in order to test the overhead of `tracing`
        _ => tracing_subscriber::fmt().with_test_writer().set_default(),
    };

    let start = Instant::now();
    let mut cursor = fetch_cursor::<T>(opts.compression, opts.validation, query).await;

    let mut sum = 0;
    while let Some(row) = cursor.next().await.unwrap() {
        sum += row.id();
        std::hint::black_box(&row);
    }

    BenchmarkStats::new(&cursor, &start, sum)
}

pub(crate) struct BenchmarkStats<R> {
    pub(crate) throughput_mbytes_sec: f64,
    pub(crate) decoded_mbytes: f64,
    pub(crate) received_mbytes: f64,
    pub(crate) elapsed: Duration,
    // RustRover is unhappy with pub(crate)
    pub result: R,
}

impl<R> BenchmarkStats<R> {
    pub(crate) fn new<T>(cursor: &RowCursor<T>, start: &Instant, result: R) -> Self {
        let elapsed = start.elapsed();
        let dec_bytes = cursor.decoded_bytes();
        let decoded_mbytes = dec_bytes as f64 / 1024.0 / 1024.0;
        let recv_bytes = cursor.received_bytes();
        let received_mbytes = recv_bytes as f64 / 1024.0 / 1024.0;
        let throughput_mbytes_sec = decoded_mbytes / elapsed.as_secs_f64();
        BenchmarkStats {
            throughput_mbytes_sec,
            decoded_mbytes,
            received_mbytes,
            elapsed,
            result,
        }
    }
}

#[derive(Debug, Copy, Clone)]
pub(crate) struct BenchmarkOpts {
    pub compression: Compression,
    pub validation: bool,
    pub otel: bool,
}

impl BenchmarkOpts {
    pub(crate) fn permutations() -> impl Iterator<Item = Self> + 'static {
        let compression_modes = [
            Compression::None,
            #[cfg(feature = "lz4")]
            Compression::Lz4,
            #[cfg(feature = "zstd")]
            Compression::Zstd(-4),
            #[cfg(feature = "zstd")]
            Compression::Zstd(-1),
            #[cfg(feature = "zstd")]
            Compression::Zstd(1),
            #[cfg(feature = "zstd")]
            Compression::Zstd(zstd::DEFAULT_COMPRESSION_LEVEL),
        ];

        let validation_modes = [false, true];
        let otel_modes = [
            false,
            #[cfg(feature = "opentelemetry")]
            true,
        ];

        compression_modes.into_iter().flat_map(move |compression| {
            validation_modes.into_iter().flat_map(move |validation| {
                otel_modes.into_iter().map(move |otel| BenchmarkOpts {
                    compression,
                    validation,
                    otel,
                })
            })
        })
    }
}

#[cfg(feature = "opentelemetry")]
fn get_tracer() -> opentelemetry::global::BoxedTracer {
    use opentelemetry_sdk::propagation::TraceContextPropagator;
    use opentelemetry_sdk::trace::SdkTracerProvider;

    use std::sync::Once;

    static ONCE: Once = Once::new();

    ONCE.call_once(|| {
        // These set global statics so we want to ensure this is done,
        // but it only needs to be done once.
        opentelemetry::global::set_text_map_propagator(TraceContextPropagator::new());

        let provider = SdkTracerProvider::builder()
            // .with_simple_exporter(opentelemetry_stdout::SpanExporter::default())
            .build();

        opentelemetry::global::set_tracer_provider(provider);
    });

    opentelemetry::global::tracer("clickhouse_test_opentelemetry")
}
