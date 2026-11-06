//! Bridge errors and how they are explained to the agent.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// The engine's error shape (`ApiError` in commands.md).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiError {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    /// Machine-readable fix (`add_package`: accountId, account, packageName).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BridgeError {
    /// The app is not running and could not be started / reached.
    Unavailable(String),
    /// The connection dropped while a request was in flight.
    Disconnected,
    /// No response within the deadline.
    Timeout { method: String, secs: u64 },
    /// An application error from the engine.
    Api(ApiError),
    /// Any other JSON-RPC error from the app.
    Rpc { code: i64, message: String },
    /// Bad input caught by the bridge before reaching the app.
    Invalid(String),
    /// Local failure (I/O, serialisation).
    Internal(String),
}

impl std::fmt::Display for BridgeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable(m) => write!(f, "ASOMaster is not reachable: {m}"),
            Self::Disconnected => write!(f, "lost the connection to ASOMaster"),
            Self::Timeout { method, secs } => {
                write!(f, "ASOMaster did not answer \"{method}\" within {secs}s")
            }
            Self::Api(e) => write!(f, "{}: {}", e.code, e.message),
            Self::Rpc { code, message } => write!(f, "ipc error {code}: {message}"),
            Self::Invalid(m) => write!(f, "invalid input: {m}"),
            Self::Internal(m) => write!(f, "internal error: {m}"),
        }
    }
}

impl std::error::Error for BridgeError {}

impl BridgeError {
    pub fn api_code(&self) -> Option<&str> {
        match self {
            Self::Api(e) => Some(e.code.as_str()),
            _ => None,
        }
    }

    /// Structured payload returned to the agent as an `isError` tool result.
    pub fn to_tool_payload(&self) -> Value {
        let details = match self {
            Self::Api(e) => e.details.clone(),
            _ => None,
        };
        let (code, message, action) = match self {
            Self::Api(e) => (e.code.clone(), e.message.clone(), e.action.clone()),
            Self::Unavailable(m) => ("app_unavailable".into(), m.clone(), None),
            Self::Disconnected => ("disconnected".into(), self.to_string(), None),
            Self::Timeout { .. } => ("timeout".into(), self.to_string(), None),
            Self::Rpc { code, message } => (
                format!("ipc_{}", code.unsigned_abs()),
                message.clone(),
                None,
            ),
            Self::Invalid(m) => ("invalid_input".into(), m.clone(), None),
            Self::Internal(m) => ("internal".into(), m.clone(), None),
        };
        let mut err = json!({ "code": code, "message": message });
        if let Some(a) = action {
            err["action"] = Value::String(a);
        }
        if let Some(d) = details {
            err["details"] = d;
        }
        json!({ "error": err, "hint": hint(&code, self) })
    }
}

/// What the agent should do next for a given error code.
fn hint(code: &str, err: &BridgeError) -> &'static str {
    if matches!(err, BridgeError::Api(e) if e.action.as_deref() == Some("add_package")) {
        return "A Play Console account is connected but does not include this app's package. Call connect_console with store google, accountId and packageNames from error.details (no key needed).";
    }
    match code {
        "license_required" => "ASOMaster needs an active license that includes MCP access. Ask the owner to activate it in the app (Settings > License); do not retry until they confirm.",
        "feature_not_licensed" => "The owner's license does not include this feature. Tell the owner; retrying will not help.",
        "ai_not_configured" | "ai_error" => "No working AI provider. Call get_ai_status, then set_ai_provider / test_ai_provider, or ask the owner to finish AI setup in the app.",
        "console_not_connected" => "No developer console account is connected for this store. Use connect_console (or ask the owner to connect it in the app).",
        "console_error" => "The developer console rejected the request. Check credentials with test_console or ask the owner to replace the key.",
        "rate_limited" => "The store is throttling requests. Wait (wait_for_jobs or a few minutes) and retry; do not hammer.",
        "not_found" => "The id does not exist. Re-read ids with get_overview / get_project_board.",
        "invalid_input" => "Fix the arguments and retry.",
        "conflict" => "The item already exists or conflicts with current state. Re-read state before retrying.",
        "unauthorized" => "Bridge authentication failed. Ask the owner to restart ASOMaster.",
        "unsupported" => "This operation is not supported for this store / format.",
        "cancelled" => "The work was stopped.",
        "app_unavailable" => "Ask the owner to open ASOMaster (and sign in / activate if prompted), then retry.",
        "disconnected" => "The app closed the connection; the request may or may not have been applied. Re-read state with get_overview before retrying.",
        "timeout" => "The app is busy. Call wait_for_jobs or get_overview and retry later.",
        _ if matches!(err, BridgeError::Rpc { code: -32601, .. }) => "This ASOMaster version does not support the operation; ask the owner to update the app.",
        _ => "See the message for details.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_error_payload_keeps_action_and_adds_hint() {
        let e = BridgeError::Api(ApiError {
            code: "license_required".into(),
            message: "not activated".into(),
            action: Some("activate".into()),
            details: None,
        });
        let p = e.to_tool_payload();
        assert_eq!(p["error"]["code"], "license_required");
        assert_eq!(p["error"]["action"], "activate");
        assert!(p["hint"].as_str().unwrap().contains("license"));
    }

    #[test]
    fn transport_errors_have_codes() {
        let p = BridgeError::Timeout {
            method: "x".into(),
            secs: 3,
        }
        .to_tool_payload();
        assert_eq!(p["error"]["code"], "timeout");
        let p = BridgeError::Unavailable("no".into()).to_tool_payload();
        assert_eq!(p["error"]["code"], "app_unavailable");
    }
}
