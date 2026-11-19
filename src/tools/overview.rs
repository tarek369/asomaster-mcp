//! The dashboard across all apps and one app's insights, read-only. The
//! daily trend is thinned to one point a week to keep answers small.

use serde_json::{json, Value};

use super::schema as s;
use super::{Tool, READ};
use crate::error::BridgeError;
use crate::ipc::Bridge;

/// Trend points kept: every this many days, plus today.
pub const TREND_STEP: usize = 7;

pub fn tools() -> Vec<Tool> {
    vec![
        Tool::custom(
            "get_dashboard",
            "Dashboard: every app at a glance",
            "Every tracked app of both stores with its search measures today over all tracked markets together: visibility (0-100, the demand-weighted share of search attention on the audit's rank curve; whole points from 1 up, one decimal below, at least 0.1 when any tracked search shows the app and 0 only when none does; null without the scoring bundle or before release), searches (phrase x market) in the top 10 and top 3, average rank of the ranked ones, the latest audit score, and how each moved over 7, 30 and 90 days (null when nothing was checked back then; avgRank negative = better). Each phrase keeps its last checked rank between checks. Own apps with store analytics connected add downloads and conversion (downloads / page views) per period, ending on the last day the store reported, with the change vs the period before (null when the store reported under 80% of those days). trend: weekly points of the last 90 days. Local data.",
            s::object(vec![], &[]),
            READ,
        ),
        Tool::custom(
            "get_app_insights",
            "App insights: near the top 10 and biggest moves",
            "One app: the same measures as get_dashboard now and per market, phrases ranked 11-30 with demand (most worth a push first: demand x closeness to the top 10), and the biggest climbs and falls over the period (by search attention won or lost; change = places, not ranked counts as 251). demand is REAL popularity when demandSource is popularity, else the ESTIMATED volume. Use keywordId + country with get_term_detail. Local data.",
            s::object(
                vec![
                    ("projectId", s::project_id()),
                    ("days", json!({"type": "integer", "enum": [7, 30], "description": "Period for the moves and changes (default 7)."})),
                ],
                &["projectId"],
            ),
            READ,
        ),
    ]
}

/// Keeps one trend point a week, ending on today's.
fn thin(trend: &mut Value) {
    let Some(points) = trend.as_array() else { return };
    let last = points.len().saturating_sub(1);
    let kept: Vec<Value> = points
        .iter()
        .enumerate()
        .filter(|(i, _)| (last - i) % TREND_STEP == 0)
        .map(|(_, p)| p.clone())
        .collect();
    *trend = Value::Array(kept);
}

pub fn shape_dashboard(mut view: Value) -> Value {
    if let Some(apps) = view.get_mut("apps").and_then(Value::as_array_mut) {
        apps.iter_mut().filter_map(|a| a.get_mut("trend")).for_each(thin);
    }
    view
}

pub fn shape_insights(mut view: Value) -> Value {
    if let Some(t) = view.get_mut("trend") {
        thin(t);
    }
    view
}

pub async fn run(bridge: &Bridge, name: &str, args: &Value) -> Option<Result<Value, BridgeError>> {
    match name {
        "get_dashboard" => Some(bridge.call("get_dashboard", json!({})).await.map(shape_dashboard)),
        "get_app_insights" => {
            let params = json!({ "projectId": args["projectId"], "days": args.get("days").cloned().unwrap_or(Value::Null) });
            Some(bridge.call("get_app_insights", params).await.map(shape_insights))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trend(n: usize) -> Value {
        Value::Array((0..n).map(|i| json!({ "day": i })).collect())
    }

    #[test]
    fn trends_keep_one_point_a_week_ending_today() {
        let v = shape_dashboard(json!({ "apps": [{ "trend": trend(91) }] }));
        let t = v["apps"][0]["trend"].as_array().unwrap();
        assert_eq!(t.len(), 13);
        assert_eq!(t[0]["day"], 6);
        assert_eq!(t[12]["day"], 90);
        let i = shape_insights(json!({ "trend": trend(10) }));
        assert_eq!(i["trend"], json!([{ "day": 2 }, { "day": 9 }]));
        assert_eq!(shape_insights(Value::Null), Value::Null);
    }
}
