#[tokio::main]
async fn main() -> anyhow::Result<()> {
    onetcli_mongodb_driver::run("modern").await
}
