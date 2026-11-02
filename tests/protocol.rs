//! MCP handshake, version negotiation and JSON-RPC error handling.
#![cfg(unix)]

mod common;

use std::sync::Arc;

use asomaster_mcp::server::SUPPORTED_VERSIONS;
use asomaster_mcp::{serve, Bridge, BridgeConfig, McpServer};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

fn offline_server() -> McpServer {
    let mut cfg = BridgeConfig::for_data_dir(std::env::temp_dir().join("asomaster-mcp-none"));
    cfg.launch = false;
    McpServer::new(Bridge::new(cfg))
}

async fn rpc(server: &McpServer, msg: Value) -> Value {
    server.handle_value(msg).await.expect("a response")
}

#[tokio::test]
async fn initialize_negotiates_every_supported_version() {
    let server = offline_server();
    for v in SUPPORTED_VERSIONS {
        let r = rpc(
            &server,
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
                "protocolVersion": v, "capabilities": {}, "clientInfo": {"name":"t","version":"1"}}}),
        )
        .await;
        assert_eq!(r["result"]["protocolVersion"], *v);
        assert_eq!(r["result"]["serverInfo"]["name"], "asomaster");
        assert!(r["result"]["capabilities"]["tools"].is_object());
        let instr = r["result"]["instructions"].as_str().unwrap();
        assert!(instr.contains("wait_for_jobs"));
        assert!(instr.contains("publish_listing"));
        assert!(instr.to_lowercase().contains("estimate"));
    }
}

#[tokio::test]
async fn unknown_version_gets_latest() {
    let r = rpc(
        &offline_server(),
        json!({"jsonrpc":"2.0","id":"a","method":"initialize","params":{"protocolVersion":"2030-01-01"}}),
    )
    .await;
    assert_eq!(r["id"], "a");
    assert_eq!(r["result"]["protocolVersion"], SUPPORTED_VERSIONS[0]);
}

#[tokio::test]
async fn ping_and_notifications() {
    let s = offline_server();
    let r = rpc(&s, json!({"jsonrpc":"2.0","id":2,"method":"ping"})).await;
    assert_eq!(r["result"], json!({}));
    assert!(s
        .handle_value(json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
        .await
        .is_none());
    assert!(s
        .handle_value(
            json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":2}})
        )
        .await
        .is_none());
    // Responses from the client are ignored.
    assert!(s
        .handle_value(json!({"jsonrpc":"2.0","id":9,"result":{}}))
        .await
        .is_none());
}

#[tokio::test]
async fn error_codes() {
    let s = offline_server();
    let parse = s.handle_line("{not json").await.unwrap();
    let parse: Value = serde_json::from_str(&parse).unwrap();
    assert_eq!(parse["error"]["code"], -32700);
    assert_eq!(parse["id"], Value::Null);

    let r = rpc(
        &s,
        json!({"jsonrpc":"2.0","id":3,"method":"resources/list"}),
    )
    .await;
    assert_eq!(r["error"]["code"], -32601);

    let r = rpc(&s, json!({"jsonrpc":"1.0","id":4,"method":"ping"})).await;
    assert_eq!(r["error"]["code"], -32600);

    let r = rpc(&s, json!({"id":5})).await;
    assert_eq!(r["error"]["code"], -32600);

    let r = rpc(&s, json!(42)).await;
    assert_eq!(r["error"]["code"], -32600);

    let r = rpc(
        &s,
        json!({"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"nope"}}),
    )
    .await;
    assert_eq!(r["error"]["code"], -32602);

    let r = rpc(
        &s,
        json!({"jsonrpc":"2.0","id":7,"method":"tools/call","params":{}}),
    )
    .await;
    assert_eq!(r["error"]["code"], -32602);

    let r = rpc(
        &s,
        json!({"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"get_overview","arguments":[1]}}),
    )
    .await;
    assert_eq!(r["error"]["code"], -32602);
}

#[tokio::test]
async fn batches_are_answered_in_order() {
    let s = offline_server();
    let r = rpc(
        &s,
        json!([
            {"jsonrpc":"2.0","id":1,"method":"ping"},
            {"jsonrpc":"2.0","method":"notifications/initialized"},
            {"jsonrpc":"2.0","id":2,"method":"tools/list"}
        ]),
    )
    .await;
    let arr = r.as_array().unwrap();
    assert_eq!(arr.len(), 2);
    assert_eq!(arr[0]["id"], 1);
    assert!(arr[1]["result"]["tools"].as_array().unwrap().len() > 40);
    let empty = rpc(&s, json!([])).await;
    assert_eq!(empty["error"]["code"], -32600);
}

#[tokio::test]
async fn app_not_running_is_a_tool_error_not_a_crash() {
    let (is_error, payload) = common::call_tool(&offline_server(), "get_overview", json!({})).await;
    assert!(is_error);
    assert_eq!(payload["error"]["code"], "app_unavailable");
    assert!(payload["hint"].as_str().unwrap().contains("open ASOMaster"));
}

#[tokio::test]
async fn stdio_session_end_to_end() {
    let server = Arc::new(offline_server());
    let (client, srv) = tokio::io::duplex(1 << 20);
    let (srv_r, srv_w) = tokio::io::split(srv);
    let task = tokio::spawn(serve(server, srv_r, srv_w));
    let (cr, mut cw) = tokio::io::split(client);
    let mut lines = BufReader::new(cr).lines();
    let send = [
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
    ];
    for m in send {
        cw.write_all(format!("{m}\n").as_bytes()).await.unwrap();
    }
    let mut seen = std::collections::HashMap::new();
    for _ in 0..2 {
        let line = lines.next_line().await.unwrap().unwrap();
        let v: Value = serde_json::from_str(&line).unwrap();
        seen.insert(v["id"].as_i64().unwrap(), v);
    }
    assert_eq!(seen[&1]["result"]["protocolVersion"], "2025-06-18");
    assert!(seen[&2]["result"]["tools"].is_array());
    drop(cw);
    drop(lines);
    task.await.unwrap().unwrap();
}
