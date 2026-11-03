//! App audit: read it (compact for every country but one), run it with
//! options, and dismiss / restore competitors.

use serde_json::{json, Map, Value};

use super::schema as s;
use super::{Tool, READ, SET, WORK};
use crate::error::BridgeError;
use crate::ipc::Bridge;

pub fn tools() -> Vec<Tool> {
    vec![
        Tool::custom(
            "get_audit",
            "App audit",
            "Where the app stands per tracked country and overall, 0-100 (countries weighted by market potential): rankings (visibility of each checked phrase's rank, weighted by demand), listing (how well the country's own listing holds the searched words; localized says whether it is in the country's language), competition (how beatable the apps ranked above are), a verdict (pivot = change keywords, align = fine-tune, fine), deterministic issues with the step that fixes each (action: keywords, names, names_localize, copy, copy_localize, screens, screens_localize, store), the top-10 and gap phrases, main competitors, and the AI conclusions (headline, reason, takeaways) of the last run_audit plus a cross-country summary. One country is returned in full (country, default the main one); the others as one line each. Demand is real popularity when connected (demandSource), else estimated volume.",
            s::object(vec![("projectId", s::project_id()), ("country", s::country())], &["projectId"]),
            READ,
        ),
        Tool::action(
            "run_audit",
            "Run the app audit",
            "Queue an audit run: optionally re-read the store listing (refetch), let the AI research more keywords (moreKeywords, steered by note, else toward the main competitors), check phrases (only never-checked ones unless recheck), then write AI conclusions for the main country first and the other picked countries, and a cross-country summary. Countries left out keep their last audit. Results are saved per country as they finish; an interrupted run resumes when the app restarts. Then wait_for_jobs and get_audit.",
            s::object(
                vec![
                    ("projectId", s::project_id()),
                    (
                        "options",
                        s::object(
                            vec![
                                ("countries", s::markets("Countries to audit (default: every tracked market).")),
                                ("refetch", s::boolean("Re-read the store listing first (default false).")),
                                ("moreKeywords", s::boolean("Research more keywords with the AI first (default false).")),
                                ("recheck", s::boolean("Re-check every phrase, not only never-checked ones (default false).")),
                                ("note", s::text("Owner note for the AI: focus, constraints, context.", 1, 2000)),
                                ("useAi", s::boolean("Write AI conclusions (default true); false = scores, issues and lists only.")),
                                ("ai", s::ai_provider()),
                            ],
                            &[],
                        ),
                    ),
                ],
                &["projectId"],
            ),
            WORK,
            "audit.run",
            &[],
            true,
        ),
        Tool::custom(
            "dismiss_audit_competitor",
            "Dismiss or restore an audit competitor",
            "Leave an app out of the audit's competitor lists and the AI's evidence (it still counts in rankings and competition), or bring it back with restore. appId is the store id from get_audit competitors[].app.id.",
            s::object(
                vec![
                    ("projectId", s::project_id()),
                    ("appId", s::text("Store app id of the competitor.", 1, 200)),
                    ("restore", s::boolean("Bring a dismissed app back (default false).")),
                ],
                &["projectId", "appId"],
            ),
            SET,
        ),
    ]
}

/// One line per country that is not shown in full.
fn compact(c: &Value) -> Value {
    let mut out = Map::new();
    for k in ["country", "name", "potential", "main", "localized", "total", "verdict", "gapCount"] {
        out.insert(k.into(), c[k].clone());
    }
    for part in ["rankings", "listing", "competition"] {
        out.insert(part.into(), c[part]["score"].clone());
    }
    out.insert("issues".into(), json!(c["issues"].as_array().map_or(0, Vec::len)));
    out.insert("headline".into(), c["conclusion"]["headline"].clone());
    Value::Object(out)
}

/// The view with one country in full and the rest compact.
pub fn shape(mut view: Value, country: Option<&str>) -> Value {
    let wanted = country.map(str::to_lowercase).or_else(|| view["mainCountry"].as_str().map(str::to_string));
    if let Some(list) = view.get_mut("countries").and_then(Value::as_array_mut) {
        let full = list.iter().position(|c| c["country"].as_str() == wanted.as_deref()).unwrap_or(0);
        let shaped: Vec<Value> = list.iter().enumerate().map(|(i, c)| if i == full { c.clone() } else { compact(c) }).collect();
        *list = shaped;
    }
    view
}

pub async fn run(bridge: &Bridge, name: &str, args: &Value) -> Option<Result<Value, BridgeError>> {
    match name {
        "get_audit" => Some(
            bridge
                .call("get_audit", json!({ "projectId": args["projectId"] }))
                .await
                .map(|v| shape(v, args.get("country").and_then(Value::as_str))),
        ),
        "dismiss_audit_competitor" => Some(dismiss(bridge, args).await),
        _ => None,
    }
}

async fn dismiss(bridge: &Bridge, args: &Value) -> Result<Value, BridgeError> {
    let restore = args.get("restore").and_then(Value::as_bool).unwrap_or(false);
    let ty = if restore { "audit.restore" } else { "audit.dismiss" };
    let action = json!({ "type": ty, "projectId": args["projectId"], "appId": args["appId"] });
    bridge.dispatch(action).await?;
    Ok(json!({ "ok": true, "action": ty }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_country_full_the_rest_compact() {
        let c = |cc: &str| json!({"country": cc, "name": cc, "total": 50, "rankings": {"score": 40}, "issues": [{}, {}], "top10": [1], "conclusion": {"headline": "h"}});
        let view = json!({"mainCountry": "us", "countries": [c("us"), c("de")]});
        let main = shape(view.clone(), None);
        assert!(main["countries"][0]["top10"].is_array());
        assert_eq!(main["countries"][1]["rankings"], 40);
        assert_eq!(main["countries"][1]["issues"], 2);
        assert!(main["countries"][1].get("top10").is_none());
        let de = shape(view, Some("DE"));
        assert!(de["countries"][1]["top10"].is_array() && de["countries"][0].get("top10").is_none());
    }
}
