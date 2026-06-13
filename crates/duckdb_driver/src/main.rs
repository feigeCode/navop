use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .init();

    let socket_name = std::env::var("ONETCLI_EXT_SOCKET")
        .ok()
        .or_else(|| std::env::args().nth(1))
        .unwrap_or_else(|| "onetcli-duckdb-driver.sock".to_string());

    duckdb_driver::server::run(&socket_name).await
}
