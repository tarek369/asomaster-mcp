//! Apple Ads search terms as keyword ideas (read-only). Without `market` the
//! answer is a compact summary per market (agents ask for one market's rows
//! next); with `market`, that market's terms in full.

use serde_json::{json, Map, Value};

use super::schema as s;
use super::{Tool, READ};
use crate::error::BridgeError;
use crate::ipc::Bridge;

/// Terms returned for one market unless `limit` asks for another number.
pub const DEFAULT_LIMIT: usize = 50;
/// Most terms returned for one market.
pub const MAX_LIMIT: usize = 200;
/// Markets in the summary (most impressions first).
pub const SUMMARY_MARKETS: usize = 10;
/// Top terms per market in the summary.
pub const SUMMARY_TERMS: usize = 5;

pub fn tools() -> Vec<Tool> {
    vec![Tool::custom(
        "get_search_terms",
        "Apple Ads search terms",
        "REAL search terms people typed in the App Store that showed the owner's own app in its Apple Ads campaigns, per country, as keyword ideas. Without market: a summary of the markets with the most impressions (totals, lowVolume, termCount, avoidedCount, the top 5 terms); call again with market for that market's full rows: impressions, taps, installs, ttr (taps / impressions), the bid keyword and match type that triggered each term, tracked (a keyword already has this phrase in that market) and avoided (on the don't-target list). lowVolume totals the terms Apple hides for privacy (too few searches to name). Cached by the app: a period the owner loaded once is refreshed at most once a day while Apple Ads is connected (weekly while no campaign promotes the app); fetchedAt / stale / nextRefreshAt say how old it is, warning names campaigns that failed. state: ready, no_campaigns (no Apple Ads campaign promotes the app), not_loaded (the owner has not loaded this period in the app), not_connected (connect Apple Ads first) or unsupported (Google Play, rivals, app ideas). To track a term, call add_keywords with market set to its country (localize true when the market's search language isn't English).",
        s::object(
            vec![
                ("projectId", s::project_id()),
                (
                    "days",
                    json!({"type": "integer", "enum": [7, 30, 90], "description": "Period in days (default 30)."}),
                ),
                ("market", s::country()),
                ("limit", s::integer("With market: most terms, most impressions first (default 50).", 1, MAX_LIMIT as i64)),
                ("includeAvoided", s::boolean("With market: include terms on the don't-target list (default false).")),
            ],
            &["projectId"],
        ),
        READ,
    )]
}

pub async fn run(bridge: &Bridge, name: &str, args: &Value) -> Option<Result<Value, BridgeError>> {
    if name != "get_search_terms" {
        return None;
    }
    let call = json!({ "projectId": args["projectId"], "days": args.get("days").cloned().unwrap_or(json!(30)) });
    Some(
        bridge
            .call("get_search_terms", call)
            .await
            .map(|v| shape(v, args)),
    )
}

/// The view shaped for an agent: one market in full, or a summary.
pub fn shape(mut view: Value, args: &Value) -> Value {
    let market = args
        .get("market")
        .and_then(Value::as_str)
        .map(str::to_lowercase);
    let Some(list) = view.get_mut("markets").and_then(Value::as_array_mut) else {
        return view;
    };
    let count = list.len();
    match market {
        Some(cc) => {
            let limit = args
                .get("limit")
                .and_then(Value::as_u64)
                .map_or(DEFAULT_LIMIT, |n| (n as usize).min(MAX_LIMIT));
            let include_avoided = args
                .get("includeAvoided")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            list.retain(|m| m["country"].as_str() == Some(cc.as_str()));
            list.iter_mut()
                .for_each(|m| trim_terms(m, limit, include_avoided));
        }
        None => {
            let summary: Vec<Value> = list.iter().take(SUMMARY_MARKETS).map(summarize).collect();
            *list = summary;
        }
    }
    if let Some(obj) = view.as_object_mut() {
        obj.insert("marketCount".into(), json!(count));
    }
    view
}

fn avoided(t: &Value) -> bool {
    t["avoided"] == json!(true)
}

/// One market as totals, counts and its top terms (don't-target terms left out).
fn summarize(m: &Value) -> Value {
    let terms = m["terms"].as_array().map(Vec::as_slice).unwrap_or_default();
    let top: Vec<Value> = terms
        .iter()
        .filter(|t| !avoided(t))
        .take(SUMMARY_TERMS)
        .map(|t| json!({"term": t["term"], "impressions": t["impressions"], "taps": t["taps"], "installs": t["installs"], "tracked": t["tracked"]}))
        .collect();
    let mut out = Map::new();
    for k in [
        "country",
        "name",
        "trackedMarket",
        "englishSearch",
        "totals",
        "lowVolume",
    ] {
        out.insert(k.into(), m[k].clone());
    }
    out.insert("termCount".into(), json!(terms.len()));
    out.insert(
        "avoidedCount".into(),
        json!(terms.iter().filter(|t| avoided(t)).count()),
    );
    out.insert("topTerms".into(), json!(top));
    Value::Object(out)
}

fn trim_terms(market: &mut Value, limit: usize, include_avoided: bool) {
    let Some(terms) = market.get_mut("terms").and_then(Value::as_array_mut) else {
        return;
    };
    let total = terms.len();
    let avoided_count = terms.iter().filter(|t| avoided(t)).count();
    if !include_avoided {
        terms.retain(|t| !avoided(t));
    }
    terms.truncate(limit);
    let shown = terms.len();
    if let Some(obj) = market.as_object_mut() {
        obj.insert("termCount".into(), json!(total));
        obj.insert("avoidedCount".into(), json!(avoided_count));
        obj.insert("shownTerms".into(), json!(shown));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(term: &str, avoided: bool) -> Value {
        json!({"term": term, "impressions": 10, "taps": 1, "installs": 0, "tracked": false, "avoided": avoided, "keyword": "k"})
    }

    fn view() -> Value {
        json!({
            "state": "ready",
            "markets": [
                {"country": "us", "name": "United States", "totals": {"impressions": 30}, "lowVolume": {"impressions": 3},
                 "terms": [t("a", false), t("b", true), t("c", false)]},
                {"country": "gb", "name": "United Kingdom", "terms": [t("d", false)]}
            ]
        })
    }

    #[test]
    fn one_market_in_full_with_a_capped_limit() {
        let out = shape(view(), &json!({"market": "US", "limit": 1}));
        let markets = out["markets"].as_array().unwrap();
        assert_eq!(markets.len(), 1);
        assert_eq!(markets[0]["terms"], json!([t("a", false)]));
        assert_eq!(
            (
                markets[0]["termCount"].clone(),
                markets[0]["avoidedCount"].clone()
            ),
            (json!(3), json!(1))
        );
        let many: Vec<Value> = (0..300).map(|i| t(&format!("t{i}"), false)).collect();
        let big = json!({"markets": [{"country": "us", "terms": many}]});
        let out = shape(big.clone(), &json!({"market": "us", "limit": 999}));
        assert_eq!(
            out["markets"][0]["terms"].as_array().unwrap().len(),
            MAX_LIMIT
        );
        let out = shape(big, &json!({"market": "us"}));
        assert_eq!(
            out["markets"][0]["terms"].as_array().unwrap().len(),
            DEFAULT_LIMIT
        );
    }

    #[test]
    fn avoided_terms_only_on_request() {
        let out = shape(view(), &json!({"market": "us"}));
        assert_eq!(out["markets"][0]["terms"].as_array().unwrap().len(), 2);
        let out = shape(view(), &json!({"market": "us", "includeAvoided": true}));
        assert_eq!(out["markets"][0]["terms"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn without_market_only_a_summary_of_the_top_markets() {
        let out = shape(view(), &json!({}));
        let us = &out["markets"][0];
        assert!(us.get("terms").is_none(), "no full rows without market");
        assert_eq!(
            (us["termCount"].clone(), us["avoidedCount"].clone()),
            (json!(3), json!(1))
        );
        assert_eq!(
            us["topTerms"],
            json!([
                {"term": "a", "impressions": 10, "taps": 1, "installs": 0, "tracked": false},
                {"term": "c", "impressions": 10, "taps": 1, "installs": 0, "tracked": false}
            ])
        );
        assert_eq!(us["lowVolume"]["impressions"], 3);
        assert_eq!(out["marketCount"], 2);
        let many: Vec<Value> = (0..15)
            .map(|i| json!({"country": format!("c{i}"), "terms": []}))
            .collect();
        let out = shape(json!({"markets": many}), &json!({}));
        assert_eq!(out["markets"].as_array().unwrap().len(), SUMMARY_MARKETS);
        assert_eq!(out["marketCount"], 15);
    }
}
