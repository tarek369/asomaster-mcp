//! Connection to the running app (protocol: `crates/engine/IPC.md`).
//!
//! [`Bridge`] keeps one shared connection for ordinary calls, (re)connects
//! lazily, starts the app when the endpoint is missing and retries with
//! backoff until `connect_timeout`.

pub mod client;
pub mod launch;
pub mod transport;

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tokio::sync::Mutex;

pub use client::{IpcClient, IpcEvent};

use crate::config::BridgeConfig;
use crate::error::BridgeError;

const FIRST_BACKOFF: Duration = Duration::from_millis(250);
const MAX_BACKOFF: Duration = Duration::from_secs(2);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

pub struct Bridge {
    cfg: BridgeConfig,
    shared: Mutex<Option<Arc<IpcClient>>>,
}

impl Bridge {
    pub fn new(cfg: BridgeConfig) -> Self {
        Self {
            cfg,
            shared: Mutex::new(None),
        }
    }

    pub fn config(&self) -> &BridgeConfig {
        &self.cfg
    }

    /// Call a command / action / helper with the default timeout.
    pub async fn call(&self, method: &str, params: Value) -> Result<Value, BridgeError> {
        self.call_with_timeout(method, params, self.cfg.call_timeout)
            .await
    }

    pub async fn call_with_timeout(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, BridgeError> {
        let client = self.shared_client().await?;
        client.request(method, params, timeout).await
    }

    /// `dispatch` one action (`{"type": ..., ...fields}`).
    pub async fn dispatch(&self, action: Value) -> Result<Value, BridgeError> {
        self.call("dispatch", json!({ "action": action })).await
    }

    /// The shared connection, reconnecting when the previous one closed.
    async fn shared_client(&self) -> Result<Arc<IpcClient>, BridgeError> {
        let mut guard = self.shared.lock().await;
        if let Some(c) = guard.as_ref().filter(|c| !c.is_closed()) {
            return Ok(c.clone());
        }
        let client = Arc::new(self.connect().await?);
        *guard = Some(client.clone());
        Ok(client)
    }

    /// A fresh dedicated connection (used for event subscriptions).
    pub async fn connect(&self) -> Result<IpcClient, BridgeError> {
        let deadline = Instant::now() + self.cfg.connect_timeout;
        let mut launched = false;
        let mut backoff = FIRST_BACKOFF;
        loop {
            match transport::open(&self.cfg.endpoint).await {
                Ok(stream) => {
                    let client = IpcClient::start(stream);
                    self.authenticate(&client).await?;
                    return Ok(client);
                }
                Err(e) => {
                    let missing = transport::is_not_listening(&e);
                    if !missing {
                        return Err(BridgeError::Unavailable(format!(
                            "cannot open {}: {e}",
                            self.cfg.endpoint
                        )));
                    }
                    if !self.cfg.launch {
                        return Err(BridgeError::Unavailable(
                            "the ASOMaster app is not running (auto-launch is disabled). Open ASOMaster and retry."
                                .into(),
                        ));
                    }
                    if !launched {
                        launch::launch_app(&self.cfg)
                            .await
                            .map_err(BridgeError::Unavailable)?;
                        launched = true;
                    }
                }
            }
            if Instant::now() + backoff > deadline {
                return Err(BridgeError::Unavailable(format!(
                    "started ASOMaster but it did not accept connections within {}s. If it shows a window (license, update, sign-in), finish it and retry.",
                    self.cfg.connect_timeout.as_secs()
                )));
            }
            tokio::time::sleep(backoff).await;
            backoff = (backoff * 3 / 2).min(MAX_BACKOFF);
        }
    }

    /// Send `auth` with the token the app wrote on start. Required on every
    /// platform: without a readable token, or when the app refuses it, the
    /// connection is not used.
    async fn authenticate(&self, client: &IpcClient) -> Result<(), BridgeError> {
        let token = read_token(&self.cfg.token_path)?;
        client
            .request("auth", json!({ "token": token }), HANDSHAKE_TIMEOUT)
            .await
            .map(|_| ())
    }
}

/// The bridge token (`mcp.token`), which the app rewrites on every start.
fn read_token(path: &std::path::Path) -> Result<String, BridgeError> {
    let unreadable = |why: String| {
        BridgeError::Unavailable(format!(
            "cannot read the bridge token at {} ({why}). Restart ASOMaster and retry.",
            path.display()
        ))
    };
    let token = std::fs::read_to_string(path)
        .map_err(|e| unreadable(e.to_string()))?
        .trim()
        .to_string();
    if token.is_empty() {
        return Err(unreadable("empty".into()));
    }
    Ok(token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_is_read_trimmed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mcp.token");
        std::fs::write(&path, "abc123\n").unwrap();
        assert_eq!(read_token(&path).unwrap(), "abc123");
    }

    #[test]
    fn missing_or_empty_token_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mcp.token");
        let missing = read_token(&path).unwrap_err();
        assert!(matches!(missing, BridgeError::Unavailable(ref m) if m.contains("bridge token")));
        std::fs::write(&path, "  \n").unwrap();
        assert!(matches!(
            read_token(&path),
            Err(BridgeError::Unavailable(_))
        ));
    }
}
