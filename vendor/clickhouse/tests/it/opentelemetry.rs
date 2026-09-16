use crate::get_client;
use opentelemetry::Context;
use opentelemetry::trace::{Status, TraceContextExt, TracerProvider};
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::propagation::TraceContextPropagator;
use opentelemetry_sdk::trace::{SdkTracer, SdkTracerProvider, Span, SpanData, SpanProcessor};
use std::cell::Cell;
use std::sync::Once;
use std::time::Duration;
use tracing::Instrument;
use tracing_opentelemetry::OpenTelemetrySpanExt;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

#[tokio::test]
async fn query_with_opentelemetry() {
    let tracer = get_tracer();

    let _guard = tracing_subscriber::registry()
        .with(tracing_opentelemetry::layer().with_tracer(tracer))
        .set_default();

    let span = tracing::info_span!("query_with_opentelemetry", "foo=bar");

    let context = span.context();

    let client = get_client();

    let query_id = format!("clickhouse_test_{}", rand::random::<u64>());

    let _numbers = client
        .query("SELECT * FROM system.numbers LIMIT 10")
        .with_setting("query_id", query_id.clone())
        .fetch_all::<u64>()
        .instrument(span)
        .await
        .unwrap();

    let trace_id = context.span().span_context().trace_id();

    crate::flush_query_log(&client).await;

    let span_query_id = client
        .query(
            "SELECT attribute['clickhouse.query_id'] AS query_id \
         FROM system.opentelemetry_span_log \
         WHERE trace_id = {trace_id:String} AND query_id != ''",
        )
        .param("trace_id", trace_id.to_string())
        // ClickHouse parses and stores the hex span ID as an integer
        .fetch_one::<String>()
        .await
        .unwrap();

    assert_eq!(query_id, span_query_id);
}

#[tokio::test]
async fn insert_with_opentelemetry() {
    #[derive(serde::Serialize, clickhouse::Row, Debug)]
    struct FooRow {
        bar: i32,
        baz: String,
    }

    let tracer = get_tracer();

    let _guard = tracing_subscriber::registry()
        .with(tracing_opentelemetry::layer().with_tracer(tracer))
        .set_default();

    let client = prepare_database!();

    client
        .query("CREATE TABLE foo(bar Int32, baz String)")
        .execute()
        .await
        .unwrap();

    let span = tracing::info_span!("insert_with_opentelemetry", "foo=bar");

    let context = span.context();

    let query_id = format!("clickhouse_test_{}", rand::random::<u64>());

    // Ensure this entire request chain is instrumented
    async {
        let mut insert = client
            .insert::<FooRow>("foo")
            .await
            .unwrap()
            .with_setting("query_id", &query_id);

        for i in 0..10 {
            insert
                .write(&FooRow {
                    bar: i,
                    baz: format!("baz_{i}"),
                })
                .await
                .unwrap();
        }

        insert.end().await.unwrap();
    }
    .instrument(span)
    .await;

    let trace_id = context.span().span_context().trace_id();

    crate::flush_query_log(&client).await;

    // The metadata query by `Insert` will appear under the same trace ID,
    // so we sort by event time to ensure we're looking at the last span emitted,
    // which should be the insert itself.
    let span_query_id = client
        .query(
            "SELECT attribute['clickhouse.query_id'] AS query_id \
         FROM system.opentelemetry_span_log \
         WHERE trace_id = {trace_id:String} AND query_id != ''\
         ORDER BY finish_time_us DESC",
        )
        .param("trace_id", trace_id.to_string())
        // ClickHouse parses and stores the hex span ID as an integer
        .fetch_one::<String>()
        .await
        .unwrap();

    assert_eq!(query_id, span_query_id);
}

// FIXME: this test is far too flaky to be useful
// https://github.com/ClickHouse/clickhouse-rs/issues/463
#[ignore]
#[tokio::test]
async fn error_sets_span_status() {
    let tracer = get_tracer();

    let _guard = tracing_subscriber::registry()
        .with(tracing_opentelemetry::layer().with_tracer(tracer))
        .set_default();

    let res = get_client()
        .query("SELECT throwif(true, 'test error')")
        .execute()
        .await;

    let _err = res.unwrap_err();

    // `opentelemetry` doesn't give us any way to look up the status of a live span,
    // so we instead install a middleware that stores its data in a thread-local.
    let span_data = LAST_SPAN_DATA.take().expect("expected last span data");

    assert!(
        matches!(span_data.status, Status::Error { .. }),
        "expected error status, got {:?}",
        span_data.status
    );
}

thread_local! {
    /// The data for the last span ended on the current thread.
    static LAST_SPAN_DATA: Cell<Option<SpanData>> = const {  Cell::new(None) };
}

fn get_tracer() -> SdkTracer {
    static ONCE: Once = Once::new();

    #[derive(Debug)]
    struct LastSpanProcessor;

    impl SpanProcessor for LastSpanProcessor {
        fn on_start(&self, _span: &mut Span, _cx: &Context) {}

        fn on_end(&self, span: SpanData) {
            eprintln!("last span data: {span:?}");
            LAST_SPAN_DATA.set(Some(span));
        }

        fn force_flush(&self) -> OTelSdkResult {
            Ok(())
        }

        fn shutdown_with_timeout(&self, _timeout: Duration) -> OTelSdkResult {
            Ok(())
        }
    }

    ONCE.call_once(|| {
        // This sets a global static so we want to ensure this is done,
        // but it only needs to be done once.
        opentelemetry::global::set_text_map_propagator(TraceContextPropagator::new());
    });

    SdkTracerProvider::builder()
        .with_simple_exporter(opentelemetry_stdout::SpanExporter::default())
        .with_span_processor(LastSpanProcessor)
        .build()
        .tracer("clickhouse_test_opentelemetry")
}
