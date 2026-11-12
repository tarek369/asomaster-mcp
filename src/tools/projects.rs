//! Projects (tracked apps, competitors, brainstorm sessions) and folders.

use serde_json::{json, Value};

use super::schema as s;
use super::{Tool, DESTRUCTIVE, SET, WORK, WRITE};
use crate::error::BridgeError;
use crate::ipc::Bridge;

const PROJECT_ID: &[(&str, &str)] = &[("projectId", "id")];

pub fn tools() -> Vec<Tool> {
    vec![
        Tool::action(
            "add_app",
            "Track an app",
            "Start tracking an app as a project: your own app (kind mine) or a competitor. Accepts an App Store / Play id, bundle / package name or store URL (find it with search_store). Imports the live listing and seeds keywords in the background; the new project appears in get_overview when the job finishes.",
            s::object(
                vec![
                    ("store", s::store()),
                    ("app", s::text("App id, package name or store URL.", 1, 500)),
                    ("kind", s::enumeration(&["mine", "competitor"], "mine = an app the owner publishes; competitor = someone else's.")),
                    ("folderId", s::id("Folder to put it in.")),
                    ("markets", s::markets("Markets to track (default: the owner's default markets).")),
                    ("brief", s::text("Product facts, upcoming features and positioning, saved before the first AI brainstorm (keep upcoming features apart from the current listing).", 1, 4000)),
                    ("mainLocale", s::locale("Main listing language for the listing optimizer, e.g. de-DE (default en-US).")),
                    ("ai", s::ai_provider()),
                ],
                &["store", "app", "kind"],
            ),
            WORK,
            "project.add",
            &[("app", "appIdOrUrl")],
            true,
        ),
        Tool::action(
            "link_released_app",
            "Link an app idea to its released app",
            "An app idea (a brainstorm project, no store app yet) is now published: link it to its App Store id / Play package / store URL. It becomes kind mine (or competitor), keeps every keyword and history, and the store listing replaces the idea's name. Fails for projects that already have an app or when the app is already tracked.",
            s::object(
                vec![
                    ("projectId", s::project_id()),
                    ("app", s::text("App id, package name or store URL.", 1, 500)),
                    ("kind", s::enumeration(&["mine", "competitor"], "mine = the owner's app; competitor = someone else's.")),
                ],
                &["projectId", "app", "kind"],
            ),
            WORK,
            "project.link",
            &[("projectId", "id"), ("app", "appIdOrUrl")],
            true,
        ),
        Tool::action(
            "add_console_apps",
            "Track all console apps",
            "Track every app visible in the connected developer console account(s) for a store as 'mine' projects (skips ones already tracked).",
            s::object(vec![("store", s::store())], &["store"]),
            WORK,
            "project.addAll",
            &[],
            true,
        ),
        Tool::action(
            "brainstorm_project",
            "Brainstorm a new app idea",
            "Create a brainstorm project for an app that does not exist yet: the AI proposes keywords from a short brief and checks them in the chosen markets. Requires a ready AI provider.",
            s::object(
                vec![
                    ("store", s::store()),
                    ("brief", s::text("What the app does and for whom.", 3, 4000)),
                    ("markets", s::markets("Markets to evaluate (default: owner defaults).")),
                ],
                &["store", "brief"],
            ),
            WORK,
            "project.brainstorm",
            &[],
            true,
        ),
        Tool::action(
            "update_project",
            "Edit project",
            "Rename a project, change its brief (context used by the AI), its automatic rank-check schedule, or whether it is the owner's app (mine) or a competitor (keeps keywords and history).",
            s::object(
                vec![
                    ("projectId", s::project_id()),
                    ("name", s::text("Display name.", 1, 200)),
                    ("brief", s::text("Short description of the app used by the AI.", 0, 4000)),
                    ("autoCheck", s::enumeration(&["inherit", "off", "daily", "weekly", "biweekly", "monthly"], "Automatic check schedule (inherit = global setting).")),
                    ("kind", s::enumeration(&["mine", "competitor"], "mine = an app the owner publishes; competitor = someone else's.")),
                ],
                &["projectId"],
            ),
            SET,
            "project.update",
            PROJECT_ID,
            false,
        ),
        Tool::action(
            "set_project_markets",
            "Set tracked markets",
            "Replace the list of markets (countries) tracked for a project. New markets are empty until run_rank_check runs for them.",
            s::object(
                vec![
                    ("projectId", s::project_id()),
                    ("markets", s::markets("Full list of markets to track.")),
                ],
                &["projectId", "markets"],
            ),
            SET,
            "project.markets",
            PROJECT_ID,
            false,
        ),
        Tool::action(
            "move_project",
            "Move project",
            "Move a project into a folder (null = top level), optionally placing it before another project.",
            s::object(
                vec![
                    ("projectId", s::project_id()),
                    ("folderId", s::nullable_id("Target folder id, or null for no folder.")),
                    ("beforeId", s::id("Place before this project id.")),
                ],
                &["projectId", "folderId"],
            ),
            SET,
            "project.move",
            PROJECT_ID,
            false,
        ),
        Tool::custom(
            "remove_project",
            "Delete project",
            "Permanently delete a project with its keywords, rank history and optimizer drafts. Only when the owner asked for this exact project: pass its name (from get_overview) as confirmName; a mismatch deletes nothing.",
            s::object(
                vec![
                    ("projectId", s::project_id()),
                    ("confirmName", s::text("The project's exact name, as get_overview lists it.", 1, 200)),
                ],
                &["projectId", "confirmName"],
            ),
            DESTRUCTIVE,
        ),
        Tool::action(
            "resync_project",
            "Refresh app metadata",
            "Re-import the app's live listing / metadata from the store (and console when connected).",
            s::object(vec![("projectId", s::project_id())], &["projectId"]),
            WORK,
            "project.resync",
            PROJECT_ID,
            true,
        ),
        Tool::custom(
            "save_folder",
            "Create or rename folder",
            "Create a folder (omit folderId) or rename an existing one.",
            s::object(
                vec![
                    ("folderId", s::id("Folder to rename; omit to create a new folder.")),
                    ("name", s::text("Folder name.", 1, 200)),
                ],
                &["name"],
            ),
            WRITE,
        ),
        Tool::action(
            "remove_folder",
            "Delete folder",
            "Delete a folder. Its projects are kept and move to the top level.",
            s::object(vec![("folderId", s::id("Folder id."))], &["folderId"]),
            DESTRUCTIVE,
            "folder.remove",
            &[("folderId", "id")],
            false,
        ),
    ]
}

pub async fn run(bridge: &Bridge, name: &str, args: &Value) -> Option<Result<Value, BridgeError>> {
    match name {
        "save_folder" => Some(save_folder(bridge, args).await),
        "remove_project" => Some(remove_project(bridge, args).await),
        _ => None,
    }
}

async fn save_folder(bridge: &Bridge, args: &Value) -> Result<Value, BridgeError> {
    let action = match args.get("folderId").and_then(Value::as_str) {
        Some(id) => json!({ "type": "folder.rename", "id": id, "name": args["name"] }),
        None => json!({ "type": "folder.create", "name": args["name"] }),
    };
    bridge.dispatch(action.clone()).await?;
    Ok(
        json!({ "ok": true, "action": action["type"], "next": "Folder ids are listed by get_overview." }),
    )
}

/// Delete only when the agent names the project it means: a stale or
/// mistyped id must never remove another app (projects are not recoverable).
async fn remove_project(bridge: &Bridge, args: &Value) -> Result<Value, BridgeError> {
    let id = super::arg_str(args, "projectId")?;
    let confirm = super::arg_str(args, "confirmName")?.trim();
    let snapshot = bridge.call("get_snapshot", json!({})).await?;
    let project = snapshot["projects"]
        .as_array()
        .and_then(|ps| ps.iter().find(|p| p["id"].as_str() == Some(id)))
        .ok_or_else(|| BridgeError::Invalid(format!("no project with id \"{id}\"")))?;
    let name = project["name"].as_str().unwrap_or_default().trim();
    if !name.eq_ignore_ascii_case(confirm) {
        return Err(BridgeError::Invalid(format!(
            "project {id} is \"{name}\", not \"{confirm}\"; nothing was deleted"
        )));
    }
    bridge
        .dispatch(json!({ "type": "project.remove", "id": id }))
        .await?;
    Ok(json!({ "ok": true, "action": "project.remove", "removed": { "id": id, "name": name } }))
}
