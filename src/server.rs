//! MCP over stdio: newline-delimited JSON-RPC 2.0. Handles `initialize`
//! (version negotiation), `ping`, `tools/list` and `tools/call`; requests run
//! concurrently so a long `wait_for_jobs` never blocks a `ping`.

use std::sync::Arc;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

use crate::error::BridgeError;
use crate::instructions::INSTRUCTIONS;
use crate::ipc::Bridge;
use crate::summarize::prune;
use crate::tools::{self, registry};

/// Newest first; the first entry is offered when the client asks for an
/// unknown version.
pub const SUPPORTED_VERSIONS: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];
pub const SERVER_NAME: &str = "asomaster";

pub const PARSE_ERROR: i64 = -32700;
pub const INVALID_REQUEST: i64 = -32600;
pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;
pub const INTERNAL_ERROR: i64 = -32603;

/// Pick the protocol version for the session.
pub fn negotiate(requested: Option<&str>) -> &'static str {
    requested
        .and_then(|r| SUPPORTED_VERSIONS.iter().find(|v| **v == r).copied())
        .unwrap_or(SUPPORTED_VERSIONS[0])
}

fn error(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message.into() } })
}

fn ok(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

/// `tools/call` result with one compact JSON text block.
pub fn tool_result(payload: &Value, is_error: bool) -> Value {
    json!({
        "content": [{ "type": "text", "text": payload.to_string() }],
        "isError": is_error,
    })
}

pub struct McpServer {
    bridge: Bridge,
}

impl McpServer {
    pub fn new(bridge: Bridge) -> Self {
        Self { bridge }
    }

    /// Handle one input line; `None` when nothing must be written back.
    pub async fn handle_line(&self, line: &str) -> Option<String> {
        let msg = match serde_json::from_str::<Value>(line) {
            Ok(v) => v,
            Err(e) => {
                return Some(
                    error(Value::Null, PARSE_ERROR, format!("parse error: {e}")).to_string(),
                )
            }
        };
        self.handle_value(msg).await.map(|v| v.to_string())
    }

    /// A message or a batch (batches are accepted for 2024-11-05 / 2025-03-26 clients).
    pub async fn handle_value(&self, msg: Value) -> Option<Value> {
        match msg {
            Value::Array(items) if items.is_empty() => {
                Some(error(Value::Null, INVALID_REQUEST, "empty batch"))
            }
            Value::Array(items) => {
                let mut out = Vec::new();
                for item in items {
                    if let Some(r) = self.handle_message(item).await {
                        out.push(r);
                    }
                }
                (!out.is_empty()).then_some(Value::Array(out))
            }
            other => self.handle_message(other).await,
        }
    }

    async fn handle_message(&self, msg: Value) -> Option<Value> {
        let Value::Object(obj) = &msg else {
            return Some(error(
                Value::Null,
                INVALID_REQUEST,
                "expected a JSON-RPC object",
            ));
        };
        let id = obj.get("id").cloned();
        let Some(method) = obj.get("method").and_then(Value::as_str) else {
            // A response to a server request (we send none) or garbage.
            return match (id, obj.contains_key("result") || obj.contains_key("error")) {
                (_, true) => None,
                (id, false) => Some(error(
                    id.unwrap_or(Value::Null),
                    INVALID_REQUEST,
                    "missing method",
                )),
            };
        };
        if obj.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
            return id.map(|id| error(id, INVALID_REQUEST, "jsonrpc must be \"2.0\""));
        }
        let params = obj.get("params").cloned().unwrap_or(Value::Null);
        // Notifications (no id) never get a response.
        let id = id?;
        Some(match self.request(method, &params).await {
            Ok(result) => ok(id, result),
            Err((code, message)) => error(id, code, message),
        })
    }

    async fn request(&self, method: &str, params: &Value) -> Result<Value, (i64, String)> {
        match method {
            "initialize" => Ok(initialize(params)),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({
                "tools": registry().tools().iter().map(tools::Tool::to_mcp).collect::<Vec<_>>()
            })),
            "tools/call" => self.call_tool(params).await,
            _ => Err((METHOD_NOT_FOUND, format!("method not found: {method}"))),
        }
    }

    async fn call_tool(&self, params: &Value) -> Result<Value, (i64, String)> {
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .ok_or((INVALID_PARAMS, "tools/call needs a tool name".to_string()))?;
        let reg = registry();
        let tool = reg
            .get(name)
            .ok_or_else(|| (INVALID_PARAMS, format!("unknown tool: {name}")))?;
        let args = match params.get("arguments") {
            None | Some(Value::Null) => json!({}),
            Some(v @ Value::Object(_)) => v.clone(),
            Some(_) => return Err((INVALID_PARAMS, "arguments must be an object".into())),
        };
        if let Err(msg) = reg.validate(name, &args) {
            let e = BridgeError::Invalid(msg);
            return Ok(tool_result(&e.to_tool_payload(), true));
        }
        Ok(match tools::run(&self.bridge, tool, args).await {
            Ok(v) => tool_result(&prune(v), false),
            Err(e) => tool_result(&e.to_tool_payload(), true),
        })
    }
}

fn initialize(params: &Value) -> Value {
    let version = negotiate(params.get("protocolVersion").and_then(Value::as_str));
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": { "listChanged": false } },
        "serverInfo": {
            "name": SERVER_NAME,
            "title": "ASOMaster",
            "version": env!("CARGO_PKG_VERSION"),
        },
        "instructions": INSTRUCTIONS,
    })
}

/// Serve until `input` closes. Each request runs in its own task; responses
/// are written one per line in completion order.
pub async fn serve<R, W>(server: Arc<McpServer>, input: R, output: W) -> std::io::Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    let writer = tokio::spawn(async move {
        let mut output = output;
        while let Some(mut line) = rx.recv().await {
            line.push('\n');
            output.write_all(line.as_bytes()).await?;
            output.flush().await?;
        }
        Ok::<(), std::io::Error>(())
    });
    let mut lines = BufReader::new(input).lines();
    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let server = server.clone();
        let tx = tx.clone();
        tokio::spawn(async move {
            if let Some(resp) = server.handle_line(line.trim()).await {
                let _ = tx.send(resp);
            }
        });
    }
    drop(tx);
    writer
        .await
        .map_err(|e| std::io::Error::other(e.to_string()))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negotiates_known_versions_and_falls_back_to_latest() {
        assert_eq!(negotiate(Some("2024-11-05")), "2024-11-05");
        assert_eq!(negotiate(Some("2025-06-18")), "2025-06-18");
        assert_eq!(negotiate(Some("1999-01-01")), "2025-11-25");
        assert_eq!(negotiate(None), "2025-11-25");
    }
}
