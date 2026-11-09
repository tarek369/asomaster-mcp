//! One multiplexed IPC connection: requests run concurrently and are matched
//! by id; `event` notifications (after `subscribe`) go to a broadcast channel.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::{broadcast, mpsc, oneshot};

use crate::error::{ApiError, BridgeError};

/// JSON-RPC error code carrying an `ApiError` in `data`.
pub const APP_ERROR: i64 = -32000;
const EVENT_BUFFER: usize = 256;

type Reply = Result<Value, BridgeError>;
type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Reply>>>>;

/// `{channel, payload}` pushed by the app after `subscribe`.
#[derive(Debug, Clone, PartialEq)]
pub struct IpcEvent {
    pub channel: String,
    pub payload: Value,
}

pub struct IpcClient {
    out: mpsc::UnboundedSender<String>,
    pending: Pending,
    next_id: AtomicU64,
    events: broadcast::Sender<IpcEvent>,
    closed: Arc<AtomicBool>,
    /// The reader holds half of the stream: it is stopped on drop so the
    /// connection closes (the app serves a fixed number of connections).
    reader: tokio::task::AbortHandle,
}

impl Drop for IpcClient {
    fn drop(&mut self) {
        self.reader.abort();
    }
}

impl IpcClient {
    /// Wrap a connected stream and start its reader / writer tasks.
    pub fn start<S>(stream: S) -> Self
    where
        S: AsyncRead + AsyncWrite + Send + 'static,
    {
        let (read, write) = tokio::io::split(stream);
        let (out, rx) = mpsc::unbounded_channel::<String>();
        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let (events, _) = broadcast::channel(EVENT_BUFFER);
        let closed = Arc::new(AtomicBool::new(false));
        tokio::spawn(write_loop(write, rx, closed.clone()));
        let reader = tokio::spawn(read_loop(
            read,
            pending.clone(),
            events.clone(),
            closed.clone(),
        ))
        .abort_handle();
        Self {
            out,
            pending,
            next_id: AtomicU64::new(1),
            events,
            closed,
            reader,
        }
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    /// Receiver for event notifications (create it before `subscribe`).
    pub fn events(&self) -> broadcast::Receiver<IpcEvent> {
        self.events.subscribe()
    }

    /// Send one request and wait for its response.
    pub async fn request(&self, method: &str, params: Value, timeout: Duration) -> Reply {
        if self.is_closed() {
            return Err(BridgeError::Disconnected);
        }
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        lock(&self.pending).insert(id, tx);
        let msg = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        if self.out.send(msg.to_string()).is_err() {
            lock(&self.pending).remove(&id);
            return Err(BridgeError::Disconnected);
        }
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(reply)) => reply,
            Ok(Err(_)) => Err(BridgeError::Disconnected),
            Err(_) => {
                lock(&self.pending).remove(&id);
                Err(BridgeError::Timeout {
                    method: method.to_string(),
                    secs: timeout.as_secs().max(1),
                })
            }
        }
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

async fn write_loop<W: AsyncWrite>(
    write: W,
    mut rx: mpsc::UnboundedReceiver<String>,
    closed: Arc<AtomicBool>,
) {
    let mut write = Box::pin(write);
    while let Some(mut msg) = rx.recv().await {
        msg.push('\n');
        if write.write_all(msg.as_bytes()).await.is_err() || write.flush().await.is_err() {
            break;
        }
    }
    // Queue closed (client dropped) or write failed: tell the app we are done.
    let _ = write.shutdown().await;
    closed.store(true, Ordering::SeqCst);
}

async fn read_loop<R: AsyncRead>(
    read: R,
    pending: Pending,
    events: broadcast::Sender<IpcEvent>,
    closed: Arc<AtomicBool>,
) {
    let mut reader = BufReader::new(Box::pin(read));
    let mut buf = String::new();
    loop {
        buf.clear();
        match reader.read_line(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let Ok(msg) = serde_json::from_str::<Value>(buf.trim()) else {
            continue;
        };
        route(msg, &pending, &events);
    }
    closed.store(true, Ordering::SeqCst);
    let waiters: Vec<_> = lock(&pending).drain().collect();
    for (_, tx) in waiters {
        let _ = tx.send(Err(BridgeError::Disconnected));
    }
}

fn route(msg: Value, pending: &Pending, events: &broadcast::Sender<IpcEvent>) {
    if msg.get("method").and_then(Value::as_str) == Some("event") {
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        let channel = params
            .get("channel")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let payload = params.get("payload").cloned().unwrap_or(Value::Null);
        let _ = events.send(IpcEvent { channel, payload });
        return;
    }
    let Some(id) = msg.get("id").and_then(Value::as_u64) else {
        return;
    };
    let Some(tx) = lock(pending).remove(&id) else {
        return;
    };
    let _ = tx.send(parse_reply(msg));
}

/// Turn a JSON-RPC response object into a result.
pub fn parse_reply(msg: Value) -> Reply {
    if let Some(err) = msg.get("error") {
        let code = err.get("code").and_then(Value::as_i64).unwrap_or(-32603);
        let message = err
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("unknown error")
            .to_string();
        if code == APP_ERROR {
            if let Some(api) = err
                .get("data")
                .and_then(|d| serde_json::from_value::<ApiError>(d.clone()).ok())
            {
                return Err(BridgeError::Api(api));
            }
        }
        return Err(BridgeError::Rpc { code, message });
    }
    Ok(msg.get("result").cloned().unwrap_or(Value::Null))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_app_errors_into_api_errors() {
        let r = parse_reply(json!({"id":1,"error":{"code":-32000,"message":"x",
            "data":{"code":"not_found","message":"project 9 not found"}}}));
        match r {
            Err(BridgeError::Api(e)) => assert_eq!(e.code, "not_found"),
            other => panic!("{other:?}"),
        }
        let r = parse_reply(json!({"id":1,"error":{"code":-32601,"message":"nope"}}));
        assert!(matches!(r, Err(BridgeError::Rpc { code: -32601, .. })));
        assert_eq!(
            parse_reply(json!({"id":1,"result":null})).unwrap(),
            Value::Null
        );
    }

    #[tokio::test]
    async fn request_round_trip_over_duplex() {
        let (a, b) = tokio::io::duplex(1 << 16);
        let client = IpcClient::start(a);
        tokio::spawn(async move {
            let (r, mut w) = tokio::io::split(b);
            let mut lines = BufReader::new(r).lines();
            while let Ok(Some(l)) = lines.next_line().await {
                let v: Value = serde_json::from_str(&l).unwrap();
                let resp = json!({"jsonrpc":"2.0","id":v["id"],"result":{"echo":v["method"]}});
                w.write_all(format!("{resp}\n").as_bytes()).await.unwrap();
            }
        });
        let r = client
            .request("ping", Value::Null, Duration::from_secs(2))
            .await
            .unwrap();
        assert_eq!(r["echo"], "ping");
    }

    /// A dropped client must close its connection: the app serves a fixed
    /// number of connections, and `wait_for_jobs` opens one per call.
    #[tokio::test]
    async fn dropping_the_client_closes_the_connection() {
        let (a, b) = tokio::io::duplex(1 << 16);
        let client = IpcClient::start(a);
        let (r, mut w) = tokio::io::split(b);
        // The app keeps pushing events to a subscribed connection.
        w.write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"event\",\"params\":{}}\n")
            .await
            .unwrap();
        drop(client);
        let mut lines = BufReader::new(r).lines();
        let eof = tokio::time::timeout(Duration::from_secs(2), lines.next_line()).await;
        assert!(
            matches!(eof, Ok(Ok(None))),
            "the app side saw no EOF: {eof:?}"
        );
    }
}
