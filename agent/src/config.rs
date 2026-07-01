use std::path::PathBuf;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    pub server_url: String,
    pub device_id: String,
    pub device_token: String,
}

pub fn config_path() -> PathBuf {
    if let Ok(p) = std::env::var("RD_AGENT_CONFIG") {
        return PathBuf::from(p);
    }
    let base = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    base.join("rd-agent").join("config.json")
}

impl AgentConfig {
    pub fn load() -> anyhow::Result<Option<AgentConfig>> {
        let path = config_path();
        if !path.exists() {
            return Ok(None);
        }
        let data = std::fs::read_to_string(&path)?;
        Ok(Some(serde_json::from_str(&data)?))
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let path = config_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn save_then_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::env::set_var("RD_AGENT_CONFIG", &path);
        let cfg = AgentConfig {
            server_url: "http://127.0.0.1:8080".into(),
            device_id: "dev-1".into(),
            device_token: "tok-abc".into(),
        };
        cfg.save().unwrap();
        let loaded = AgentConfig::load().unwrap().expect("should exist");
        assert_eq!(loaded.device_id, "dev-1");
        assert_eq!(loaded.device_token, "tok-abc");
        std::env::remove_var("RD_AGENT_CONFIG");
    }
    #[test]
    fn load_missing_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("RD_AGENT_CONFIG", dir.path().join("nope.json"));
        assert!(AgentConfig::load().unwrap().is_none());
        std::env::remove_var("RD_AGENT_CONFIG");
    }
}
