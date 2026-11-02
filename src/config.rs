//! Where the running app listens and how the bridge reaches it.
//!
//! Data dir resolution mirrors the desktop app (Tauri `app_data_dir` for the
//! `com.asomaster.desktop` identifier), with `ASOMASTER_DATA_DIR` as override.

use std::path::{Path, PathBuf};
use std::time::Duration;

pub const APP_IDENTIFIER: &str = "com.asomaster.desktop";
pub const SOCKET_FILE: &str = "mcp.sock";
pub const TOKEN_FILE: &str = "mcp.token";

pub const ENV_DATA_DIR: &str = "ASOMASTER_DATA_DIR";
pub const ENV_ENDPOINT: &str = "ASOMASTER_IPC_ENDPOINT";
pub const ENV_APP_PATH: &str = "ASOMASTER_APP_PATH";
pub const ENV_NO_LAUNCH: &str = "ASOMASTER_NO_LAUNCH";
pub const ENV_CALL_TIMEOUT_MS: &str = "ASOMASTER_MCP_CALL_TIMEOUT_MS";

/// How long to keep retrying while the app starts.
pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// Per-request timeout for ordinary calls (long work runs as jobs).
pub const DEFAULT_CALL_TIMEOUT: Duration = Duration::from_secs(90);

#[derive(Debug, Clone)]
pub struct BridgeConfig {
    pub data_dir: PathBuf,
    /// Unix socket path or Windows pipe name.
    pub endpoint: String,
    pub token_path: PathBuf,
    /// Start the app when the endpoint is missing.
    pub launch: bool,
    /// Explicit app executable / bundle to start (otherwise platform default).
    pub app_path: Option<PathBuf>,
    pub connect_timeout: Duration,
    pub call_timeout: Duration,
    /// User home: agents may only point the bridge at files in its
    /// Downloads / Desktop / Documents (`None` = no file access).
    pub home_dir: Option<PathBuf>,
}

impl BridgeConfig {
    /// Config for a data dir with platform-default endpoint and timeouts.
    pub fn for_data_dir(data_dir: PathBuf) -> Self {
        Self {
            endpoint: endpoint_for(&data_dir),
            token_path: data_dir.join(TOKEN_FILE),
            data_dir,
            launch: true,
            app_path: None,
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
            call_timeout: DEFAULT_CALL_TIMEOUT,
            home_dir: home(),
        }
    }

    /// Config from the environment (used by the binary).
    pub fn from_env() -> Result<Self, String> {
        let data_dir = match non_empty_env(ENV_DATA_DIR) {
            Some(d) => PathBuf::from(d),
            None => default_data_dir().ok_or_else(|| {
                format!("cannot locate the ASOMaster data folder; set {ENV_DATA_DIR}")
            })?,
        };
        let mut cfg = Self::for_data_dir(data_dir);
        if let Some(ep) = non_empty_env(ENV_ENDPOINT) {
            cfg.endpoint = ep;
        }
        cfg.app_path = non_empty_env(ENV_APP_PATH).map(PathBuf::from);
        cfg.launch = !matches!(
            non_empty_env(ENV_NO_LAUNCH).as_deref(),
            Some("1" | "true" | "yes")
        );
        if let Some(ms) = non_empty_env(ENV_CALL_TIMEOUT_MS).and_then(|v| v.parse::<u64>().ok()) {
            cfg.call_timeout = Duration::from_millis(ms.max(1_000));
        }
        Ok(cfg)
    }
}

fn non_empty_env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

/// Platform data dir, identical to the app's:
/// macOS `~/Library/Application Support/<id>`, Windows `%APPDATA%\<id>`,
/// Linux `$XDG_DATA_HOME/<id>` (default `~/.local/share/<id>`).
pub fn default_data_dir() -> Option<PathBuf> {
    base_data_dir().map(|b| b.join(APP_IDENTIFIER))
}

fn home() -> Option<PathBuf> {
    non_empty_env("HOME")
        .or_else(|| non_empty_env("USERPROFILE"))
        .map(PathBuf::from)
}

#[cfg(target_os = "macos")]
fn base_data_dir() -> Option<PathBuf> {
    home().map(|h| h.join("Library").join("Application Support"))
}

#[cfg(windows)]
fn base_data_dir() -> Option<PathBuf> {
    non_empty_env("APPDATA")
        .map(PathBuf::from)
        .or_else(|| home().map(|h| h.join("AppData").join("Roaming")))
}

#[cfg(all(unix, not(target_os = "macos")))]
fn base_data_dir() -> Option<PathBuf> {
    non_empty_env("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| home().map(|h| h.join(".local").join("share")))
}

/// Endpoint for a data dir. Must match `engine::ipc::endpoint`: the socket
/// file on unix, `\\.\pipe\asomaster-mcp-<FNV-1a 64 of the dir path>` on Windows.
pub fn endpoint_for(data_dir: &Path) -> String {
    if cfg!(windows) {
        pipe_name(data_dir)
    } else {
        data_dir.join(SOCKET_FILE).to_string_lossy().to_string()
    }
}

/// Windows pipe name for a data dir (exposed for tests on every platform).
pub fn pipe_name(data_dir: &Path) -> String {
    let digest = data_dir
        .to_string_lossy()
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
            (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
        });
    format!(r"\\.\pipe\asomaster-mcp-{digest:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipe_name_is_stable_fnv1a() {
        // FNV-1a 64 of the empty string is the offset basis.
        assert_eq!(
            pipe_name(Path::new("")),
            r"\\.\pipe\asomaster-mcp-cbf29ce484222325"
        );
        let a = pipe_name(Path::new(
            r"C:\Users\a\AppData\Roaming\com.asomaster.desktop",
        ));
        assert!(a.starts_with(r"\\.\pipe\asomaster-mcp-"));
        assert_eq!(a.len(), r"\\.\pipe\asomaster-mcp-".len() + 16);
    }

    #[test]
    fn data_dir_ends_with_identifier() {
        if let Some(d) = default_data_dir() {
            assert!(d.ends_with(APP_IDENTIFIER));
        }
    }

    #[cfg(unix)]
    #[test]
    fn unix_endpoint_is_socket_in_data_dir() {
        let cfg = BridgeConfig::for_data_dir(PathBuf::from("/tmp/x"));
        assert_eq!(cfg.endpoint, "/tmp/x/mcp.sock");
        assert_eq!(cfg.token_path, PathBuf::from("/tmp/x/mcp.token"));
    }
}
