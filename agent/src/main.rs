use rd_agent::config;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    tracing::info!("rd-agent starting");
    let _ = config::config_path();
    Ok(())
}
