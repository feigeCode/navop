use anyhow::Result;

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("onetcli-public-mcp: {error:#}");
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    let discovery_path = public_mcp::launcher::parse_discovery_path_arg(std::env::args().skip(1))?;
    public_mcp::launcher::run_stdio_bridge(discovery_path).await
}
