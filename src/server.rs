use adk_rust::Agent;
use adk_rust::server::ServerConfig;
use adk_session::SqliteSessionService;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::config::NamiConfig;

pub struct NamiServer {
    pub base_url: String,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    task: Option<tokio::task::JoinHandle<Result<(), std::io::Error>>>,
}

pub fn build_server_config(
    agent: Arc<dyn Agent>,
    session_service: Arc<SqliteSessionService>,
    config: &NamiConfig,
) -> anyhow::Result<ServerConfig> {
    let mut server_config = ServerConfig::new(
        Arc::new(adk_rust::SingleAgentLoader::new(agent)),
        session_service,
    );
    if let Some(compaction) = crate::compaction::build_compaction_config(config)? {
        tracing::info!(
            interval = compaction.compaction_interval,
            "Enabled Context Compaction (post-run, every {} turns)",
            compaction.compaction_interval
        );
        server_config = server_config.with_compaction(compaction);
    }
    Ok(server_config)
}

impl NamiServer {
    /// Binds, spawns the server, and polls /api/health until it answers.
    pub async fn start(app: axum::Router, host: &str, port: u16) -> anyhow::Result<Self> {
        let addr_str = format!("{}:{}", host, port);
        let listener = match tokio::net::TcpListener::bind(&addr_str).await {
            Ok(l) => l,
            Err(e) => {
                anyhow::bail!(
                    "Failed to bind to {}: {}. Port may already be in use. Try passing --port <port> or --url <base_url>.",
                    addr_str,
                    e
                );
            }
        };

        let local_addr = listener.local_addr()?;
        let effective_ip = if local_addr.ip().is_unspecified() {
            "127.0.0.1".to_string()
        } else {
            local_addr.ip().to_string()
        };
        let base_url = format!("http://{}:{}", effective_ip, local_addr.port());

        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

        let task = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    let _ = shutdown_rx.await;
                })
                .await
        });

        let client = reqwest::Client::builder()
            .timeout(Duration::from_millis(500))
            .build()?;

        let health_url = format!("{}/api/health", base_url);
        let start_time = Instant::now();
        let timeout = Duration::from_secs(5);
        let mut healthy = false;

        while start_time.elapsed() < timeout {
            if let Ok(resp) = client.get(&health_url).send().await
                && resp.status().is_success()
            {
                healthy = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }

        if !healthy {
            let _ = shutdown_tx.send(());
            let _ = task.await;
            anyhow::bail!(
                "Server failed to become healthy within 5 seconds at {}",
                health_url
            );
        }

        Ok(Self {
            base_url,
            shutdown: Some(shutdown_tx),
            task: Some(task),
        })
    }

    /// Signals graceful shutdown and awaits the task.
    pub async fn shutdown(mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_server_ephemeral_port_readiness() {
        let app = axum::Router::new().route("/api/health", axum::routing::get(|| async { "ok" }));
        let server = NamiServer::start(app, "127.0.0.1", 0).await.unwrap();
        assert!(server.base_url.starts_with("http://127.0.0.1:"));

        let client = reqwest::Client::new();
        let resp = client
            .get(format!("{}/api/health", server.base_url))
            .send()
            .await
            .unwrap();
        assert!(resp.status().is_success());

        server.shutdown().await;
    }
}
