//! MCP over Streamable HTTP (2025-11-25 transports spec), loopback only.
//!
//! The running app serves this next to the stdio bridge: each POST to `/mcp`
//! carries one JSON-RPC message (or a legacy batch), answered with one JSON
//! body (`200`) or `202` for notifications. No server-initiated stream (GET is
//! `405`) and no sessions. [`guard::Guard`] refuses foreign hosts, browsers,
//! wrong tokens and other methods before the body is read. See
//! `docs/specs/remote-access.md`.

pub mod guard;
mod service;

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use hyper::server::conn::http1;
use hyper_util::rt::{TokioIo, TokioTimer};
use tokio::net::TcpListener;
use tokio::sync::{watch, Semaphore};
use tokio::task::JoinSet;

pub use guard::{Guard, Rejection, MCP_PATH};

use crate::server::McpServer;

/// Open HTTP connections at once (each waits for a slot when full).
pub const MAX_CONNECTIONS: usize = 16;
/// Largest request body (same cap as one IPC line).
pub const MAX_BODY_BYTES: usize = 4 * 1024 * 1024;
/// Time a client has to send the request headers (also closes idle
/// keep-alive connections).
pub const HEADER_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy)]
pub struct HttpLimits {
    pub max_connections: usize,
    pub max_body_bytes: usize,
    pub header_timeout: Duration,
}

impl Default for HttpLimits {
    fn default() -> Self {
        Self {
            max_connections: MAX_CONNECTIONS,
            max_body_bytes: MAX_BODY_BYTES,
            header_timeout: HEADER_TIMEOUT,
        }
    }
}

/// A running server. Dropping the handle stops it too (the stop channel closes).
pub struct HttpHandle {
    pub port: u16,
    stop: watch::Sender<bool>,
    task: tokio::task::JoinHandle<()>,
}

impl HttpHandle {
    /// Stop accepting and close every open connection (requests in flight are
    /// dropped, so a rotated token cannot keep serving on an old connection).
    pub fn stop(&self) {
        let _ = self.stop.send(true);
    }

    /// [`HttpHandle::stop`], then wait until the port is released (so the
    /// same port can be bound again right away).
    pub async fn shutdown(self) {
        self.stop();
        let _ = self.task.await;
    }

    pub fn url(&self) -> String {
        url_for(self.port)
    }
}

pub fn url_for(port: u16) -> String {
    format!("http://127.0.0.1:{port}{MCP_PATH}")
}

/// Bind `127.0.0.1:<port>` (`0` = any free port). Never any other interface.
pub async fn bind(port: u16) -> std::io::Result<TcpListener> {
    TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, port))).await
}

/// Serve `listener` in the background with `token` as the bearer token.
pub fn start(
    listener: TcpListener,
    server: Arc<McpServer>,
    token: String,
    limits: HttpLimits,
) -> std::io::Result<HttpHandle> {
    let addr = listener.local_addr()?;
    if !addr.ip().is_loopback() {
        return Err(std::io::Error::other("MCP over HTTP only listens on loopback"));
    }
    let port = addr.port();
    let (stop, stopped) = watch::channel(false);
    let guard = Guard::new(token, port);
    let task = tokio::spawn(accept_loop(listener, server, guard, limits, stopped));
    Ok(HttpHandle { port, stop, task })
}

async fn accept_loop(
    listener: TcpListener,
    server: Arc<McpServer>,
    guard: Guard,
    limits: HttpLimits,
    mut stopped: watch::Receiver<bool>,
) {
    let slots = Arc::new(Semaphore::new(limits.max_connections.max(1)));
    let mut conns = JoinSet::new();
    let ctx = Arc::new(service::Ctx {
        server,
        guard,
        max_body: limits.max_body_bytes,
    });
    loop {
        let permit = tokio::select! {
            _ = stopped.changed() => break,
            p = slots.clone().acquire_owned() => match p { Ok(p) => p, Err(_) => break },
        };
        let accepted = tokio::select! {
            _ = stopped.changed() => break,
            a = listener.accept() => a,
        };
        // Reap finished connection tasks so the set does not grow.
        while conns.try_join_next().is_some() {}
        let stream = match accepted {
            Ok((stream, _)) => stream,
            Err(e) => {
                tracing::debug!(error = %e, "mcp http accept failed");
                tokio::time::sleep(Duration::from_millis(50)).await;
                continue;
            }
        };
        let ctx = ctx.clone();
        let timeout = limits.header_timeout;
        conns.spawn(async move {
            let _permit = permit;
            let svc = hyper::service::service_fn(move |req| service::handle(ctx.clone(), req));
            let conn = http1::Builder::new()
                .timer(TokioTimer::new())
                .header_read_timeout(timeout)
                .serve_connection(TokioIo::new(stream), svc);
            if let Err(e) = conn.await {
                tracing::debug!(error = %e, "mcp http connection ended");
            }
        });
    }
    drop(listener);
    conns.shutdown().await;
}
