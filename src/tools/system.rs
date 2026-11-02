//! Settings, AI provider, alerts, import / export.

use serde_json::{json, Value};

use super::connections::FilePolicy;
use super::schema as s;
use super::{queued, Annotations, Tool, DESTRUCTIVE, READ_NET, SET, WORK};
use crate::error::BridgeError;
use crate::ipc::Bridge;
use crate::summarize::truncate;

const MAX_IMPORT_FILE: u64 = 20 * 1024 * 1024;
const DEFAULT_EXPORT_CHARS: u64 = 50_000;

/// May write a file into the app's exports folder.
const EXPORT: Annotations = Annotations {
    read_only: false,
    destructive: false,
    idempotent: true,
    open_world: false,
};

fn settings_schema() -> Value {
    let schedule = s::enumeration(
        &["off", "daily", "weekly", "biweekly", "monthly"],
        "Global automatic check schedule.",
    );
    let markets = |d: &str| s::array(s::country(), d, 0, 200);
    s::object(
        vec![
            ("theme", s::enumeration(&["system", "light", "dark"], "UI theme.")),
            ("checkSchedule", schedule),
            ("checkSpeed", s::enumeration(&["normal", "fast"], "Store request pacing (fast risks throttling).")),
            ("parallelApps", s::integer("Projects checked at the same time.", 1, 4)),
            ("defaultMarkets", s::object(
                vec![
                    ("apple", markets("Default App Store markets for new projects.")),
                    ("google", markets("Default Google Play markets for new projects.")),
                ],
                &[],
            )),
            ("metricDriver", s::enumeration(&["estimate", "popularity"], "Which metric drives colours, verdicts and opportunity: ASOMaster estimate or real popularity.")),
            ("cellDisplay", s::enumeration(&["rank", "estimate", "popularity"], "What board cells show.")),
            ("aiEffort", s::enumeration(&["low", "medium", "high"], "AI reasoning effort.")),
            ("aiConcurrency", s::integer("Parallel AI requests.", 1, 16)),
            ("aiFallback", s::boolean("Fall back to another ready provider on failure.")),
            ("alerts", s::object(
                vec![
                    ("enabled", s::boolean("Rank alerts on.")),
                    ("threshold", s::integer("Minimum rank move to alert on.", 1, 200)),
                    ("osNotifications", s::boolean("Show OS notifications.")),
                    ("onlyTop", s::integer("Only alert within the top N.", 1, 200)),
                ],
                &["enabled", "threshold", "osNotifications", "onlyTop"],
            )),
            ("blurMode", s::boolean("Blur app names for screenshots.")),
            ("updateChannel", s::enumeration(&["stable", "beta"], "App update channel.")),
            ("autoUpdate", s::boolean("Install updates automatically.")),
        ],
        &[],
    )
}

pub fn tools() -> Vec<Tool> {
    vec![
        Tool::action(
            "update_settings",
            "Update settings",
            "Change app settings; only the keys you pass change. Use set_ai_provider for the AI provider/model.",
            s::object(vec![("settings", {
                let mut o = settings_schema();
                o["minProperties"] = json!(1);
                o
            })], &["settings"]),
            SET,
            "settings.save",
            &[],
            false,
        ),
        Tool::action(
            "set_ai_provider",
            "Choose AI provider",
            "Select the local AI provider (and optionally model) the app uses for generation. Check readiness with get_ai_status first.",
            s::object(
                vec![
                    ("provider", s::ai_provider()),
                    ("model", s::text("Model id from get_ai_status (default: provider default).", 1, 200)),
                ],
                &["provider"],
            ),
            SET,
            "ai.select",
            &[],
            false,
        ),
        Tool::action(
            "test_ai_provider",
            "Test AI provider",
            "Send a tiny test prompt through a provider and record latency / errors (see get_ai_status). Refused for providers that bill an API key; the user runs those tests in the app.",
            s::object(vec![("provider", s::ai_provider())], &["provider"]),
            READ_NET,
            "ai.test",
            &[],
            false,
        ),
        Tool::custom(
            "acknowledge_alerts",
            "Mark alerts read / clear",
            "Mark rank alerts as read (given ids, or all when ids is omitted). With clear:true, delete all alerts instead.",
            s::object(
                vec![
                    ("ids", s::array(s::id("Alert id."), "Alerts to mark read (default: all).", 1, 1000)),
                    ("clear", s::boolean("Delete all alerts (ignores ids).")),
                ],
                &[],
            ),
            DESTRUCTIVE,
        ),
        Tool::custom(
            "import_file",
            "Import keywords file",
            "Import an Astro export or a CSV of apps / keywords from a local path (must be a .csv/.tsv/.txt/.json file in the owner's Downloads, Desktop or Documents folder). format astro without path reads Astro's own database on this Mac: its popularity becomes real popularity and each Astro app's phrases join the tracked app with the same App Store id. previewOnly:true just reports what would be imported (rows, apps, keywords, countries). With projectId the phrases are added as keywords of that project; otherwise apps become projects (optionally in folderId). Background job.",
            s::object(
                vec![
                    ("path", s::text("Absolute path of the file (omit with format astro to read Astro's local database).", 1, 1024)),
                    ("format", s::enumeration(&["astro", "csv"], "File format.")),
                    ("previewOnly", s::boolean("Only preview (default false).")),
                    ("folderId", s::id("Folder for imported projects.")),
                    ("projectId", s::id("Add the imported phrases to this project.")),
                    ("store", s::store()),
                ],
                &["format"],
            ),
            WORK,
        ),
        Tool::custom(
            "export_data",
            "Export board / history / optimizer",
            "Export a project's board, rank history or optimizer drafts as CSV or JSON. By default returns the content (cut at maxChars); with saveToFile:true writes the file into the app's exports folder instead. format fastlane (what optimizer only) always writes a zip of fastlane deliver / supply metadata to the exports folder.",
            s::object(
                vec![
                    ("projectId", s::project_id()),
                    ("what", s::enumeration(&["board", "history", "optimizer"], "What to export.")),
                    ("format", s::enumeration(&["csv", "json", "fastlane"], "File format.")),
                    ("saveToFile", s::boolean("Write to <dataDir>/exports instead of returning content.")),
                    ("maxChars", s::integer("Max characters of content to return (default 50000).", 100, 2_000_000)),
                ],
                &["projectId", "what", "format"],
            ),
            EXPORT,
        ),
        Tool::action(
            "ask_assistant",
            "Ask the ASO assistant",
            "Send a message to the in-app ASO assistant for a project (keywords or optimizer scope). It PROPOSES keyword / listing changes (status proposed) but applies nothing by itself; review them with the owner, then call apply_assistant_changes or dismiss_assistant_changes. Background job; read the reply with get_assistant_thread after wait_for_jobs.",
            s::object(
                vec![
                    ("projectId", s::project_id()),
                    ("text", s::text("Message.", 1, 8000)),
                    ("scope", s::enumeration(&["keywords", "optimizer"], "Conversation scope.")),
                    ("ai", s::ai_provider()),
                ],
                &["projectId", "text", "scope"],
            ),
            WORK,
            "assistant.ask",
            &[],
            true,
        ),
        Tool::action(
            "apply_assistant_changes",
            "Apply assistant proposals",
            "Apply changes the assistant proposed in one of its messages (ids from get_assistant_thread, e.g. \"c0\"; omit changeIds for every still-proposed change). Already applied or dismissed changes are skipped. Only apply what the owner agreed to.",
            s::object(
                vec![
                    ("projectId", s::project_id()),
                    ("messageId", s::id("Assistant message id (get_assistant_thread).")),
                    ("changeIds", s::array(s::id("Change id."), "Changes to apply (default: all proposed).", 1, 100)),
                ],
                &["projectId", "messageId"],
            ),
            SET,
            "assistant.apply",
            &[("changeIds", "actionIds")],
            false,
        ),
        Tool::action(
            "dismiss_assistant_changes",
            "Dismiss assistant proposals",
            "Mark changes the assistant proposed in one of its messages as dismissed (omit changeIds for every still-proposed change). Nothing in the project changes.",
            s::object(
                vec![
                    ("projectId", s::project_id()),
                    ("messageId", s::id("Assistant message id (get_assistant_thread).")),
                    ("changeIds", s::array(s::id("Change id."), "Changes to dismiss (default: all proposed).", 1, 100)),
                ],
                &["projectId", "messageId"],
            ),
            SET,
            "assistant.dismiss",
            &[("changeIds", "actionIds")],
            false,
        ),
        Tool::action(
            "clear_assistant",
            "Clear assistant conversation",
            "Delete the assistant conversation history of a project.",
            s::object(vec![("projectId", s::project_id())], &["projectId"]),
            DESTRUCTIVE,
            "assistant.clear",
            &[],
            false,
        ),
    ]
}

pub async fn run(bridge: &Bridge, name: &str, args: &Value) -> Option<Result<Value, BridgeError>> {
    let r = match name {
        "acknowledge_alerts" => alerts(bridge, args).await,
        "import_file" => import(bridge, args).await,
        "export_data" => export(bridge, args).await,
        _ => return super::jobs::run(bridge, name, args).await,
    };
    Some(r)
}

async fn alerts(bridge: &Bridge, args: &Value) -> Result<Value, BridgeError> {
    if args["clear"] == json!(true) {
        bridge.dispatch(json!({ "type": "alerts.clear" })).await?;
        return Ok(json!({ "ok": true, "action": "alerts.clear" }));
    }
    let mut action = json!({ "type": "alerts.read" });
    if let Some(ids) = args.get("ids") {
        action["ids"] = ids.clone();
    }
    bridge.dispatch(action).await?;
    Ok(json!({ "ok": true, "action": "alerts.read" }))
}

async fn import(bridge: &Bridge, args: &Value) -> Result<Value, BridgeError> {
    let path = args["path"].as_str().unwrap_or_default();
    let local_astro = path.is_empty() && args["format"] == json!("astro");
    if path.is_empty() && !local_astro {
        return Err(BridgeError::Invalid("pass path (only format astro can read Astro's local database without one)".into()));
    }
    if local_astro && args["previewOnly"] == json!(true) {
        let preview = bridge
            .call("import_preview", json!({ "fileName": "", "content": "", "format": "astro" }))
            .await?;
        return Ok(json!({ "preview": crate::summarize::prune(preview) }));
    }
    if args["previewOnly"] == json!(true) {
        let content = FilePolicy::for_bridge(bridge).read(path, MAX_IMPORT_FILE)?;
        let file_name = std::path::Path::new(path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let preview = bridge
            .call(
                "import_preview",
                json!({ "fileName": file_name, "content": content, "format": args["format"] }),
            )
            .await?;
        return Ok(json!({ "preview": crate::summarize::prune(preview) }));
    }
    let mut action = json!({ "type": "import.file", "path": path, "format": args["format"] });
    for k in ["folderId", "projectId", "store"] {
        if let Some(v) = args.get(k) {
            action[k] = v.clone();
        }
    }
    bridge.dispatch(action).await?;
    queued(
        bridge,
        "import.file",
        args.get("projectId").and_then(Value::as_str),
    )
    .await
}

async fn export(bridge: &Bridge, args: &Value) -> Result<Value, BridgeError> {
    let project_id = &args["projectId"];
    let what = args["what"].as_str().unwrap_or("board");
    let format = &args["format"];
    let fastlane = format == &json!("fastlane");
    if fastlane && what != "optimizer" {
        return Err(BridgeError::Invalid("format fastlane exports the listing: use what optimizer".into()));
    }
    // A zip is binary: it always goes to the exports folder.
    if fastlane || args["saveToFile"] == json!(true) {
        let ty = match what {
            "history" => "export.history",
            "optimizer" => "optimizer.export",
            _ => "export.board",
        };
        bridge
            .dispatch(json!({ "type": ty, "projectId": project_id, "format": format }))
            .await?;
        let dir = bridge.config().data_dir.join("exports");
        return Ok(json!({ "ok": true, "action": ty, "folder": dir.to_string_lossy() }));
    }
    let file = bridge
        .call(
            "export_data",
            json!({ "projectId": project_id, "what": what, "format": format }),
        )
        .await?;
    let max = args
        .get("maxChars")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_EXPORT_CHARS) as usize;
    let content = file["content"].as_str().unwrap_or_default();
    Ok(json!({
        "fileName": file["fileName"],
        "mime": file["mime"],
        "totalChars": content.chars().count(),
        "content": truncate(content, max),
    }))
}
