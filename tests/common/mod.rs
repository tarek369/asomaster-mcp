//! A fake ASOMaster IPC server on a unix socket, speaking IPC.md.
#![allow(dead_code)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use asomaster_mcp::{Bridge, BridgeConfig, McpServer};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tokio::sync::{broadcast, mpsc};

pub enum Reply {
    Ok(Value),
    /// JSON-RPC error object.
    Err(Value),
    /// Never answer (timeouts).
    Silent,
}

pub fn api_error(code: &str, message: &str, action: Option<&str>) -> Reply {
    let mut data = json!({ "code": code, "message": message });
    if let Some(a) = action {
        data["action"] = json!(a);
    }
    Reply::Err(json!({ "code": -32000, "message": message, "data": data }))
}

type Handler = dyn Fn(&str, &Value) -> Reply + Send + Sync;

pub struct FakeApp {
    pub dir: tempfile::TempDir,
    pub calls: Arc<Mutex<Vec<(String, Value)>>>,
    events: broadcast::Sender<Value>,
}

impl FakeApp {
    pub async fn start(handler: impl Fn(&str, &Value) -> Reply + Send + Sync + 'static) -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("mcp.token"), "secret-token").unwrap();
        let listener = UnixListener::bind(dir.path().join("mcp.sock")).expect("bind");
        let calls: Arc<Mutex<Vec<(String, Value)>>> = Arc::default();
        let (events, _) = broadcast::channel(64);
        let handler: Arc<Handler> = Arc::new(handler);
        let (c, ev) = (calls.clone(), events.clone());
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                tokio::spawn(serve_conn(stream, handler.clone(), c.clone(), ev.clone()));
            }
        });
        Self { dir, calls, events }
    }

    pub fn config(&self) -> BridgeConfig {
        let mut cfg = BridgeConfig::for_data_dir(self.dir.path().to_path_buf());
        cfg.launch = false;
        cfg.connect_timeout = Duration::from_secs(1);
        cfg.call_timeout = Duration::from_secs(3);
        cfg.home_dir = Some(self.dir.path().to_path_buf());
        cfg
    }

    /// `<fake home>/Downloads`, where agents may point the bridge at files.
    pub fn downloads(&self) -> std::path::PathBuf {
        let d = self.dir.path().join("Downloads");
        std::fs::create_dir_all(&d).expect("downloads");
        d
    }

    pub fn server(&self) -> McpServer {
        McpServer::new(Bridge::new(self.config()))
    }

    pub fn server_with(&self, cfg: BridgeConfig) -> McpServer {
        McpServer::new(Bridge::new(cfg))
    }

    pub fn push_event(&self, channel: &str, payload: Value) {
        let _ = self
            .events
            .send(json!({ "jsonrpc": "2.0", "method": "event", "params": { "channel": channel, "payload": payload } }));
    }

    pub fn methods(&self) -> Vec<String> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .map(|(m, _)| m.clone())
            .collect()
    }

    /// Every dispatched action (via `dispatch` or an action-type method).
    pub fn actions(&self) -> Vec<Value> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter_map(|(m, p)| match m.as_str() {
                "dispatch" => Some(p["action"].clone()),
                m if m.contains('.') => {
                    let mut a = p.clone();
                    a["type"] = json!(m);
                    Some(a)
                }
                _ => None,
            })
            .collect()
    }
}

async fn serve_conn(
    stream: tokio::net::UnixStream,
    handler: Arc<Handler>,
    calls: Arc<Mutex<Vec<(String, Value)>>>,
    events: broadcast::Sender<Value>,
) {
    let (r, mut w) = stream.into_split();
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    tokio::spawn(async move {
        while let Some(line) = rx.recv().await {
            if w.write_all(format!("{line}\n").as_bytes()).await.is_err() {
                break;
            }
        }
    });
    let mut lines = BufReader::new(r).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let req: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let method = req["method"].as_str().unwrap_or_default().to_string();
        let params = req.get("params").cloned().unwrap_or(Value::Null);
        let id = req["id"].clone();
        if method == "subscribe" {
            let mut rx_ev = events.subscribe();
            let tx2 = tx.clone();
            tokio::spawn(async move {
                while let Ok(ev) = rx_ev.recv().await {
                    if tx2.send(ev.to_string()).is_err() {
                        break;
                    }
                }
            });
            let _ = tx.send(json!({"jsonrpc":"2.0","id":id,"result":{"ok":true}}).to_string());
            continue;
        }
        if method != "auth" {
            calls.lock().unwrap().push((method.clone(), params.clone()));
        }
        let reply = if method == "auth" {
            Reply::Ok(json!({"ok": true}))
        } else {
            handler(&method, &params)
        };
        match reply {
            Reply::Ok(v) => {
                let _ = tx.send(json!({"jsonrpc":"2.0","id":id,"result":v}).to_string());
            }
            Reply::Err(e) => {
                let _ = tx.send(json!({"jsonrpc":"2.0","id":id,"error":e}).to_string());
            }
            Reply::Silent => {}
        }
    }
}

/// Default answers for every read command.
pub fn default_reply(method: &str, _params: &Value) -> Reply {
    Reply::Ok(match method {
        "get_snapshot" => snapshot(json!([])),
        "license_status" => {
            json!({"state":"active","plan":"pro","features":["mcp"],"devices":[],"maxDevices":2})
        }
        "list_alerts" => {
            json!([{"id":"a1","projectId":"1","keyword":"habit","market":"us","from":12,"to":4,"at":"2026-10-01T00:00:00Z","read":false}])
        }
        "get_board" => board(),
        "list_markets" => {
            json!([{"store":"apple","country":"us","name":"United States","searchLanguage":"en","listingLocales":["en-US"],"featured":true,"potential":10.0}])
        }
        "console_accounts" | "console_apps" | "ai_status" | "popularity_status" => json!([]),
        "next_version" => json!({"suggested":"1.2.0"}),
        "export_data" => json!({"fileName":"b.csv","mime":"text/csv","content":"a,b\n1,2\n"}),
        "import_preview" => {
            json!({"format":"csv","rows":1,"apps":[],"keywords":["x"],"countries":["us"],"skipped":0})
        }
        _ => Value::Null,
    })
}

pub fn snapshot(jobs: Value) -> Value {
    json!({
        "projects": [{"id":"1","store":"apple","kind":"mine","name":"Habit App","appId":"123","folderId":null,"order":0,
            "markets":["us"],"mainMarket":"us","brief":null,"autoCheck":"inherit","keywordCount":3,"top10Count":1,"checkedAt":null,"busy":false}],
        "folders": [{"id":"f1","name":"Mine","order":0}],
        "jobs": jobs,
        "settings": {"metricDriver":"estimate","cellDisplay":"rank","aiProvider":null,"checkSchedule":"daily","defaultMarkets":{"apple":["us"]}}
    })
}

pub fn board() -> Value {
    json!({
        "project": {"id":"1","store":"apple","kind":"mine","name":"Habit App","markets":["us"],"mainMarket":"us","keywordCount":1,"top10Count":1},
        "markets": [{"country":"us","name":"United States","searchLanguage":"en"}],
        "sections": [{"id":"listing","rows":[{"keywordId":"k1","label":"habit tracker","source":"manual",
            "cells":{"us":{"phrase":"habit tracker","rank":4,"prevRank":9,"history":[{"day":"2026-10-01","rank":9}],
                "estimate":{"volume":61,"difficulty":44},"popularity":null,"verdict":"winning","checked":true,"overridden":false,"opportunity":70}},
            "summary":{"top10":1,"ranked":1,"markets":1,"bestRank":4,"avgVolume":61.0,"verdict":"winning","addedAt":"2026-09-01T00:00:00Z"}}]}],
        "competitors": [], "checkedAt": "2026-10-01T00:00:00Z"
    })
}

pub fn job(id: &str, status: &str) -> Value {
    json!({"id":id,"kind":"check","projectId":"1","label":"Check Habit App","status":status,"done":1,"total":4,
        "manual":true,"startedAt":"2026-10-01T00:00:00Z","finishedAt": if status == "running" { Value::Null } else { json!("2026-10-01T00:01:00Z") }})
}

/// Call a tool through the MCP layer; returns (isError, parsed payload).
pub async fn call_tool(server: &McpServer, name: &str, args: Value) -> (bool, Value) {
    let req = json!({"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":name,"arguments":args}});
    let resp = server.handle_value(req).await.expect("response");
    assert!(resp.get("error").is_none(), "protocol error: {resp}");
    let result = &resp["result"];
    let text = result["content"][0]["text"].as_str().expect("text content");
    let payload: Value = serde_json::from_str(text).expect("json text");
    (result["isError"] == json!(true), payload)
}
