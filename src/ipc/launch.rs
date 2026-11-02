//! Starting the desktop app when it is not running.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use crate::config::{BridgeConfig, APP_IDENTIFIER};

/// Executable names the desktop app ships under.
const APP_EXE_NAMES: &[&str] = &["ASOMaster", "asomaster", "asomaster-desktop"];

/// Start the app in the background. Returns a human-readable error when it
/// cannot be started (not installed, launcher failed).
pub async fn launch_app(cfg: &BridgeConfig) -> Result<(), String> {
    let mut cmd = command(cfg)?;
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("could not start ASOMaster: {e}"))?;
    if cfg!(target_os = "macos") && cfg.app_path.as_deref().is_none_or(is_bundle) {
        // `open` exits right away; a failure means the app is not installed.
        let status = child
            .wait()
            .await
            .map_err(|e| format!("could not start ASOMaster: {e}"))?;
        if !status.success() {
            return Err(
                "ASOMaster is not installed (macOS could not find com.asomaster.desktop). Install it, open it once, then retry."
                    .into(),
            );
        }
    } else {
        // Detached app process: reap it in the background if it exits.
        tokio::spawn(async move {
            let _ = child.wait().await;
        });
    }
    Ok(())
}

fn is_bundle(p: &Path) -> bool {
    p.extension().is_some_and(|e| e == "app")
}

fn command(cfg: &BridgeConfig) -> Result<tokio::process::Command, String> {
    if cfg!(target_os = "macos") {
        let mut c = tokio::process::Command::new("open");
        match cfg.app_path.as_deref() {
            Some(p) if is_bundle(p) => {
                c.arg("-g").arg("-a").arg(p);
            }
            Some(p) => return Ok(tokio::process::Command::new(p)),
            None => {
                c.arg("-g").arg("-b").arg(APP_IDENTIFIER);
            }
        }
        return Ok(c);
    }
    let exe = match cfg.app_path.clone() {
        Some(p) => p,
        None => find_app_exe().ok_or_else(|| {
            format!(
                "could not find the ASOMaster app executable; open ASOMaster manually or set {}",
                crate::config::ENV_APP_PATH
            )
        })?,
    };
    #[cfg_attr(not(windows), allow(unused_mut))]
    let mut c = tokio::process::Command::new(exe);
    #[cfg(windows)]
    {
        /// `DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP`: no console, not tied to ours.
        const DETACHED: u32 = 0x0000_0008 | 0x0000_0200;
        c.creation_flags(DETACHED);
    }
    Ok(c)
}

/// Candidate locations of the desktop executable (Windows / Linux).
pub fn app_exe_candidates() -> Vec<PathBuf> {
    let suffix = std::env::consts::EXE_SUFFIX;
    let names: Vec<String> = APP_EXE_NAMES
        .iter()
        .map(|n| format!("{n}{suffix}"))
        .collect();
    let mut dirs: Vec<PathBuf> = Vec::new();
    // The bridge is bundled next to the app binary.
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
    {
        dirs.push(dir);
    }
    for var in ["LOCALAPPDATA", "ProgramFiles", "ProgramFiles(x86)"] {
        if let Ok(base) = std::env::var(var) {
            dirs.push(PathBuf::from(&base).join("ASOMaster"));
            dirs.push(PathBuf::from(&base).join("Programs").join("ASOMaster"));
        }
    }
    if cfg!(all(unix, not(target_os = "macos"))) {
        dirs.push(PathBuf::from("/usr/bin"));
        dirs.push(PathBuf::from("/usr/local/bin"));
        if let Ok(home) = std::env::var("HOME") {
            dirs.push(PathBuf::from(home).join(".local").join("bin"));
        }
    }
    dirs.iter()
        .flat_map(|d| names.iter().map(move |n| d.join(n)))
        .collect()
}

fn find_app_exe() -> Option<PathBuf> {
    app_exe_candidates().into_iter().find(|p| p.is_file())
}
