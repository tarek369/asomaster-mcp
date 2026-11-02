//! `asomaster-mcp` binary: MCP over stdin / stdout. Diagnostics go to stderr
//! (stdout carries protocol messages only).

use std::sync::Arc;

use asomaster_mcp::{serve, Bridge, BridgeConfig, McpServer};

fn print_help() {
    eprintln!(
        "asomaster-mcp {} - MCP server for ASOMaster (stdio).\n\n\
         Add it to your agent as a stdio MCP server; it talks to the running ASOMaster app\n\
         and starts the app when needed.\n\n\
         Environment:\n  \
         ASOMASTER_DATA_DIR             app data folder (default: platform app-data dir)\n  \
         ASOMASTER_IPC_ENDPOINT         socket path / pipe name override\n  \
         ASOMASTER_APP_PATH             app to launch when it is not running\n  \
         ASOMASTER_NO_LAUNCH=1          never launch the app\n  \
         ASOMASTER_MCP_CALL_TIMEOUT_MS  per-request timeout (default 90000)",
        env!("CARGO_PKG_VERSION")
    );
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_help();
        return;
    }
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("asomaster-mcp {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    let cfg = match BridgeConfig::from_env() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("asomaster-mcp: {e}");
            std::process::exit(2);
        }
    };
    let server = Arc::new(McpServer::new(Bridge::new(cfg)));
    if let Err(e) = serve(server, tokio::io::stdin(), tokio::io::stdout()).await {
        eprintln!("asomaster-mcp: stdio error: {e}");
        std::process::exit(1);
    }
}
