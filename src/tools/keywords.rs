//! Keywords, per-market phrases, rank checks and popularity fetches.

use serde_json::{json, Value};

use super::schema as s;
use super::{Tool, DESTRUCTIVE, SET, WORK, WRITE};
use crate::error::BridgeError;
use crate::ipc::Bridge;

const MAX_BATCH: u64 = 100;

pub fn tools() -> Vec<Tool> {
    vec![
        Tool::custom(
            "add_keywords",
            "Add keywords",
            "Add one or more keywords (search phrases) to a project. section 'listing' = phrases the listing targets now; 'idea' = candidates to evaluate (default). localize=true asks the AI to translate each phrase into every tracked market's search language (otherwise the same phrase is used everywhere). New keywords have no ranks until run_rank_check.",
            s::object(
                vec![
                    ("projectId", s::project_id()),
                    ("keywords", s::array(s::text("Search phrase.", 1, 100), "Phrases to add.", 1, MAX_BATCH)),
                    ("section", s::enumeration(&["listing", "idea"], "Board section (default idea).")),
                    ("localize", s::boolean("Translate per market with AI (default false).")),
                ],
                &["projectId", "keywords"],
            ),
            WRITE,
        ),
        Tool::custom(
            "edit_keyword",
            "Edit keyword",
            "Rename a keyword and/or hide it (hidden keywords stay tracked but leave the active board) or unhide it.",
            {
                let mut o = s::object(
                    vec![
                        ("keywordId", s::keyword_id()),
                        ("label", s::text("New label.", 1, 100)),
                        ("hidden", s::boolean("true = hide, false = show again.")),
                    ],
                    &["keywordId"],
                );
                o["anyOf"] = json!([{ "required": ["label"] }, { "required": ["hidden"] }]);
                o
            },
            SET,
        ),
        Tool::custom(
            "remove_keywords",
            "Delete keywords",
            "Permanently delete keywords and their rank history. Prefer edit_keyword hidden:true unless the owner wants them gone.",
            s::object(
                vec![("keywordIds", s::array(s::keyword_id(), "Keywords to delete.", 1, MAX_BATCH))],
                &["keywordIds"],
            ),
            DESTRUCTIVE,
        ),
        Tool::action(
            "dont_target",
            "Don't target a phrase",
            "Put a phrase on the project's \"don't target\" list (with an optional note, e.g. a rival's brand), or take it off with remove:true. Listed phrases are left out of every AI step (suggestions, listing options, localization, screenshot headings) and of drafts; a tracked keyword with that label is hidden. get_project_board lists them under dontTarget.",
            s::object(
                vec![
                    ("projectId", s::project_id()),
                    ("phrase", s::text("Search phrase.", 1, 200)),
                    ("note", s::text("Why (optional).", 0, 500)),
                    ("remove", s::boolean("true = take the phrase off the list.")),
                ],
                &["projectId", "phrase"],
            ),
            SET,
            "keyword.avoid",
            &[],
            false,
        ),
        Tool::action(
            "localize_keyword",
            "Localize keyword",
            "Have the AI translate a keyword into the search language of each market (or the given markets), replacing non-overridden per-market phrases.",
            s::object(
                vec![
                    ("keywordId", s::keyword_id()),
                    ("markets", s::markets("Only these markets (default: all tracked).")),
                ],
                &["keywordId"],
            ),
            WORK,
            "keyword.localize",
            &[("keywordId", "id")],
            true,
        ),
        Tool::action(
            "suggest_keywords",
            "AI keyword suggestions",
            "Ask the AI for new keyword ideas for a project based on its listing, brief and current board; they land in the 'idea' section.",
            s::object(
                vec![
                    ("projectId", s::project_id()),
                    ("count", s::integer("How many ideas (default chosen by the app).", 1, 100)),
                    ("hint", s::text("Focus for the ideas, e.g. \"budgeting for students\".", 1, 1000)),
                ],
                &["projectId"],
            ),
            WORK,
            "keywords.suggest",
            &[],
            true,
        ),
        Tool::action(
            "set_market_phrase",
            "Set phrase for one market",
            "Override the exact search phrase used for a keyword in one market (e.g. a better local wording). An empty phrase removes the override.",
            s::object(
                vec![
                    ("keywordId", s::keyword_id()),
                    ("market", s::country()),
                    ("phrase", s::text("Phrase to search in that market; empty = remove override.", 0, 100)),
                ],
                &["keywordId", "market", "phrase"],
            ),
            SET,
            "term.set",
            &[],
            false,
        ),
        Tool::action(
            "run_rank_check",
            "Check ranks",
            "Search the store for every keyword in the project's markets and record ranks, estimated volume/difficulty and competitors. Runs as a background job (the store is rate-limited, so large boards take minutes). Results already checked today are reused unless force is true.",
            s::object(
                vec![
                    ("projectId", s::project_id()),
                    ("markets", s::markets("Only these markets (default: all tracked).")),
                    ("force", s::boolean("Re-check even if already checked today.")),
                ],
                &["projectId"],
            ),
            WORK,
            "check.run",
            &[],
            true,
        ),
        Tool::action(
            "fetch_popularity",
            "Fetch real popularity",
            "Pull REAL search popularity for the project's keywords from the connected source (Apple Ads popularity or Google Ads Keyword Planner). Separate from rank checks and from ASOMaster's estimates; requires a connected source (get_popularity_status).",
            s::object(
                vec![
                    ("projectId", s::project_id()),
                    ("markets", s::markets("Only these markets (default: all tracked).")),
                ],
                &["projectId"],
            ),
            WORK,
            "popularity.fetch",
            &[],
            true,
        ),
    ]
}

pub async fn run(bridge: &Bridge, name: &str, args: &Value) -> Option<Result<Value, BridgeError>> {
    let r = match name {
        "add_keywords" => add_keywords(bridge, args).await,
        "edit_keyword" => edit_keyword(bridge, args).await,
        "remove_keywords" => remove_keywords(bridge, args).await,
        _ => return None,
    };
    Some(r)
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(|s| s.trim().to_string()))
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// Errors that will fail every item: stop and report instead of repeating.
fn is_fatal(e: &BridgeError) -> bool {
    !matches!(e, BridgeError::Api(a) if matches!(a.code.as_str(), "invalid_input" | "conflict" | "not_found"))
}

/// Dispatch one action per item, collecting per-item failures.
async fn each(
    bridge: &Bridge,
    items: Vec<String>,
    make: impl Fn(&str) -> Value,
) -> Result<(Vec<String>, Vec<Value>), BridgeError> {
    let mut ok = Vec::new();
    let mut failed = Vec::new();
    for item in items {
        match bridge.dispatch(make(&item)).await {
            Ok(_) => ok.push(item),
            Err(e) if is_fatal(&e) && ok.is_empty() && failed.is_empty() => return Err(e),
            Err(e) => {
                let stop = is_fatal(&e);
                failed.push(json!({ "item": item, "error": e.to_string() }));
                if stop {
                    break;
                }
            }
        }
    }
    Ok((ok, failed))
}

async fn add_keywords(bridge: &Bridge, args: &Value) -> Result<Value, BridgeError> {
    let project_id = args["projectId"].clone();
    let section = args
        .get("section")
        .and_then(Value::as_str)
        .unwrap_or("idea");
    let localize = args
        .get("localize")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let (added, failed) = each(bridge, strings(&args["keywords"]), |label| {
        json!({ "type": "keyword.add", "projectId": project_id, "label": label, "localize": localize, "section": section })
    })
    .await?;
    Ok(json!({
        "ok": failed.is_empty(),
        "added": added,
        "failed": failed,
        "next": "Keyword ids appear in get_project_board. Run run_rank_check to get ranks and estimates."
    }))
}

async fn edit_keyword(bridge: &Bridge, args: &Value) -> Result<Value, BridgeError> {
    let id = args["keywordId"].clone();
    let mut done = Vec::new();
    if let Some(label) = args.get("label") {
        bridge
            .dispatch(json!({ "type": "keyword.update", "id": id, "label": label }))
            .await?;
        done.push("keyword.update");
    }
    if let Some(hidden) = args.get("hidden") {
        bridge
            .dispatch(json!({ "type": "keyword.hide", "id": id, "hidden": hidden }))
            .await?;
        done.push("keyword.hide");
    }
    Ok(json!({ "ok": true, "actions": done }))
}

async fn remove_keywords(bridge: &Bridge, args: &Value) -> Result<Value, BridgeError> {
    let (removed, failed) = each(
        bridge,
        strings(&args["keywordIds"]),
        |id| json!({ "type": "keyword.remove", "id": id }),
    )
    .await?;
    Ok(json!({ "ok": failed.is_empty(), "removed": removed, "failed": failed }))
}
