//! Developer console and popularity-source connections. Secrets go from the
//! agent (preferably as file paths read locally) straight to the app, which
//! stores them in the OS keychain. Nothing secret is ever returned.

use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

use super::schema as s;
use super::{Annotations, Tool, DESTRUCTIVE, WORK};
use crate::error::BridgeError;
use crate::ipc::Bridge;

/// Credential files are small; refuse anything suspiciously large.
const MAX_SECRET_FILE: u64 = 1024 * 1024;
const MAX_IMPORT_FILE: u64 = 20 * 1024 * 1024;

/// Verifies stored credentials against the remote service.
const VERIFY: Annotations = Annotations {
    read_only: false,
    destructive: false,
    idempotent: true,
    open_world: true,
};

pub fn tools() -> Vec<Tool> {
    vec![
        Tool::custom(
            "connect_console",
            "Connect developer console",
            "Connect an App Store Connect API key (apple: keyId, issuerId, private key .p8) or a Google Play service account (google: service-account JSON, optional package names). Pass FILE PATHS (privateKeyPath / serviceAccountPath; files must be .p8/.pem/.json/.txt in the owner's Downloads, Desktop or Documents folder) so secrets never enter the conversation; the bridge reads them locally and the app stores them in the OS keychain. Credentials are never returned. Needed for console apps, publishing and own-app metadata.",
            s::object(
                vec![
                    ("store", s::store()),
                    ("label", s::text("Account label shown in the app.", 1, 100)),
                    ("keyId", s::text("apple: API key id.", 1, 64)),
                    ("issuerId", s::text("apple: issuer id.", 1, 64)),
                    ("privateKeyPath", s::text("apple: path to the AuthKey_XXXX.p8 file.", 1, 1024)),
                    ("privateKeyPem", s::text("apple: PEM contents (prefer privateKeyPath).", 1, 10000)),
                    ("serviceAccountPath", s::text("google: path to the service-account JSON file.", 1, 1024)),
                    ("serviceAccountJson", s::text("google: service-account JSON contents (prefer serviceAccountPath).", 1, 100000)),
                    ("packageNames", s::array(s::text("Package name.", 1, 255), "google: package names this account manages.", 0, 500)),
                ],
                &["store"],
            ),
            WORK,
        ),
        Tool::action(
            "test_console",
            "Test console account",
            "Verify a connected console account still works (updates its status in list_console_apps).",
            s::object(vec![("accountId", s::id("Console account id (list_console_apps)."))], &["accountId"]),
            VERIFY,
            "console.test",
            &[],
            false,
        ),
        Tool::action(
            "disconnect_console",
            "Disconnect console account",
            "Remove a console account and delete its stored key from the keychain. Ask the owner first.",
            s::object(vec![("accountId", s::id("Console account id."))], &["accountId"]),
            DESTRUCTIVE,
            "console.disconnect",
            &[],
            false,
        ),
        Tool::custom(
            "connect_popularity",
            "Connect popularity source",
            "Connect a source of REAL keyword popularity: apple_ads (Apple Ads API: clientId, teamId, keyId, private key, optional orgId), google_ads_planner (Google Ads Keyword Planner: developerToken, customerId, refreshToken, oauthClientId, oauthClientSecret, optional loginCustomerId) or imported (a popularity export file at filePath). Prefer file paths for keys. Secrets go to the OS keychain and are never returned. Then call fetch_popularity.",
            s::object(
                vec![
                    ("source", s::enumeration(&["apple_ads", "google_ads_planner", "imported"], "Popularity source.")),
                    ("clientId", s::text("apple_ads: client id.", 1, 200)),
                    ("teamId", s::text("apple_ads: team id.", 1, 200)),
                    ("keyId", s::text("apple_ads: key id.", 1, 200)),
                    ("privateKeyPath", s::text("apple_ads: path to the private key PEM.", 1, 1024)),
                    ("privateKeyPem", s::text("apple_ads: PEM contents (prefer privateKeyPath).", 1, 10000)),
                    ("orgId", s::text("apple_ads: org id when several are visible.", 1, 64)),
                    ("developerToken", s::text("google_ads_planner: developer token.", 1, 200)),
                    ("customerId", s::text("google_ads_planner: customer id.", 1, 64)),
                    ("refreshToken", s::text("google_ads_planner: OAuth refresh token.", 1, 2000)),
                    ("oauthClientId", s::text("google_ads_planner: OAuth client id.", 1, 500)),
                    ("oauthClientSecret", s::text("google_ads_planner: OAuth client secret.", 1, 500)),
                    ("loginCustomerId", s::text("google_ads_planner: manager account id.", 1, 64)),
                    ("filePath", s::text("imported: path to the export file (CSV / JSON).", 1, 1024)),
                    ("store", s::store()),
                ],
                &["source"],
            ),
            WORK,
        ),
        Tool::action(
            "disconnect_popularity",
            "Disconnect popularity source",
            "Disconnect a popularity source and delete its stored credentials. Ask the owner first.",
            s::object(
                vec![("source", s::enumeration(&["apple_ads", "google_ads_planner", "imported"], "Popularity source."))],
                &["source"],
            ),
            DESTRUCTIVE,
            "popularity.disconnect",
            &[],
            false,
        ),
    ]
}

pub async fn run(bridge: &Bridge, name: &str, args: &Value) -> Option<Result<Value, BridgeError>> {
    let r = match name {
        "connect_console" => connect_console(bridge, args).await,
        "connect_popularity" => connect_popularity(bridge, args).await,
        _ => return None,
    };
    Some(r)
}

/// Folders under the home directory the agent may point at. Mirrors
/// `engine::file_policy` (the bridge does not link the engine).
const ALLOWED_DIRS: &[&str] = &["Downloads", "Desktop", "Documents"];
/// File types the agent may point at (imports and credential files).
const ALLOWED_EXTENSIONS: &[&str] = &["csv", "tsv", "txt", "p8", "pem", "json"];

/// Which local files an agent may make the bridge read: regular files of an
/// allowed type inside the user's Downloads / Desktop / Documents, after
/// resolving symlinks, below a size cap.
#[derive(Debug, Clone)]
pub struct FilePolicy {
    home: Option<PathBuf>,
}

impl FilePolicy {
    pub fn new(home: Option<PathBuf>) -> Self {
        Self { home }
    }

    /// The policy for the bridge's configured home folder.
    pub fn for_bridge(bridge: &Bridge) -> Self {
        Self::new(bridge.config().home_dir.clone())
    }

    /// Validate `path`; returns the resolved file to read.
    pub fn check(&self, path: &str, max: u64) -> Result<PathBuf, BridgeError> {
        let home = self
            .home
            .as_deref()
            .ok_or_else(|| BridgeError::Invalid("file access is unavailable (home folder unknown)".into()))?;
        let given = Path::new(path);
        if path.contains('\0') || !given.is_absolute() {
            return Err(BridgeError::Invalid("pass an absolute file path".into()));
        }
        let resolved = std::fs::canonicalize(given)
            .map_err(|e| BridgeError::Invalid(format!("cannot read {path}: {e}")))?;
        let inside = ALLOWED_DIRS
            .iter()
            .filter_map(|d| std::fs::canonicalize(home.join(d)).ok())
            .any(|root| resolved.starts_with(root));
        if !inside {
            return Err(BridgeError::Invalid(format!(
                "for safety only files in the owner's {} folders can be used; ask them to move the file there or to import it in the app",
                ALLOWED_DIRS.join(" / ")
            )));
        }
        let ext = resolved
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        if !ALLOWED_EXTENSIONS.contains(&ext.as_str()) {
            return Err(BridgeError::Invalid(format!(
                "only .{} files can be used",
                ALLOWED_EXTENSIONS.join(", .")
            )));
        }
        let meta = std::fs::metadata(&resolved)
            .map_err(|e| BridgeError::Invalid(format!("cannot read {path}: {e}")))?;
        if !meta.is_file() {
            return Err(BridgeError::Invalid(format!("{path} is not a file")));
        }
        if meta.len() > max {
            return Err(BridgeError::Invalid(format!(
                "{path} is too large ({} bytes, max {max})",
                meta.len()
            )));
        }
        Ok(resolved)
    }

    /// Read a checked file (size-capped while reading, too).
    pub fn read(&self, path: &str, max: u64) -> Result<String, BridgeError> {
        use std::io::Read;
        let resolved = self.check(path, max)?;
        let file = std::fs::File::open(&resolved)
            .map_err(|e| BridgeError::Invalid(format!("cannot read {path}: {e}")))?;
        let mut buf = String::new();
        file.take(max + 1)
            .read_to_string(&mut buf)
            .map_err(|e| BridgeError::Invalid(format!("cannot read {path}: {e}")))?;
        if buf.len() as u64 > max {
            return Err(BridgeError::Invalid(format!("{path} is too large (max {max} bytes)")));
        }
        Ok(buf)
    }
}

/// `value` from `<inline>` or the file at `<path>`; one of them is required.
fn secret(files: &FilePolicy, args: &Value, inline: &str, path: &str) -> Result<String, BridgeError> {
    if let Some(p) = args.get(path).and_then(Value::as_str) {
        return files.read(p, MAX_SECRET_FILE);
    }
    args.get(inline)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| BridgeError::Invalid(format!("pass {path} (preferred) or {inline}")))
}

fn required(args: &Value, name: &str, ctx: &str) -> Result<Value, BridgeError> {
    args.get(name)
        .filter(|v| v.as_str().is_some_and(|s| !s.trim().is_empty()))
        .cloned()
        .ok_or_else(|| BridgeError::Invalid(format!("{ctx} needs \"{name}\"")))
}

/// Reject arguments that belong to another store / source.
fn only(args: &Value, allowed: &[&str], ctx: &str) -> Result<(), BridgeError> {
    let extra: Vec<&str> = args
        .as_object()
        .map(|o| {
            o.keys()
                .map(String::as_str)
                .filter(|k| !allowed.contains(k))
                .collect()
        })
        .unwrap_or_default();
    if extra.is_empty() {
        Ok(())
    } else {
        Err(BridgeError::Invalid(format!(
            "{ctx} does not take: {}",
            extra.join(", ")
        )))
    }
}

fn console_credentials(args: &Value, files: &FilePolicy) -> Result<Value, BridgeError> {
    let mut c = Map::new();
    match args["store"].as_str() {
        Some("apple") => {
            only(
                args,
                &[
                    "store",
                    "label",
                    "keyId",
                    "issuerId",
                    "privateKeyPath",
                    "privateKeyPem",
                ],
                "apple",
            )?;
            c.insert("store".into(), json!("apple"));
            c.insert("keyId".into(), required(args, "keyId", "apple")?);
            c.insert("issuerId".into(), required(args, "issuerId", "apple")?);
            c.insert(
                "privateKeyPem".into(),
                json!(secret(files, args, "privateKeyPem", "privateKeyPath")?),
            );
        }
        Some("google") => {
            only(
                args,
                &[
                    "store",
                    "label",
                    "serviceAccountPath",
                    "serviceAccountJson",
                    "packageNames",
                ],
                "google",
            )?;
            c.insert("store".into(), json!("google"));
            let sa = secret(files, args, "serviceAccountJson", "serviceAccountPath")?;
            if serde_json::from_str::<Value>(&sa).is_err() {
                return Err(BridgeError::Invalid(
                    "the service account is not valid JSON".into(),
                ));
            }
            c.insert("serviceAccountJson".into(), json!(sa));
            c.insert(
                "packageNames".into(),
                args.get("packageNames").cloned().unwrap_or(json!([])),
            );
        }
        _ => return Err(BridgeError::Invalid("store must be apple or google".into())),
    }
    if let Some(l) = args.get("label") {
        c.insert("label".into(), l.clone());
    }
    Ok(Value::Object(c))
}

async fn connect_console(bridge: &Bridge, args: &Value) -> Result<Value, BridgeError> {
    let credentials = console_credentials(args, &FilePolicy::for_bridge(bridge))?;
    bridge
        .dispatch(json!({ "type": "console.connect", "store": args["store"], "credentials": credentials }))
        .await?;
    let accounts = bridge
        .call("console_accounts", json!({}))
        .await
        .map(crate::summarize::prune)
        .unwrap_or(Value::Null);
    Ok(
        json!({ "ok": true, "accounts": accounts, "next": "list_console_apps shows the apps; add_console_apps tracks them." }),
    )
}

fn popularity_credentials(args: &Value, files: &FilePolicy) -> Result<Value, BridgeError> {
    let source = args["source"].as_str().unwrap_or_default();
    let mut c = Map::new();
    c.insert("source".into(), json!(source));
    match source {
        "apple_ads" => {
            only(
                args,
                &[
                    "source",
                    "clientId",
                    "teamId",
                    "keyId",
                    "privateKeyPath",
                    "privateKeyPem",
                    "orgId",
                ],
                source,
            )?;
            for k in ["clientId", "teamId", "keyId"] {
                c.insert(k.into(), required(args, k, source)?);
            }
            c.insert(
                "privateKeyPem".into(),
                json!(secret(files, args, "privateKeyPem", "privateKeyPath")?),
            );
            if let Some(o) = args.get("orgId") {
                c.insert("orgId".into(), o.clone());
            }
        }
        "google_ads_planner" => {
            only(
                args,
                &[
                    "source",
                    "developerToken",
                    "customerId",
                    "refreshToken",
                    "oauthClientId",
                    "oauthClientSecret",
                    "loginCustomerId",
                ],
                source,
            )?;
            for k in [
                "developerToken",
                "customerId",
                "refreshToken",
                "oauthClientId",
                "oauthClientSecret",
            ] {
                c.insert(k.into(), required(args, k, source)?);
            }
            if let Some(l) = args.get("loginCustomerId") {
                c.insert("loginCustomerId".into(), l.clone());
            }
        }
        "imported" => {
            only(args, &["source", "filePath", "store"], source)?;
            let path = required(args, "filePath", source)?;
            let path = path.as_str().unwrap_or_default();
            let name = Path::new(path)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "import".into());
            c.insert("fileName".into(), json!(name));
            c.insert("content".into(), json!(files.read(path, MAX_IMPORT_FILE)?));
            if let Some(st) = args.get("store") {
                c.insert("store".into(), st.clone());
            }
        }
        _ => return Err(BridgeError::Invalid("unknown popularity source".into())),
    }
    Ok(Value::Object(c))
}

async fn connect_popularity(bridge: &Bridge, args: &Value) -> Result<Value, BridgeError> {
    let credentials = popularity_credentials(args, &FilePolicy::for_bridge(bridge))?;
    bridge
        .dispatch(json!({ "type": "popularity.connect", "source": args["source"], "credentials": credentials }))
        .await?;
    let status = bridge
        .call("popularity_status", json!({}))
        .await
        .map(crate::summarize::prune)
        .unwrap_or(Value::Null);
    Ok(
        json!({ "ok": true, "sources": status, "next": "Call fetch_popularity for a project to pull real popularity." }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fake home with the allowed folders.
    fn home() -> (tempfile::TempDir, FilePolicy) {
        let dir = tempfile::tempdir().unwrap();
        for d in ALLOWED_DIRS {
            std::fs::create_dir_all(dir.path().join(d)).unwrap();
        }
        let policy = FilePolicy::new(Some(dir.path().to_path_buf()));
        (dir, policy)
    }

    #[test]
    fn apple_console_needs_key_and_rejects_google_fields() {
        let (_h, files) = home();
        let ok = console_credentials(
            &json!({"store":"apple","keyId":"K","issuerId":"I","privateKeyPem":"P"}),
            &files,
        )
        .unwrap();
        assert_eq!(ok["privateKeyPem"], "P");
        assert!(console_credentials(&json!({"store":"apple","keyId":"K","issuerId":"I"}), &files).is_err());
        assert!(console_credentials(&json!({"store":"apple","keyId":"K","issuerId":"I","privateKeyPem":"P","packageNames":[]}), &files).is_err());
    }

    #[test]
    fn reads_secrets_from_files() {
        let (dir, files) = home();
        let p = dir.path().join("Downloads/sa.json");
        std::fs::write(&p, r#"{"type":"service_account"}"#).unwrap();
        let c = console_credentials(
            &json!({"store":"google","serviceAccountPath": p.to_string_lossy()}),
            &files,
        )
        .unwrap();
        assert!(c["serviceAccountJson"]
            .as_str()
            .unwrap()
            .contains("service_account"));
        assert_eq!(c["packageNames"], json!([]));
    }

    #[test]
    fn google_ads_requires_all_tokens() {
        let (_h, files) = home();
        let r = popularity_credentials(
            &json!({"source":"google_ads_planner","developerToken":"d","customerId":"c"}),
            &files,
        );
        assert!(r.is_err());
    }

    #[test]
    fn refuses_files_outside_the_allowed_folders() {
        let (dir, files) = home();
        let secret = dir.path().join(".ssh/id.json");
        std::fs::create_dir_all(secret.parent().unwrap()).unwrap();
        std::fs::write(&secret, "{}").unwrap();
        let r = console_credentials(
            &json!({"store":"google","serviceAccountPath": secret.to_string_lossy()}),
            &files,
        );
        assert!(r.is_err());
        let r = popularity_credentials(
            &json!({"source":"imported","filePath": secret.to_string_lossy()}),
            &files,
        );
        assert!(r.is_err());
        assert!(files.read("Downloads/a.csv", 10).is_err(), "relative");
    }

    #[test]
    fn refuses_bad_types_large_files_and_escaping_symlinks() {
        let (dir, files) = home();
        let dl = dir.path().join("Downloads");
        std::fs::write(dl.join("x.sh"), "x").unwrap();
        assert!(files.read(&dl.join("x.sh").to_string_lossy(), 10).is_err());
        std::fs::write(dl.join("big.csv"), "x".repeat(100)).unwrap();
        assert!(files.read(&dl.join("big.csv").to_string_lossy(), 10).is_err());
        std::fs::write(dl.join("ok.csv"), "a,b").unwrap();
        assert_eq!(files.read(&dl.join("ok.csv").to_string_lossy(), 10).unwrap(), "a,b");
        #[cfg(unix)]
        {
            let outside = dir.path().join("private.txt");
            std::fs::write(&outside, "secret").unwrap();
            let link = dl.join("innocent.csv");
            std::os::unix::fs::symlink(&outside, &link).unwrap();
            assert!(files.read(&link.to_string_lossy(), 100).is_err());
        }
    }
}
