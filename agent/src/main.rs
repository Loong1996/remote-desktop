mod config;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    tracing::info!("rd-agent starting");
    // 实际启动逻辑在 Task 6 接入
    Ok(())
}
