//! `asomaster-mcp`: an MCP (Model Context Protocol) stdio server that lets AI
//! agents drive the running ASOMaster desktop app over its local IPC socket
//! (`crates/engine/IPC.md`). The app also serves the same server over local
//! HTTP ([`http`]).

pub mod config;
pub mod error;
pub mod http;
pub mod instructions;
pub mod ipc;
pub mod server;
pub mod summarize;
pub mod tools;

pub use config::BridgeConfig;
pub use ipc::Bridge;
pub use server::{serve, McpServer};
