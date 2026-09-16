use opentelemetry::trace::TracerProvider;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::propagation::TraceContextPropagator;
use opentelemetry_sdk::trace::SdkTracerProvider;
use tracing::Instrument;
use tracing::level_filters::LevelFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer};

#[derive(serde::Serialize, serde::Deserialize, clickhouse::Row, Debug)]
struct Foo {
    bar: u32,
    baz: String,
}

#[tokio::main]
async fn main() {
    // The `TraceContextPropagator` is responsible for generating a trace ID
    // that will then be forwarded to ClickHouse server for distributed tracing:
    // https://clickhouse.com/docs/operations/opentelemetry
    //
    // Without this, OpenTelemetry spans will still be generated, but the `clickhouse` crate
    // will not be able to add the `traceparent` and `tracestate` headers to requests
    // to the ClickHouse HTTP interface.
    opentelemetry::global::set_text_map_propagator(TraceContextPropagator::new());

    // Configure export of OTLP spans over HTTP.
    let span_exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_http()
        // The endpoint to listen on for the collector (default).
        .with_endpoint("http://localhost:4318/")
        .build()
        .unwrap();

    let tracer_provider = SdkTracerProvider::builder()
        // `.with_batch_exporter()` spawns a background thread to collect traces
        // so application threads don't get blocked.
        .with_batch_exporter(span_exporter)
        .build();

    // Creates a `tracing` subscriber stack that obeys the `RUST_LOG` variable, writes JSON logs to
    // stdout, and emits span traces to our OTLP endpoint using a different variable.
    //
    // Unlike traces, OpenTelemetry doesn't have a strong recommendation for applications to export
    // logs through OTLP, instead preferring whatever frameworks/conventions already exist for the
    // given programming language.
    //
    // To send logs to OpenTelemetry as well, set up `opentelemetry-appender-tracing` as shown here:
    // https://github.com/open-telemetry/opentelemetry-rust/blob/main/examples/logs-basic/src/main.rs
    // Permalink to the current version as of writing:
    // https://github.com/open-telemetry/opentelemetry-rust/blob/965078315b58ae14725721735f1c8e2bc2d3b445/examples/logs-basic/src/main.rs
    //
    // Notice how (as of writing) it has to add filters for crates that it calls internally to avoid
    // generating logs in a loop. Since the `clickhouse` crate also uses some of these dependencies
    // (namely `hyper`), which might be useful have in logs, it didn't make sense to include this in
    // the example until this limitation was addressed.
    //
    // Writing logs via `tracing-subscriber` directly avoids this issue.
    //
    // `tracing-opentelemetry` also already exports `tracing` events as OTel "span events"
    // which are recorded in the context of their parent span, which is arguably more useful
    // than exporting events as logs anyway, which does not include trace metadata.
    // In this light, it would seem that exporting logs separately to OTel is simply redundant.
    //
    // However, we would love to hear from anyone who can provide more context here.
    // If you have questions or comments, please feel free to join the #rust channel
    // in our community Slack and chat with us: clickhouse.com/slack
    tracing_subscriber::registry()
        .with(
            // Write JSON logs to stdout. Most cloud providers can consume these automatically,
            // or be configured to consume them.
            tracing_subscriber::fmt::layer()
                // Omit this to write pretty-printed logs for human consumption.
                // Requires the `json` feature of `tracing`.
                .json()
                // Don't output ANSI terminal color codes when pretty-printing to avoid the added
                // noise when reading logs in a viewer that doesn't handle them.
                // No-op when combined with `.json()` (shown here for informational purposes).
                .with_ansi(false)
                // Filter logs using the conventional `RUST_LOG` variable:
                // https://docs.rs/tracing-subscriber/latest/tracing_subscriber/fmt/index.html#filtering-events-with-environment-variables
                // By placing the filter here, we can filter spans going to OTLP separately.
                //
                // Run with the following command to see logs:
                // env RUST_LOG=info cargo run --example opentelemetry_example --features opentelemetry
                .with_filter(EnvFilter::from_default_env()),
        )
        .with(
            // Automatically links `tracing::Span`s to OpenTelemetry spans.
            tracing_opentelemetry::layer()
                .with_tracer(
                    // Set the application name in emitted spans.
                    tracer_provider.tracer("clickhouse_rs_example_opentelemetry"),
                )
                // Filter what modules emit traces to OTLP by setting `RUST_SPAN`,
                // using the same syntax as the `RUST_LOG` variable.
                .with_filter(
                    EnvFilter::builder()
                        .with_env_var("RUST_SPAN")
                        // By default, emit all spans at `INFO` level and above.
                        .with_default_directive(LevelFilter::INFO.into())
                        // Don't error if the `RUST_SPAN` variable fails to parse.
                        // This is the default behavior of `tracing_subscriber::fmt::init()`.
                        .from_env_lossy(),
                ),
        )
        .init();

    let client = clickhouse::Client::default().with_url("http://localhost:8123");

    // Note that when instrumenting your code, you should not explicitly enter a span in an
    // `async fn` or `async {}` block if the guard will be held across an `.await`:
    //
    // ```
    // let span = tracing::info_span!("insert_query");
    // let _span_guard = span.enter();
    //
    // foo().await;
    // ```
    //
    // This will not properly exit the span when `.await` causes control flow to yield,
    // potentially leading to spans in unexpected parts of the code showing up in the trace
    // when the runtime polls their tasks.
    //
    // Instead, you should apply the `#[tracing::instrument]` attribute to an `async fn` you want
    // in a span, or use the `.instrument()` combinator to explicitly attach a span to a `Future`.
    // These ensure the span is only entered when the `Future` is actually being polled,
    // and also ensures that any child `Future`s it awaits are also covered by the span.
    //
    // https://docs.rs/tracing/latest/tracing/struct.Span.html#in-asynchronous-code
    client
        .query(
            "CREATE OR REPLACE TABLE clickhouse_rs_example_opentelemetry( \
                 bar UInt32, \
                 baz String \
             ) ENGINE = MergeTree PRIMARY KEY bar",
        )
        .execute()
        // Attaches the following span just to this query:
        .instrument(tracing::info_span!("create_table_query"))
        .await
        .unwrap();

    // This call is self-instrumenting thanks to the `#[instrument]` attribute (see below).
    insert_query(&client, 16).await;

    // An inline `async {}` block can be used to instrument just one bit of code,
    // albeit at a small cost to code readability:
    async {
        let foos = client
            .query("SELECT bar, baz FROM clickhouse_rs_example_opentelemetry ORDER BY bar")
            .fetch_all::<Foo>()
            .await
            .unwrap();

        tracing::info!(count = foos.len(), "successfully retrieved `count` records");

        assert_eq!(foos.len(), 16);
    }
    .instrument(tracing::info_span!("select_query"))
    .await;
}

// Wraps this whole function in a span named `insert_query` with the `count` field being recorded.
//
// `Client` does not implement `Debug` and so should be skipped (it would add too much noise anyway)
//
// Note that the `attributes` feature of `tracing` is required to use `#[instrument]`.
// https://docs.rs/tracing/latest/tracing/attr.instrument.html
#[tracing::instrument(skip(client))] // Referenced fully qualified here for clarity.
async fn insert_query(client: &clickhouse::Client, count: u32) {
    // `Client::insert()` makes a query the first time for each table to get metadata for validation,
    // and then the actual insert request begins on the first write, so if you want a single span to
    // cover both those requests then it should be applied to the outer `async {}` block or `async fn`.
    let mut insert = client
        .insert::<Foo>("clickhouse_rs_example_opentelemetry")
        .await
        .unwrap();

    for i in 0..count {
        insert
            .write(&Foo {
                bar: i,
                baz: format!("baz_{i}"),
            })
            .await
            .unwrap();
    }

    insert.end().await.unwrap();

    tracing::info!("successfully inserted `count` records");
}
