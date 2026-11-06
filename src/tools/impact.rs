//! "Did it work?": the change-impact report, read-only. Every change in one
//! line, one change in full (markets compact unless one is asked for).

use serde_json::{json, Map, Value};

use super::schema as s;
use super::{Tool, READ};
use crate::error::BridgeError;
use crate::ipc::Bridge;

pub fn tools() -> Vec<Tool> {
    vec![Tool::custom(
        "get_change_impact",
        "Did a change work?",
        "Before vs after each change of the app: listing text published through ASOMaster, the app's own listing text changing when re-read, a new App Store version (its release day), and markers the owner added (\"new screenshots\"). For each change and market: tracked keywords ranked / in the top 10 / top 3 and their median rank (same keywords in both windows), the biggest risers and fallers, store traffic per day and conversion (downloads / page views; coverage says when the store has not reported every day yet: App Store Connect lags 1-2 days, Google Play posts monthly) and the average rating, with a verdict: improved, worse, no_clear_change, too_early (the after window has not passed), not_enough_data or waiting_for_release (App Store text sent for a version Apple has not released yet; once released, day is the release day, liveDay says so and sentDay keeps the day it was sent). The change day itself is left out; overlaps lists other changes within the window, whose effects mix in. It shows what happened around a change, not proof that the change caused it. All changes come back as one line each; the one picked by eventId (default the newest) in full with every market as one line, or one market in full with market.",
        s::object(
            vec![
                ("projectId", s::project_id()),
                ("days", json!({"type": "integer", "enum": [7, 14, 28], "description": "Days before and after the change (default 14)."})),
                ("eventId", s::id("The change to show in full (default the newest).")),
                ("market", s::country()),
            ],
            &["projectId"],
        ),
        READ,
    )]
}

/// Old / new field text kept per value (characters).
pub const MAX_TEXT: usize = 200;
/// Field rows kept for the change shown in full ("fieldsMore" counts the rest).
pub const MAX_FIELDS: usize = 20;

fn cut(v: &Value) -> Value {
    match v.as_str() {
        Some(t) if t.chars().count() > MAX_TEXT => {
            Value::String(format!("{}…", t.chars().take(MAX_TEXT).collect::<String>()))
        }
        _ => v.clone(),
    }
}

/// The full change, sized for an agent: field texts cut, at most
/// [`MAX_FIELDS`] rows, and `overall` without the verdict the change repeats.
fn trim_full(e: &mut Value) {
    if let Some(fields) = e.get("fields").and_then(Value::as_array).cloned() {
        let kept: Vec<Value> = fields
            .iter()
            .take(MAX_FIELDS)
            .map(|f| {
                let mut f = f.clone();
                for k in ["from", "to"] {
                    if let Some(v) = f.get(k).map(cut) {
                        f[k] = v;
                    }
                }
                f
            })
            .collect();
        if fields.len() > MAX_FIELDS {
            e["fieldsMore"] = json!(fields.len() - MAX_FIELDS);
        }
        e["fields"] = Value::Array(kept);
    }
    if let Some(overall) = e.get_mut("overall").and_then(Value::as_object_mut) {
        overall.remove("verdict");
    }
}

/// One line for a change that is not shown in full.
fn compact_event(e: &Value) -> Value {
    let mut out = Map::new();
    for k in [
        "id", "kind", "day", "label", "version", "verdict", "complete",
    ] {
        out.insert(k.into(), e[k].clone());
    }
    out.insert(
        "fields".into(),
        json!(e["fields"].as_array().map_or(0, Vec::len)),
    );
    out.insert(
        "overlaps".into(),
        json!(e["overlaps"].as_array().map_or(0, Vec::len)),
    );
    Value::Object(out)
}

/// One line per market: the verdict and the headline numbers.
fn compact_market(m: &Value) -> Value {
    let (r, t, g) = (&m["ranks"], &m["traffic"], &m["ratings"]);
    json!({
        "country": m["country"],
        "verdict": m["verdict"],
        "top10": [r["before"]["top10"], r["after"]["top10"]],
        "ranked": [r["before"]["ranked"], r["after"]["ranked"]],
        "medianRank": [r["before"]["medianRank"], r["after"]["medianRank"]],
        "climbed": r["climbed"],
        "fell": r["fell"],
        "downloadsPerDay": [t["before"]["downloadsPerDay"], t["after"]["downloadsPerDay"]],
        "conversionPct": [t["before"]["conversionPct"], t["after"]["conversionPct"]],
        "trafficCoverage": [t["before"]["coverage"], t["after"]["coverage"]],
        "rating": [g["before"]["average"], g["after"]["average"]],
    })
}

/// The view with one change in full and the rest compact.
pub fn shape(mut view: Value, event_id: Option<&str>, market: Option<&str>) -> Value {
    let market = market.map(str::to_lowercase);
    let Some(events) = view.get_mut("events").and_then(Value::as_array_mut) else {
        return view;
    };
    let full = event_id
        .and_then(|id| events.iter().position(|e| e["id"].as_str() == Some(id)))
        .unwrap_or(0);
    let shaped: Vec<Value> = events
        .iter()
        .enumerate()
        .map(|(i, e)| {
            if i != full {
                return compact_event(e);
            }
            let mut e = e.clone();
            trim_full(&mut e);
            if let Some(markets) = e.get_mut("markets").and_then(Value::as_array_mut) {
                let keep: Vec<Value> = markets
                    .iter()
                    .map(|m| {
                        if m["country"].as_str() == market.as_deref() {
                            m.clone()
                        } else {
                            compact_market(m)
                        }
                    })
                    .collect();
                *markets = keep;
            }
            e
        })
        .collect();
    *events = shaped;
    view
}

pub async fn run(bridge: &Bridge, name: &str, args: &Value) -> Option<Result<Value, BridgeError>> {
    if name != "get_change_impact" {
        return None;
    }
    let params = json!({ "projectId": args["projectId"], "days": args.get("days").cloned().unwrap_or(Value::Null) });
    Some(bridge.call("get_change_impact", params).await.map(|v| {
        shape(
            v,
            args.get("eventId").and_then(Value::as_str),
            args.get("market").and_then(Value::as_str),
        )
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn market(cc: &str) -> Value {
        json!({"country": cc, "verdict": "improved", "ranks": {"before": {"top10": 1}, "after": {"top10": 3}, "climbed": 3, "risers": [{"label": "rain"}]},
               "traffic": {"before": {"downloadsPerDay": 20.0}, "after": {"downloadsPerDay": 33.0}}, "ratings": {}})
    }

    fn event(id: &str) -> Value {
        json!({"id": id, "kind": "manual", "label": "New screenshots", "verdict": "improved", "fields": [{}, {}], "overlaps": [],
               "overall": market(""), "markets": [market("us"), market("gb")]})
    }

    #[test]
    fn one_change_in_full_with_compact_markets() {
        let view = json!({"events": [event("9"), event("4")]});
        let newest = shape(view.clone(), None, None);
        assert!(newest["events"][0]["overall"]["ranks"]["risers"].is_array());
        assert_eq!(newest["events"][0]["markets"][0]["top10"], json!([1, 3]));
        assert!(newest["events"][0]["markets"][0].get("ranks").is_none());
        assert_eq!(newest["events"][1]["fields"], 2);
        assert!(newest["events"][1].get("markets").is_none());
        let picked = shape(view, Some("4"), Some("GB"));
        assert!(picked["events"][0].get("markets").is_none());
        assert!(
            picked["events"][1]["markets"][1]["ranks"]["risers"].is_array(),
            "gb in full"
        );
        assert!(picked["events"][1]["markets"][0].get("ranks").is_none());
    }

    #[test]
    fn a_view_without_events_passes_through() {
        assert_eq!(shape(Value::Null, None, None), Value::Null);
    }

    #[test]
    fn long_texts_and_many_fields_are_cut_and_overall_drops_its_repeated_verdict() {
        let long = "x".repeat(500);
        let fields: Vec<Value> = (0..25).map(|i| json!({"locale": format!("l{i}"), "field": "description", "from": long, "to": "short"})).collect();
        let mut e = event("9");
        e["fields"] = json!(fields);
        let out = shape(json!({"events": [e]}), None, None);
        let f = &out["events"][0]["fields"];
        assert_eq!(f.as_array().unwrap().len(), MAX_FIELDS);
        assert_eq!(out["events"][0]["fieldsMore"], 5);
        let from = f[0]["from"].as_str().unwrap();
        assert!(
            from.chars().count() <= MAX_TEXT + 1 && from.ends_with('…'),
            "{from}"
        );
        assert_eq!(f[0]["to"], "short");
        assert!(
            out["events"][0]["overall"].get("verdict").is_none(),
            "same as the event's verdict"
        );
        assert!(out["events"][0]["overall"]["ranks"].is_object());
    }
}
