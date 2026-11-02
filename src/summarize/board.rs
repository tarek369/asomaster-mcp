//! Board (keyword × market grid) and term detail, summarised per market.

use serde_json::{json, Map, Value};

use super::{app, arr, pick, project, round1, truncate};

/// Markets shown in full when the caller does not choose.
const DEFAULT_MAX_MARKETS: usize = 4;
const MAX_COMPETITORS: usize = 10;

pub const BOARD_LEGEND: &str = "rank/prev = App Store / Play search position now / at the previous check (absent = not ranked in the checked results). \
estVol/estDiff = ASOMaster ESTIMATES 0-100 modelled from search results (always present once checked). \
pop = REAL popularity 0-100 from a connected provider or import (absent when none) - never mix it up with estVol. \
opp = 0-100 opportunity (demand x ease x rank headroom). verdict = ASOMaster's call for the cell/row.";

#[derive(Debug, Clone)]
pub struct BoardOpts {
    /// Countries to include (lower case); `None` = automatic.
    pub markets: Option<Vec<String>>,
    /// `active` (listing + idea), `listing`, `idea`, `hidden` or `all`.
    pub section: String,
    pub limit: usize,
    /// `board`, `opportunity`, `rank`, `estVolume`, `popularity`.
    pub sort: String,
}

impl Default for BoardOpts {
    fn default() -> Self {
        Self {
            markets: None,
            section: "active".into(),
            limit: 150,
            sort: "board".into(),
        }
    }
}

fn chosen_markets(board: &Value, opts: &BoardOpts) -> (Vec<String>, Vec<String>) {
    let tracked: Vec<String> = arr(&board["project"]["markets"])
        .iter()
        .filter_map(|m| m.as_str().map(str::to_string))
        .collect();
    let shown = match &opts.markets {
        Some(m) => m.clone(),
        None if tracked.len() <= DEFAULT_MAX_MARKETS => tracked.clone(),
        None => board["project"]["mainMarket"]
            .as_str()
            .map(|m| vec![m.to_string()])
            .unwrap_or_else(|| tracked.iter().take(1).cloned().collect()),
    };
    let omitted = tracked.into_iter().filter(|m| !shown.contains(m)).collect();
    (shown, omitted)
}

fn cell(c: &Value, label: &str) -> Value {
    let mut o = pick(
        c,
        &[
            ("rank", "rank"),
            ("prevRank", "prev"),
            ("verdict", "verdict"),
            ("opportunity", "opp"),
        ],
    );
    if let Some(p) = c["phrase"]
        .as_str()
        .filter(|p| !p.eq_ignore_ascii_case(label))
    {
        o["phrase"] = json!(p);
    }
    if !c["estimate"]["volume"].is_null() {
        o["estVol"] = c["estimate"]["volume"].clone();
    }
    if !c["estimate"]["difficulty"].is_null() {
        o["estDiff"] = c["estimate"]["difficulty"].clone();
    }
    if let Some(score) = c["popularity"].get("score").filter(|s| !s.is_null()) {
        o["pop"] = score.clone();
        o["popSource"] = c["popularity"]["source"].clone();
    }
    if c["checked"] == json!(false) {
        o["checked"] = json!(false);
    }
    if c["overridden"] == json!(true) {
        o["overridden"] = json!(true);
    }
    o
}

fn row(r: &Value, markets: &[String]) -> Value {
    let label = r["label"].as_str().unwrap_or_default();
    let s = &r["summary"];
    let mut summary = pick(
        s,
        &[
            ("top10", "top10"),
            ("ranked", "ranked"),
            ("bestRank", "best"),
            ("reviewsMedian", "reviewsMedian"),
            ("verdict", "verdict"),
        ],
    );
    for (from, to) in [
        ("avgVolume", "estVol"),
        ("avgDifficulty", "estDiff"),
        ("avgPopularity", "pop"),
    ] {
        if !s[from].is_null() {
            summary[to] = round1(&s[from]);
        }
    }
    let mut cells = Map::new();
    for m in markets {
        if let Some(c) = r["cells"].get(m) {
            cells.insert(m.clone(), cell(c, label));
        }
    }
    json!({
        "keywordId": r["keywordId"],
        "label": label,
        "source": r["source"],
        "summary": summary,
        "cells": cells,
    })
}

fn sort_key(r: &Value, sort: &str) -> f64 {
    let cells = r["cells"].as_object();
    let max_of = |k: &str| {
        cells
            .into_iter()
            .flat_map(|m| m.values())
            .filter_map(|c| c[k].as_f64())
            .fold(f64::NEG_INFINITY, f64::max)
    };
    match sort {
        "opportunity" => -max_of("opp"),
        "estVolume" => -max_of("estVol"),
        "popularity" => -max_of("pop"),
        "rank" => r["summary"]["best"].as_f64().unwrap_or(f64::INFINITY),
        _ => 0.0,
    }
}

fn section_included(id: &str, wanted: &str) -> bool {
    match wanted {
        "all" => true,
        "active" => id != "hidden",
        w => id == w,
    }
}

pub fn board(b: &Value, opts: &BoardOpts) -> Value {
    let (markets, omitted) = chosen_markets(b, opts);
    let mut sections = Map::new();
    let mut truncated = Map::new();
    let mut counts = Map::new();
    for s in arr(&b["sections"]) {
        let id = s["id"].as_str().unwrap_or("listing");
        let rows = arr(&s["rows"]);
        counts.insert(id.to_string(), json!(rows.len()));
        if !section_included(id, &opts.section) {
            continue;
        }
        let mut out: Vec<Value> = rows.iter().map(|r| row(r, &markets)).collect();
        if opts.sort != "board" {
            out.sort_by(|a, b| sort_key(a, &opts.sort).total_cmp(&sort_key(b, &opts.sort)));
        }
        if out.len() > opts.limit {
            truncated.insert(id.to_string(), json!(out.len()));
            out.truncate(opts.limit);
        }
        sections.insert(id.to_string(), Value::Array(out));
    }
    // One market asked for: that market's competitors; else the main one.
    let source = match opts.markets.as_deref() {
        Some([one]) if b["competitorsByMarket"][one].is_array() => &b["competitorsByMarket"][one],
        _ => &b["competitors"],
    };
    // Rows come per relation (losing_to, ahead_of, top): list each app once,
    // under its first (most telling) relation.
    let mut seen = std::collections::HashSet::new();
    let competitors: Vec<Value> = arr(source)
        .iter()
        .filter(|c| seen.insert(c["app"]["id"].as_str().unwrap_or_default().to_string()))
        .take(MAX_COMPETITORS)
        .map(|c| {
            let mut o = pick(
                c,
                &[
                    ("relation", "relation"),
                    ("aheadCount", "aheadOfUs"),
                    ("behindCount", "behindUs"),
                    ("sharedPhrases", "sharedPhrases"),
                ],
            );
            o["app"] = pick(
                &c["app"],
                &[("id", "id"), ("name", "name"), ("developer", "developer")],
            );
            o["avgRank"] = round1(&c["avgRank"]);
            o
        })
        .collect();
    let market_names: Vec<Value> = arr(&b["markets"])
        .iter()
        .filter(|m| markets.iter().any(|c| m["country"].as_str() == Some(c)))
        .map(|m| {
            pick(
                m,
                &[
                    ("country", "country"),
                    ("name", "name"),
                    ("searchLanguage", "language"),
                ],
            )
        })
        .collect();
    let mut out = json!({
        "project": project(&b["project"]),
        "checkedAt": b["checkedAt"],
        "shownMarkets": market_names,
        "rowCounts": counts,
        "sections": sections,
        "competitors": competitors,
        "legend": BOARD_LEGEND,
    });
    if !omitted.is_empty() {
        out["otherMarkets"] = json!(omitted);
        out["otherMarketsNote"] = json!("Pass markets:[...] to see these.");
    }
    if !truncated.is_empty() {
        out["truncatedFrom"] = Value::Object(truncated);
    }
    out
}

/// Term detail: current cell, rank / estimate history and the top results.
pub fn term(t: &Value, history_points: usize, top_n: usize) -> Value {
    let label = t["label"].as_str().unwrap_or_default();
    let history = arr(&t["cell"]["history"]);
    let skip = history.len().saturating_sub(history_points);
    let hist: Vec<Value> = history
        .iter()
        .skip(skip)
        .map(|p| {
            pick(
                p,
                &[
                    ("day", "day"),
                    ("rank", "rank"),
                    ("volume", "estVol"),
                    ("difficulty", "estDiff"),
                ],
            )
        })
        .collect();
    let top: Vec<Value> = arr(&t["top"])
        .iter()
        .take(top_n)
        .map(|r| {
            let mut o = app(&r["app"]);
            if let Some(obj) = o.as_object_mut() {
                obj.remove("url");
                if let Some(Value::String(s)) = obj.get("subtitle").cloned() {
                    obj.insert("subtitle".into(), json!(truncate(&s, 120)));
                }
            }
            o["position"] = r["position"].clone();
            o["relevance"] = r["relevance"].clone();
            if r["isMine"] == json!(true) {
                o["isMine"] = json!(true);
            }
            let fields: Vec<Value> = arr(&r["matches"])
                .iter()
                .filter_map(|m| m.get("field").cloned())
                .collect();
            if !fields.is_empty() {
                o["matchesIn"] = json!(fields);
            }
            o
        })
        .collect();
    json!({
        "projectId": t["projectId"],
        "keywordId": t["keywordId"],
        "label": label,
        "market": pick(&t["market"], &[("country", "country"), ("name", "name"), ("searchLanguage", "language")]),
        "checkedAt": t["checkedAt"],
        "now": cell(&t["cell"], label),
        "historyTotal": history.len(),
        "history": hist,
        "topApps": top,
        "legend": BOARD_LEGEND,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Value {
        json!({
            "project": {"id":"1","name":"App","store":"apple","kind":"mine","markets":["us","de","fr","gb","jp"],"mainMarket":"us","keywordCount":2,"top10Count":1},
            "markets": [{"country":"us","name":"United States","searchLanguage":"en"}],
            "sections": [
                {"id":"listing","rows":[
                    {"keywordId":"k1","label":"habit","source":"manual",
                     "cells":{"us":{"phrase":"habit","rank":4,"prevRank":7,"history":[],"estimate":{"volume":60,"difficulty":40},
                                    "popularity":{"score":55,"raw":1.0,"source":"apple_ads"},"verdict":"winning","checked":true,"overridden":false,"opportunity":70},
                              "de":{"phrase":"gewohnheit","rank":null,"history":[],"estimate":{},"verdict":"unchecked","checked":false,"overridden":true}},
                     "summary":{"top10":1,"ranked":1,"markets":2,"bestRank":4,"avgVolume":60.04,"verdict":"winning","addedAt":"x"}}]},
                {"id":"hidden","rows":[{"keywordId":"k2","label":"old","source":"ai","cells":{},"summary":{"top10":0,"ranked":0,"markets":0,"verdict":"drop","addedAt":"x"}}]}
            ],
            "competitors": [], "checkedAt": "2026-01-01T00:00:00Z"
        })
    }

    #[test]
    fn defaults_to_main_market_when_many_and_hides_hidden() {
        let out = board(&sample(), &BoardOpts::default());
        let rows = out["sections"]["listing"].as_array().unwrap();
        let cells = rows[0]["cells"].as_object().unwrap();
        assert_eq!(cells.keys().collect::<Vec<_>>(), vec!["us"]);
        assert_eq!(cells["us"]["pop"], 55);
        assert_eq!(cells["us"]["estVol"], 60);
        assert!(cells["us"].get("phrase").is_none());
        assert_eq!(rows[0]["summary"]["estVol"], 60.0);
        assert!(out["sections"].get("hidden").is_none());
        assert_eq!(out["rowCounts"]["hidden"], 1);
        assert_eq!(out["otherMarkets"].as_array().unwrap().len(), 4);
    }

    #[test]
    fn competitors_list_each_app_once_and_follow_a_single_market() {
        let mut b = sample();
        let row = |id: &str, rel: &str| json!({"app":{"id":id,"name":id},"relation":rel,"aheadCount":1,"behindCount":0,"avgRank":2.0,"sharedPhrases":2});
        b["competitors"] = json!([row("a", "losing_to"), row("b", "losing_to"), row("a", "top"), row("b", "top")]);
        b["competitorsByMarket"] = json!({"de": [row("z", "losing_to")]});
        let out = board(&b, &BoardOpts::default());
        let ids: Vec<&str> = out["competitors"].as_array().unwrap().iter().map(|c| c["app"]["id"].as_str().unwrap()).collect();
        assert_eq!(ids, vec!["a", "b"]);
        assert_eq!(out["competitors"][0]["relation"], "losing_to");
        let de = board(&b, &BoardOpts { markets: Some(vec!["de".into()]), ..BoardOpts::default() });
        assert_eq!(de["competitors"][0]["app"]["id"], "z");
    }

    #[test]
    fn explicit_markets_and_limit() {
        let opts = BoardOpts {
            markets: Some(vec!["de".into()]),
            section: "all".into(),
            limit: 1,
            sort: "opportunity".into(),
        };
        let out = board(&sample(), &opts);
        let de = &out["sections"]["listing"][0]["cells"]["de"];
        assert_eq!(de["phrase"], "gewohnheit");
        assert_eq!(de["checked"], false);
        assert_eq!(de["overridden"], true);
        assert!(out["sections"]["hidden"].is_array());
    }
}
