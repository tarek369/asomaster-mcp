//! MCP over local HTTP against a fake app: real TCP, hand-written HTTP/1.1 so
//! the tests control Host / Origin exactly.
#![cfg(unix)]

mod common;

use std::sync::Arc;
use std::time::Duration;

use asomaster_mcp::http::{self, HttpHandle, HttpLimits};
use common::{FakeApp, Reply};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const TOKEN: &str = "a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5b6c7d8e9f0a1b2";

/// Method, path, headers, expected status.
type Case = (&'static str, &'static str, Vec<(&'static str, String)>, u16);

struct Resp {
    status: u16,
    head: String,
    body: String,
}

impl Resp {
    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or(Value::Null)
    }
    fn header(&self, name: &str) -> Option<String> {
        self.head.lines().find_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.trim().eq_ignore_ascii_case(name).then(|| v.trim().to_string())
        })
    }
}

async fn raw(port: u16, request: String) -> Resp {
    let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    s.write_all(request.as_bytes()).await.unwrap();
    let mut buf = Vec::new();
    tokio::time::timeout(Duration::from_secs(10), s.read_to_end(&mut buf))
        .await
        .expect("response in time")
        .unwrap();
    let text = String::from_utf8_lossy(&buf).to_string();
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    let status = head
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    Resp {
        status,
        head: head.to_string(),
        body: body.to_string(),
    }
}

fn request(method: &str, path: &str, headers: &[(&str, String)], body: &str) -> String {
    let mut r = format!("{method} {path} HTTP/1.1\r\nConnection: close\r\nContent-Length: {}\r\n", body.len());
    for (k, v) in headers {
        r.push_str(&format!("{k}: {v}\r\n"));
    }
    r.push_str("\r\n");
    r.push_str(body);
    r
}

fn good_headers(port: u16) -> Vec<(&'static str, String)> {
    vec![
        ("Host", format!("127.0.0.1:{port}")),
        ("Authorization", format!("Bearer {TOKEN}")),
        ("Content-Type", "application/json".into()),
        ("Accept", "application/json, text/event-stream".into()),
    ]
}

async fn post(port: u16, body: Value) -> Resp {
    raw(port, request("POST", "/mcp", &good_headers(port), &body.to_string())).await
}

async fn start(app: &FakeApp, limits: HttpLimits) -> HttpHandle {
    let listener = http::bind(0).await.unwrap();
    http::start(listener, Arc::new(app.server()), TOKEN.into(), limits).unwrap()
}

async fn fake() -> FakeApp {
    FakeApp::start(|method, _| match method {
        "auth" => Reply::Ok(json!({ "ok": true })),
        "get_snapshot" => Reply::Ok(json!({ "projects": [], "folders": [], "jobs": [], "settings": {} })),
        _ => Reply::Ok(json!({})),
    })
    .await
}

#[tokio::test]
async fn initialize_list_and_call_over_http() {
    let app = fake().await;
    let h = start(&app, HttpLimits::default()).await;
    assert_eq!(h.url(), format!("http://127.0.0.1:{}/mcp", h.port));

    let init = post(
        h.port,
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}),
    )
    .await;
    assert_eq!(init.status, 200, "{}", init.body);
    assert!(init.header("content-type").unwrap().starts_with("application/json"));
    assert_eq!(init.header("cache-control").as_deref(), Some("no-store"));
    assert!(init.header("mcp-session-id").is_none());
    assert_eq!(init.json()["result"]["protocolVersion"], "2025-11-25");

    let note = post(h.port, json!({"jsonrpc":"2.0","method":"notifications/initialized"})).await;
    assert_eq!(note.status, 202);
    assert!(note.body.is_empty());

    let list = post(h.port, json!({"jsonrpc":"2.0","id":2,"method":"tools/list"})).await;
    let tools = list.json()["result"]["tools"].as_array().unwrap().len();
    assert_eq!(tools, asomaster_mcp::tools::registry().tools().len());

    let call = post(
        h.port,
        json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"get_overview","arguments":{}}}),
    )
    .await;
    assert_eq!(call.status, 200);
    assert_eq!(call.json()["result"]["isError"], false, "{}", call.body);
    assert!(app.methods().contains(&"get_snapshot".to_string()));

    let batch = post(
        h.port,
        json!([{"jsonrpc":"2.0","id":4,"method":"ping"},{"jsonrpc":"2.0","method":"notifications/x"}]),
    )
    .await;
    assert_eq!(batch.json().as_array().unwrap().len(), 1);
    h.stop();
}

#[tokio::test]
async fn refusals_follow_the_spec() {
    let app = fake().await;
    let h = start(&app, HttpLimits::default()).await;
    let p = h.port;
    let body = json!({"jsonrpc":"2.0","id":1,"method":"ping"}).to_string();
    let with = |k: &'static str, v: String| {
        let mut hs: Vec<_> = good_headers(p).into_iter().filter(|(n, _)| *n != k).collect();
        hs.push((k, v));
        hs
    };
    let cases: Vec<Case> = vec![
        ("POST", "/mcp", with("Host", format!("evil.test:{p}")), 403),
        ("POST", "/mcp", with("Origin", "https://evil.test".into()), 403),
        ("POST", "/mcp", with("Origin", format!("http://127.0.0.1:{p}")), 403),
        ("POST", "/other", good_headers(p), 404),
        ("POST", "/mcp", with("Authorization", "Bearer nope".into()), 401),
        ("GET", "/mcp", good_headers(p), 405),
        ("DELETE", "/mcp", good_headers(p), 405),
        ("POST", "/mcp", with("Content-Type", "text/plain".into()), 415),
        ("POST", "/mcp", with("MCP-Protocol-Version", "2000-01-01".into()), 400),
    ];
    for (method, path, headers, want) in cases {
        let r = raw(p, request(method, path, &headers, &body)).await;
        assert_eq!(r.status, want, "{method} {path} {headers:?}: {}", r.body);
        assert!(r.json()["error"]["message"].is_string(), "{}", r.body);
        assert!(r.header("access-control-allow-origin").is_none());
    }
    let unauthorized = raw(p, request("POST", "/mcp", &with("Authorization", "Bearer x".into()), &body)).await;
    assert_eq!(unauthorized.header("www-authenticate").as_deref(), Some("Bearer"));
    let get = raw(p, request("GET", "/mcp", &good_headers(p), "")).await;
    assert_eq!(get.header("allow").as_deref(), Some("POST"));
    // Nothing refused reached the app.
    assert!(!app.methods().iter().any(|m| m == "ping"));

    let bad = raw(p, request("POST", "/mcp", &good_headers(p), "{not json")).await;
    assert_eq!(bad.status, 400);
    assert_eq!(bad.json()["error"]["code"], -32700);
    h.stop();
}

#[tokio::test]
async fn oversized_bodies_are_refused() {
    let app = fake().await;
    let limits = HttpLimits {
        max_body_bytes: 64,
        ..HttpLimits::default()
    };
    let h = start(&app, limits).await;
    let big = json!({"jsonrpc":"2.0","id":1,"method":"ping","params":{"pad":"x".repeat(200)}});
    let r = post(h.port, big).await;
    assert_eq!(r.status, 413);
    h.stop();
}

#[tokio::test]
async fn stop_closes_the_listener_and_open_connections() {
    let app = fake().await;
    let h = start(&app, HttpLimits::default()).await;
    let port = h.port;
    // An idle keep-alive connection opened before the stop.
    let mut idle = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    h.stop();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(TcpStream::connect(("127.0.0.1", port)).await.is_err());
    let mut buf = [0u8; 16];
    let n = tokio::time::timeout(Duration::from_secs(2), idle.read(&mut buf))
        .await
        .expect("closed in time")
        .unwrap_or(0);
    assert_eq!(n, 0);
}

#[tokio::test]
async fn silent_clients_are_dropped_after_the_header_timeout() {
    let app = fake().await;
    let limits = HttpLimits {
        header_timeout: Duration::from_millis(200),
        ..HttpLimits::default()
    };
    let h = start(&app, limits).await;
    let mut s = TcpStream::connect(("127.0.0.1", h.port)).await.unwrap();
    s.write_all(b"POST /mcp HTTP/1.1\r\n").await.unwrap();
    let mut buf = Vec::new();
    let done = tokio::time::timeout(Duration::from_secs(3), s.read_to_end(&mut buf)).await;
    assert!(done.is_ok(), "connection still open after the header timeout");
    h.stop();
}

#[tokio::test]
async fn binds_loopback_only() {
    let l = http::bind(0).await.unwrap();
    assert!(l.local_addr().unwrap().ip().is_loopback());
}

#[tokio::test]
async fn idle_keep_alive_connections_do_not_hold_a_slot() {
    let app = fake().await;
    let limits = HttpLimits {
        header_timeout: Duration::from_millis(300),
        max_connections: 1,
        ..HttpLimits::default()
    };
    let h = start(&app, limits).await;
    let body = json!({"jsonrpc":"2.0","id":1,"method":"ping"}).to_string();
    let mut req = request("POST", "/mcp", &good_headers(h.port), &body);
    req = req.replacen("Connection: close\r\n", "", 1);
    let mut idle = TcpStream::connect(("127.0.0.1", h.port)).await.unwrap();
    idle.write_all(req.as_bytes()).await.unwrap();
    let mut first = [0u8; 12];
    idle.read_exact(&mut first).await.unwrap();
    assert_eq!(&first, b"HTTP/1.1 200");
    // With one slot, a second client only gets in once the idle one is closed.
    let second = tokio::time::timeout(Duration::from_secs(5), post(h.port, json!({"jsonrpc":"2.0","id":2,"method":"ping"})))
        .await
        .expect("idle connection released its slot");
    assert_eq!(second.status, 200);
    h.stop();
}

#[tokio::test]
async fn shutdown_releases_the_port_for_a_restart() {
    let app = fake().await;
    let h = start(&app, HttpLimits::default()).await;
    let port = h.port;
    h.shutdown().await;
    let again = http::bind(port).await.expect("same port again");
    let h2 = http::start(again, Arc::new(app.server()), TOKEN.into(), HttpLimits::default()).unwrap();
    assert_eq!(h2.port, port);
    h2.shutdown().await;
}
