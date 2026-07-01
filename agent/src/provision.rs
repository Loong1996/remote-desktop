use serde::Deserialize;
use crate::config::AgentConfig;

#[derive(Deserialize)]
struct LoginResp { token: String }
#[derive(Deserialize)]
struct PairResp {
    #[serde(rename = "deviceId")]
    device_id: String,
    token: String,
}

pub async fn provision(
    server_url: &str,
    email: &str,
    password: &str,
    device_name: &str,
) -> anyhow::Result<AgentConfig> {
    let client = reqwest::Client::new();

    let login = client
        .post(format!("{server_url}/login"))
        .json(&serde_json::json!({ "email": email, "password": password }))
        .send()
        .await?;
    if !login.status().is_success() {
        anyhow::bail!("login failed: HTTP {}", login.status());
    }
    let jwt = login.json::<LoginResp>().await?.token;

    let pair = client
        .post(format!("{server_url}/devices/pair"))
        .bearer_auth(&jwt)
        .json(&serde_json::json!({ "name": device_name }))
        .send()
        .await?;
    if !pair.status().is_success() {
        anyhow::bail!("pair failed: HTTP {}", pair.status());
    }
    let pair = pair.json::<PairResp>().await?;

    Ok(AgentConfig {
        server_url: server_url.to_string(),
        device_id: pair.device_id,
        device_token: pair.token,
    })
}

pub fn prompt_credentials() -> anyhow::Result<(String, String)> {
    use std::io::Write;
    print!("Email: ");
    std::io::stdout().flush()?;
    let mut email = String::new();
    std::io::stdin().read_line(&mut email)?;
    let password = rpassword::prompt_password("Password: ")?;
    Ok((email.trim().to_string(), password))
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::{Mock, MockServer, ResponseTemplate};
    use wiremock::matchers::{method, path, header};

    #[tokio::test]
    async fn provision_logs_in_then_pairs() {
        let server = MockServer::start().await;
        Mock::given(method("POST")).and(path("/login"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"token":"jwt-xyz"})))
            .mount(&server).await;
        Mock::given(method("POST")).and(path("/devices/pair"))
            .and(header("authorization", "Bearer jwt-xyz"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"deviceId":"dev-9","token":"devtok-9"})))
            .mount(&server).await;

        let cfg = provision(&server.uri(), "a@b.com", "pw123456", "MyPC").await.unwrap();
        assert_eq!(cfg.device_id, "dev-9");
        assert_eq!(cfg.device_token, "devtok-9");
        assert_eq!(cfg.server_url, server.uri());
    }

    #[tokio::test]
    async fn provision_errors_on_bad_login() {
        let server = MockServer::start().await;
        Mock::given(method("POST")).and(path("/login"))
            .respond_with(ResponseTemplate::new(401).set_body_json(serde_json::json!({"error":"bad credentials"})))
            .mount(&server).await;
        let res = provision(&server.uri(), "a@b.com", "wrong", "MyPC").await;
        assert!(res.is_err());
    }
}
