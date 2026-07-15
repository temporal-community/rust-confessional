use rust_confessional::{config::StageConfig, init_tracing, stage};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();
    stage::run(StageConfig::from_env()?).await
}
