//! Trim engine view models into compact, agent-friendly JSON.
//! Nothing here reads secrets: inputs are view models that never carry them.

mod board;
mod optimizer;

pub use board::{board, term, BoardOpts};
pub use optimizer::optimizer;

use serde_json::{json, Map, Value};

/// Recursively drop `null` fields from objects.
pub fn prune(v: Value) -> Value {
    match v {
        Value::Object(m) => Value::Object(
            m.into_iter()
                .filter(|(_, v)| !v.is_null())
                .map(|(k, v)| (k, prune(v)))
                .collect(),
        ),
        Value::Array(a) => Value::Array(a.into_iter().map(prune).collect()),
        other => other,
    }
}

/// Truncate text to `max` chars (on a char boundary) with an ellipsis.
pub fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max).collect();
    format!("{cut}… [{} chars total]", s.chars().count())
}

pub fn round1(v: &Value) -> Value {
    v.as_f64()
        .map(|f| json!((f * 10.0).round() / 10.0))
        .unwrap_or(Value::Null)
}

/// Copy selected keys (renamed `from → to`) from an object.
pub fn pick(src: &Value, keys: &[(&str, &str)]) -> Value {
    let mut out = Map::new();
    for (from, to) in keys {
        if let Some(v) = src.get(*from).filter(|v| !v.is_null()) {
            out.insert((*to).to_string(), v.clone());
        }
    }
    Value::Object(out)
}

fn arr(v: &Value) -> &[Value] {
    v.as_array().map(Vec::as_slice).unwrap_or(&[])
}

// ------------------------------------------------------------ jobs

pub fn is_active(job: &Value) -> bool {
    matches!(
        job.get("status").and_then(Value::as_str),
        Some("queued" | "running")
    )
}

pub fn is_terminal(job: &Value) -> bool {
    matches!(
        job.get("status").and_then(Value::as_str),
        Some("done" | "failed" | "stopped")
    )
}

pub fn job(j: &Value) -> Value {
    let mut out = pick(
        j,
        &[
            ("id", "id"),
            ("kind", "kind"),
            ("projectId", "projectId"),
            ("label", "label"),
            ("status", "status"),
            ("etaSeconds", "etaSeconds"),
            ("message", "message"),
            ("pausedUntil", "pausedUntil"),
            ("error", "error"),
            ("finishedAt", "finishedAt"),
        ],
    );
    let done = j.get("done").and_then(Value::as_u64).unwrap_or(0);
    let total = j.get("total").and_then(Value::as_u64).unwrap_or(0);
    if total > 0 {
        out["progress"] = json!(format!("{done}/{total}"));
    }
    out
}

/// Queued / running jobs, optionally for one project.
pub fn active_jobs(jobs: &Value, project_id: Option<&str>) -> Vec<Value> {
    arr(jobs)
        .iter()
        .filter(|j| is_active(j))
        .filter(|j| {
            project_id.is_none_or(|p| j.get("projectId").and_then(Value::as_str) == Some(p))
        })
        .map(job)
        .collect()
}

/// The last `n` finished jobs (newest first by `finishedAt`).
pub fn recent_jobs(jobs: &Value, n: usize) -> Vec<Value> {
    let mut done: Vec<&Value> = arr(jobs).iter().filter(|j| is_terminal(j)).collect();
    done.sort_by(|a, b| {
        let k = |j: &Value| {
            j.get("finishedAt")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string()
        };
        k(b).cmp(&k(a))
    });
    done.into_iter().take(n).map(job).collect()
}

// ------------------------------------------------------------ overview

pub fn project(p: &Value) -> Value {
    pick(
        p,
        &[
            ("id", "id"),
            ("name", "name"),
            ("store", "store"),
            ("kind", "kind"),
            ("appId", "appId"),
            ("developer", "developer"),
            ("folderId", "folderId"),
            ("markets", "markets"),
            ("mainMarket", "mainMarket"),
            ("keywordCount", "keywords"),
            ("top10Count", "top10"),
            ("checkedAt", "checkedAt"),
            ("autoCheck", "autoCheck"),
            ("busy", "busy"),
            ("checkedPhrases", "checkedPhrases"),
            ("uncheckedPhrases", "neverChecked"),
            ("auditStep", "auditStep"),
        ],
    )
}

pub struct OverviewFilter<'a> {
    pub store: Option<&'a str>,
    pub folder_id: Option<&'a str>,
    pub kind: Option<&'a str>,
}

pub fn overview(
    snapshot: &Value,
    license: Option<&Value>,
    alerts: Option<&Value>,
    f: &OverviewFilter,
) -> Value {
    let projects: Vec<Value> = arr(&snapshot["projects"])
        .iter()
        .filter(|p| f.store.is_none_or(|s| p["store"].as_str() == Some(s)))
        .filter(|p| {
            f.folder_id
                .is_none_or(|id| p["folderId"].as_str() == Some(id))
        })
        .filter(|p| f.kind.is_none_or(|k| p["kind"].as_str() == Some(k)))
        .map(project)
        .collect();
    let folders: Vec<Value> = arr(&snapshot["folders"])
        .iter()
        .map(|x| pick(x, &[("id", "id"), ("name", "name")]))
        .collect();
    let settings = pick(
        &snapshot["settings"],
        &[
            ("metricDriver", "metricDriver"),
            ("cellDisplay", "cellDisplay"),
            ("aiProvider", "aiProvider"),
            ("aiModel", "aiModel"),
            ("checkSchedule", "checkSchedule"),
            ("checkSpeed", "checkSpeed"),
            ("defaultMarkets", "defaultMarkets"),
        ],
    );
    let mut out = json!({
        "projects": projects,
        "folders": folders,
        "activeJobs": active_jobs(&snapshot["jobs"], None),
        "recentJobs": recent_jobs(&snapshot["jobs"], 5),
        "settings": settings,
    });
    if let Some(l) = license {
        out["license"] = pick(
            l,
            &[
                ("state", "state"),
                ("plan", "plan"),
                ("features", "features"),
                ("expiresAt", "expiresAt"),
                ("message", "message"),
            ],
        );
    }
    if let Some(a) = alerts {
        let unread = arr(a).iter().filter(|x| x["read"] == json!(false)).count();
        out["unreadRankAlerts"] = json!(unread);
    }
    out
}

// ------------------------------------------------------------ lists

pub fn markets(list: &Value, featured_only: bool) -> Value {
    let rows: Vec<Value> = arr(list)
        .iter()
        .filter(|m| !featured_only || m["featured"] == json!(true))
        .map(|m| {
            let mut o = pick(
                m,
                &[
                    ("country", "country"),
                    ("name", "name"),
                    ("searchLanguage", "searchLanguage"),
                    ("listingLocales", "listingLocales"),
                    ("englishSearch", "englishSearch"),
                    ("featured", "featured"),
                    ("accessNote", "accessNote"),
                ],
            );
            o["potential"] = round1(&m["potential"]);
            if let Some(size) = market_size(m, featured_iphone_users(list)) {
                o["size"] = size;
            }
            o
        })
        .collect();
    json!({
        "count": rows.len(),
        "markets": rows,
        "note": "potential = 0..10 market size/value score (US = 10). size: rounded estimates in millions (population, smartphone users, iPhone/Android users for the store), platform share %, GDP per person at PPP in thousand $, accessPct (% of people who can buy; accessNote says why), sharePct = this market's users as % of all featured markets. Estimates, not measurements."
    })
}

fn platform_users(m: &Value) -> Option<f64> {
    Some(m["smartphoneUsersM"].as_f64()? * m["platformSharePct"].as_f64()? / 100.0)
}

fn featured_iphone_users(list: &Value) -> f64 {
    arr(list)
        .iter()
        .filter(|m| m["featured"] == json!(true))
        .filter_map(platform_users)
        .sum()
}

/// Estimated market size of one storefront (from `MarketInfo`).
fn market_size(m: &Value, featured_total: f64) -> Option<Value> {
    let users = platform_users(m)?;
    let share_pct = m["platformSharePct"].as_f64()?;
    let mut o = json!({
        "populationM": round1(&m["populationM"]),
        "smartphoneUsersM": round1(&m["smartphoneUsersM"]),
        "platformUsersM": round1(&json!(users)),
        "platformSharePct": round1(&json!(share_pct)),
        "gdpPppK": round1(&json!(m["gdpPppPerCapita"].as_f64().unwrap_or(0.0) / 1000.0)),
        "accessPct": round1(&m["accessPct"]),
    });
    if featured_total > 0.0 && m["featured"] == json!(true) {
        o["sharePct"] = round1(&json!(users / featured_total * 100.0));
    }
    Some(o)
}

pub fn app(a: &Value) -> Value {
    pick(
        a,
        &[
            ("id", "id"),
            ("name", "name"),
            ("subtitle", "subtitle"),
            ("developer", "developer"),
            ("genre", "genre"),
            ("rating", "rating"),
            ("ratingCount", "ratings"),
            ("installs", "installs"),
            ("storeUrl", "url"),
        ],
    )
}

pub fn ai_status(list: &Value) -> Value {
    let rows: Vec<Value> = arr(list)
        .iter()
        .map(|p| {
            let mut o = pick(
                p,
                &[
                    ("id", "id"),
                    ("name", "name"),
                    ("ready", "ready"),
                    ("installed", "installed"),
                    ("signedIn", "signedIn"),
                    ("version", "version"),
                    ("model", "model"),
                    ("latencyMs", "latencyMs"),
                    ("error", "error"),
                ],
            );
            let models: Vec<Value> = arr(&p["models"]).iter().take(12).cloned().collect();
            if !models.is_empty() {
                o["models"] = json!(models);
            }
            if let Some(step) = arr(&p["steps"]).iter().find(|s| s["done"] == json!(false)) {
                o["nextSetupStep"] = pick(step, &[("title", "title"), ("command", "command")]);
            }
            o
        })
        .collect();
    json!({ "providers": rows })
}

pub fn alerts(list: &Value, unread_only: bool, project_id: Option<&str>, limit: usize) -> Value {
    let all: Vec<&Value> = arr(list)
        .iter()
        .filter(|a| !unread_only || a["read"] == json!(false))
        .filter(|a| project_id.is_none_or(|p| a["projectId"].as_str() == Some(p)))
        .collect();
    let rows: Vec<Value> = all
        .iter()
        .take(limit)
        .map(|a| prune((*a).clone()))
        .collect();
    json!({ "total": all.len(), "alerts": rows, "note": "from/to are store ranks (null = not ranked)." })
}

pub fn assistant(thread: &Value, last: usize) -> Value {
    let msgs = arr(&thread["messages"]);
    let skip = msgs.len().saturating_sub(last);
    let rows: Vec<Value> = msgs
        .iter()
        .skip(skip)
        .map(|m| {
            let mut o = pick(
                m,
                &[
                    ("id", "id"),
                    ("role", "role"),
                    ("at", "at"),
                    ("keywords", "keywords"),
                    ("changes", "changes"),
                    ("pending", "pending"),
                ],
            );
            o["text"] = json!(truncate(m["text"].as_str().unwrap_or(""), 4000));
            o
        })
        .collect();
    json!({
        "projectId": thread["projectId"],
        "scope": thread["scope"],
        "totalMessages": msgs.len(),
        "messages": rows,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prune_drops_nulls_recursively() {
        let v = prune(json!({"a": null, "b": {"c": null, "d": 1}, "e": [{"f": null}]}));
        assert_eq!(v, json!({"b": {"d": 1}, "e": [{}]}));
    }

    #[test]
    fn truncate_respects_char_boundaries() {
        assert_eq!(truncate("héllo", 10), "héllo");
        assert!(truncate("héllo wörld", 3).starts_with("hél…"));
    }

    #[test]
    fn markets_carry_size_and_share_of_featured() {
        let list = json!([
            {"country":"us","featured":true,"potential":10.0,"populationM":340.0,"smartphoneUsersM":300.0,"platformSharePct":50.0,"gdpPppPerCapita":80000,"accessPct":100.0},
            {"country":"gb","featured":true,"potential":7.0,"populationM":68.0,"smartphoneUsersM":60.0,"platformSharePct":50.0,"gdpPppPerCapita":55000,"accessPct":100.0},
            {"country":"zz","featured":false,"potential":1.0}
        ]);
        let out = markets(&list, false);
        let us = &out["markets"][0]["size"];
        assert_eq!(us["platformUsersM"], 150.0);
        assert_eq!(us["platformSharePct"], 50.0);
        assert_eq!(us["gdpPppK"], 80.0);
        assert_eq!(us["sharePct"], 83.3);
        assert!(out["markets"][2].get("size").is_none(), "no data, no size");
    }

    #[test]
    fn overview_filters_by_kind_and_shows_progress() {
        let snap = json!({"projects":[
            {"id":"1","kind":"mine","store":"apple","checkedPhrases":10,"uncheckedPhrases":2,"auditStep":"options"},
            {"id":"2","kind":"competitor","store":"apple"}],"folders":[],"jobs":[],"settings":{}});
        let f = OverviewFilter { store: None, folder_id: None, kind: Some("mine") };
        let out = overview(&snap, None, None, &f);
        let ps = out["projects"].as_array().unwrap();
        assert_eq!(ps.len(), 1);
        assert_eq!(ps[0]["neverChecked"], 2);
        assert_eq!(ps[0]["auditStep"], "options");
    }

    #[test]
    fn job_progress_and_filters() {
        let jobs = json!([
            {"id":"1","kind":"check","projectId":"p","label":"Check","status":"running","done":2,"total":5},
            {"id":"2","kind":"check","projectId":"q","label":"Check","status":"done","done":5,"total":5,"finishedAt":"2026-01-01T00:00:00Z"}
        ]);
        let a = active_jobs(&jobs, Some("p"));
        assert_eq!(a.len(), 1);
        assert_eq!(a[0]["progress"], "2/5");
        assert!(active_jobs(&jobs, Some("q")).is_empty());
        assert_eq!(recent_jobs(&jobs, 5)[0]["id"], "2");
    }
}
