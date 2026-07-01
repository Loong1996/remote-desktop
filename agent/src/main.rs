use rd_agent::{config::AgentConfig, provision, signaling};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    if !rd_agent::permission::check_input_permission() {
        tracing::warn!("continuing without input permission; session will connect but injection is disabled");
    }

    if !rd_agent::permission::check_screen_recording_permission() {
        tracing::warn!("continuing without screen-recording permission; video will be blank");
    }

    let cfg = match AgentConfig::load()? {
        Some(c) => {
            tracing::info!("loaded config for device {}", c.device_id);
            c
        }
        None => {
            let server = std::env::var("RD_SERVER_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:8080".to_string());
            println!("No config found. Log in to pair this device.");
            let (email, password) = provision::prompt_credentials()?;
            let name = hostname_or("rd-agent");
            let cfg = provision::provision(&server, &email, &password, &name).await?;
            cfg.save()?;
            println!("Paired as device {}", cfg.device_id);
            cfg
        }
    };

    signaling::run_agent(cfg).await
}

fn hostname_or(fallback: &str) -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| fallback.to_string())
}
