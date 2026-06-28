#[tokio::main]
async fn main() -> anyhow::Result<()> {
    callout::run().await
}
