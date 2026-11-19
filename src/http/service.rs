//! One HTTP request: guard, bounded body, JSON-RPC through [`McpServer`].

use std::convert::Infallible;
use std::sync::Arc;

use bytes::Bytes;
use http::header::{CACHE_CONTROL, CONTENT_LENGTH, CONTENT_TYPE};
use http::{HeaderValue, Request, Response, StatusCode};
use http_body_util::{BodyExt, Full, Limited};
use hyper::body::Incoming;
use serde_json::{json, Value};

use super::guard::{Guard, Rejection};
use crate::server::{McpServer, INVALID_REQUEST, PARSE_ERROR};

pub(super) struct Ctx {
    pub server: Arc<McpServer>,
    pub guard: Guard,
    pub max_body: usize,
}

type Reply = Response<Full<Bytes>>;

pub(super) async fn handle(ctx: Arc<Ctx>, req: Request<Incoming>) -> Result<Reply, Infallible> {
    let (parts, body) = req.into_parts();
    if let Err(r) = ctx.guard.check(&parts.method, parts.uri.path(), &parts.headers) {
        return Ok(rejected(&r));
    }
    let declared = parts
        .headers
        .get(CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());
    if declared.is_some_and(|n| n > ctx.max_body as u64) {
        return Ok(too_large());
    }
    let bytes = match Limited::new(body, ctx.max_body).collect().await {
        Ok(b) => b.to_bytes(),
        Err(e) if e.is::<http_body_util::LengthLimitError>() => return Ok(too_large()),
        Err(_) => return Ok(rpc_error(StatusCode::BAD_REQUEST, INVALID_REQUEST, "Could not read the request body.")),
    };
    let msg: Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(_) => return Ok(rpc_error(StatusCode::BAD_REQUEST, PARSE_ERROR, "The body is not valid JSON.")),
    };
    Ok(match ctx.server.handle_value(msg).await {
        Some(out) => json_reply(StatusCode::OK, &out),
        None => empty(StatusCode::ACCEPTED),
    })
}

fn base(status: StatusCode) -> http::response::Builder {
    Response::builder()
        .status(status)
        .header(CACHE_CONTROL, HeaderValue::from_static("no-store"))
}

fn empty(status: StatusCode) -> Reply {
    base(status).body(Full::new(Bytes::new())).unwrap_or_default()
}

fn json_reply(status: StatusCode, v: &Value) -> Reply {
    base(status)
        .header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
        .body(Full::new(Bytes::from(v.to_string())))
        .unwrap_or_default()
}

/// A JSON-RPC error without an id, as the spec allows for HTTP-level refusals.
fn rpc_error(status: StatusCode, code: i64, message: &str) -> Reply {
    json_reply(
        status,
        &json!({ "jsonrpc": "2.0", "id": null, "error": { "code": code, "message": message } }),
    )
}

fn too_large() -> Reply {
    rpc_error(StatusCode::PAYLOAD_TOO_LARGE, INVALID_REQUEST, "The request is too large.")
}

fn rejected(r: &Rejection) -> Reply {
    let mut reply = rpc_error(r.status, INVALID_REQUEST, r.message);
    if let Some((name, value)) = r.header {
        reply
            .headers_mut()
            .insert(name, HeaderValue::from_static(value));
    }
    reply
}
