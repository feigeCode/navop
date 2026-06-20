use std::path::PathBuf;

#[tokio::main]
async fn main() {
    if let Err(error) = public_mcp::launcher::run_stdio_bridge(discovery_path_arg()).await {
        eprintln!("onetcli-public-mcp: {error:#}");
        std::process::exit(1);
    }
}

fn discovery_path_arg() -> Option<PathBuf> {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--discovery" {
            return args.next().map(PathBuf::from);
        }
    }
    None
}
