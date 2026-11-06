//! Store reviews of the owner's apps: read the inbox, sync it and draft a
//! reply with the AI. Sending or deleting a reply is deliberately not a tool:
//! the owner sends each reply in the app after confirming the exact text.

use serde_json::{json, Map, Value};

use super::schema as s;
use super::{Tool, READ, WORK};
use crate::error::BridgeError;
use crate::ipc::Bridge;

/// Reviews returned when the agent sets no limit (the inbox can be large).
pub const DEFAULT_LIMIT: u64 = 50;

pub fn tools() -> Vec<Tool> {
    vec![
        Tool::custom(
            "get_reviews",
            "Store reviews inbox",
            "Customer reviews of one of the owner's apps, read from App Store Connect / Google Play Console by sync_reviews and kept locally (Google Play only shares about the last 7 days, so history builds up with each sync). Returns the summary for the period (count, average stars, unreplied, reviews per star, frequent phrases in 1-2 star vs 4-5 star reviews), markets (App Store countries or Play reviewer languages), keyword ideas (frequent review phrases no tracked keyword covers; add winners with add_keywords section idea), and the reviews matching the filters: stars, title, text, market, app version, current reply and the owner's draft. state says whether reviews were synced (not_loaded, ready, error with message, disconnected). Replies are sent only by the owner in the app.",
            s::object(
                vec![
                    ("projectId", s::project_id()),
                    ("days", s::integer("Period in days (written in the last N days); 0 = all (default 90).", 0, 3650)),
                    ("stars", s::array(s::integer("Star rating.", 1, 5), "Only these star ratings.", 1, 5)),
                    ("market", s::text("Country code (App Store) or reviewer language (Google Play).", 1, 32)),
                    ("replied", s::enumeration(&["all", "replied", "unreplied"], "Filter by reply (default all).")),
                    ("search", s::text("Words that must appear in the title or text.", 1, 200)),
                    ("limit", s::integer("Most reviews returned (default 50).", 1, 1000)),
                ],
                &["projectId"],
            ),
            READ,
        ),
        Tool::action(
            "sync_reviews",
            "Sync store reviews",
            "Queue a background read of new customer reviews (and their current replies) from the developer console of one of the owner's apps. The app also does this by itself once a day. Then wait_for_jobs and get_reviews.",
            s::object(vec![("projectId", s::project_id())], &["projectId"]),
            WORK,
            "reviews.sync",
            &[],
            true,
        ),
        Tool::action(
            "draft_review_reply",
            "Draft a review reply",
            "Queue an AI draft of the public reply to one review (reviewId from get_reviews): in the reviewer's language, within the store's limit (App Store 5,970 characters, Google Play 350), polite and specific, no refunds promised, no personal data, no links except the owner's support link. The draft is saved on the review for the owner to edit; nothing is posted. Only the owner can send a reply, in the app. Then wait_for_jobs and get_reviews.",
            s::object(
                vec![
                    ("projectId", s::project_id()),
                    ("reviewId", s::text("Review id from get_reviews.", 1, 256)),
                    ("directions", s::directions()),
                    ("ai", s::ai_provider()),
                ],
                &["projectId", "reviewId"],
            ),
            WORK,
            "reviews.draft",
            &[],
            true,
        ),
    ]
}

/// `get_reviews` arguments → the app's `{projectId, query}`.
fn query(args: &Value) -> Value {
    let mut q = Map::new();
    q.insert(
        "days".into(),
        args.get("days").cloned().unwrap_or(json!(90)),
    );
    q.insert(
        "limit".into(),
        args.get("limit").cloned().unwrap_or(json!(DEFAULT_LIMIT)),
    );
    for key in ["stars", "market", "replied", "search"] {
        if let Some(v) = args.get(key) {
            q.insert(key.into(), v.clone());
        }
    }
    json!({ "projectId": args["projectId"], "query": Value::Object(q) })
}

pub async fn run(bridge: &Bridge, name: &str, args: &Value) -> Option<Result<Value, BridgeError>> {
    match name {
        "get_reviews" => Some(bridge.call("get_reviews", query(args)).await),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_defaults_to_90_days_and_50_reviews() {
        let q = query(&json!({"projectId": "3", "stars": [1, 2], "replied": "unreplied"}));
        assert_eq!(
            q,
            json!({"projectId": "3", "query": {"days": 90, "limit": 50, "stars": [1, 2], "replied": "unreplied"}})
        );
    }

    #[test]
    fn sending_a_reply_is_not_a_tool() {
        let names: Vec<&str> = tools().iter().map(|t| t.name).collect();
        assert_eq!(
            names,
            vec!["get_reviews", "sync_reviews", "draft_review_reply"]
        );
    }
}
