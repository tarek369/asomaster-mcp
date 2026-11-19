//! Optimizer ("listing studio") state, trimmed: long descriptions are cut
//! unless `full`, phrase / candidate lists are capped. The measured phrases
//! (estimates, rank, chance) are listed once at the top (`phrases`, best worth
//! to the app first); each option lists only the phrases it covers, with its
//! raw and winnable (`expectedReach`) reach.

use serde_json::{json, Value};

use super::{arr, pick, round1, truncate};

const DESCRIPTION_PREVIEW: usize = 300;
const RATIONALE_MAX: usize = 600;
/// Rows of the shared phrase table (by worth to the app).
const MAX_PHRASES: usize = 40;
/// Covered phrases listed per option (by worth to the app).
const MAX_COVERED: usize = 20;
const MAX_CANDIDATES: usize = 5;

fn listing(l: &Value, full: bool) -> Value {
    let mut o = pick(
        l,
        &[
            ("locale", "locale"),
            ("title", "title"),
            ("subtitle", "subtitle"),
            ("keywords", "keywords"),
            ("shortDescription", "shortDescription"),
            ("promotionalText", "promotionalText"),
        ],
    );
    if let Some(d) = l["description"].as_str() {
        o["description"] = json!(if full {
            d.to_string()
        } else {
            truncate(d, DESCRIPTION_PREVIEW)
        });
    }
    o
}

/// `number`: the option's 1-based position, as the app and the verdict number it.
fn option(o: &Value, number: usize, full: bool) -> Value {
    let mut out = pick(
        o,
        &[
            ("id", "id"),
            ("label", "label"),
            ("recommended", "recommended"),
            ("custom", "custom"),
            ("problems", "problems"),
        ],
    );
    out["number"] = json!(number);
    out["listing"] = listing(&o["listing"], full);
    for k in ["reach", "difficulty", "reachDelta", "difficultyDelta"] {
        out[k] = round1(&o[k]);
    }
    for k in ["expectedReach", "expectedReachDelta"] {
        if !o[k].is_null() {
            out[k] = round1(&o[k]);
        }
    }
    if let Some(r) = o["rationale"].as_str() {
        out["rationale"] = json!(truncate(r, RATIONALE_MAX));
    }
    // Metrics are the same for every option (top-level `phrases`); here only what it covers.
    let covered: Vec<&Value> = by_value(arr(&o["phrases"]))
        .into_iter()
        .filter(|p| p["coveredBy"].as_str().is_some_and(|f| f != "none"))
        .collect();
    out["covered"] = json!(covered
        .iter()
        .take(MAX_COVERED)
        .map(|p| pick(p, &[("phrase", "phrase"), ("coveredBy", "coveredBy")]))
        .collect::<Vec<_>>());
    if covered.len() > MAX_COVERED {
        out["coveredTotal"] = json!(covered.len());
    }
    out
}

/// Phrases by worth to this app: estVol × chance when the chance is known, else estVol.
fn by_value(phrases: &[Value]) -> Vec<&Value> {
    let worth = |p: &Value| p["volume"].as_f64().unwrap_or(0.0) * p["chance"].as_f64().map_or(1.0, |c| c / 100.0);
    let mut out: Vec<&Value> = phrases.iter().collect();
    out.sort_by(|a, b| worth(b).total_cmp(&worth(a)).then_with(|| b["volume"].as_f64().unwrap_or(0.0).total_cmp(&a["volume"].as_f64().unwrap_or(0.0))));
    out
}

/// The measured phrases the options are scored on, once: estimates, rank and chance.
fn phrase_table(options: &[Value]) -> Option<Value> {
    let first = options.first()?;
    let all = by_value(arr(&first["phrases"]));
    let rows: Vec<Value> = all
        .iter()
        .take(MAX_PHRASES)
        .map(|p| pick(p, &[("phrase", "phrase"), ("volume", "estVol"), ("difficulty", "estDiff"), ("popularity", "pop"), ("rank", "rank"), ("chance", "chance")]))
        .collect();
    Some(json!({ "rows": rows, "total": all.len() }))
}

/// Raw and winnable reach of the current listing, read back from an option and its deltas.
fn current_reach(options: &[Value]) -> Value {
    let Some(o) = options.first() else { return Value::Null };
    let back = |total: &str, delta: &str| match (o[total].as_f64(), o[delta].as_f64()) {
        (Some(t), Some(d)) => json!((t.round() - d).round()),
        _ => Value::Null,
    };
    let mut out = json!({ "reach": back("reach", "reachDelta") });
    let expected = back("expectedReach", "expectedReachDelta");
    if !expected.is_null() {
        out["expectedReach"] = expected;
    }
    out
}

fn locale_draft(d: &Value, full: bool) -> Value {
    let mut out = pick(
        d,
        &[
            ("locale", "locale"),
            ("market", "market"),
            ("status", "status"),
            ("stale", "stale"),
            ("notes", "notes"),
        ],
    );
    out["listing"] = listing(&d["listing"], full);
    out["candidates"] = json!(arr(&d["candidates"])
        .iter()
        .map(|c| json!({
            "keyword": c["keyword"],
            "options": arr(&c["options"]).iter().take(MAX_CANDIDATES).map(|o| {
                let mut x = pick(o, &[("phrase", "phrase"), ("volume", "estVol"), ("difficulty", "estDiff"), ("rank", "rank"), ("picked", "picked")]);
                x["score"] = round1(&o["score"]);
                x
            }).collect::<Vec<_>>(),
        }))
        .collect::<Vec<_>>());
    out
}

/// `locale` = only that locale's draft in full (others listed by status).
pub fn optimizer(s: &Value, full: bool, locale: Option<&str>) -> Value {
    let mut out = pick(
        s,
        &[
            ("projectId", "projectId"),
            ("store", "store"),
            ("step", "step"),
            ("mainLocale", "mainLocale"),
            ("mainMarket", "mainMarket"),
            ("stale", "stale"),
            ("chosenOptionId", "chosenOptionId"),
            ("corePhrases", "corePhrases"),
            ("lastPublishedAt", "lastPublishedAt"),
            ("lastPublishedVersion", "lastPublishedVersion"),
            ("lastError", "lastError"),
            ("verdict", "verdict"),
            ("consoleLinked", "consoleLinked"),
            ("keywordsUnknown", "keywordsUnknown"),
        ],
    );
    if !arr(&s["failedLocales"]).is_empty() {
        out["failedLocales"] = s["failedLocales"].clone();
        out["fix"] = json!("optimizer_localize with remaining:true writes only the missing / failed locales.");
    }
    if let Some(r) = s["rationale"].as_str() {
        out["rationale"] = json!(truncate(r, RATIONALE_MAX));
    }
    let review = &s["review"];
    if !arr(&review["changes"]).is_empty() || !arr(&review["looks"]).is_empty() {
        out["review"] = pick(review, &[("changes", "changes"), ("looks", "looks"), ("runs", "runs"), ("stale", "stale")]);
    }
    if !arr(&s["renames"]).is_empty() {
        out["renames"] = s["renames"].clone();
    }
    out["current"] = listing(&s["current"], full);
    let options = arr(&s["options"]);
    if !options.is_empty() {
        out["currentReach"] = current_reach(options);
    }
    if let Some(t) = phrase_table(options) {
        out["phrases"] = t["rows"].clone();
        if t["total"].as_u64().unwrap_or(0) > MAX_PHRASES as u64 {
            out["phrasesTotal"] = t["total"].clone();
        }
    }
    out["options"] = json!(options
        .iter()
        .enumerate()
        .map(|(i, o)| option(o, i + 1, full))
        .collect::<Vec<_>>());
    out["locales"] = json!(arr(&s["locales"])
        .iter()
        .map(|d| match locale {
            Some(l) if d["locale"].as_str() != Some(l) => pick(
                d,
                &[
                    ("locale", "locale"),
                    ("status", "status"),
                    ("stale", "stale")
                ]
            ),
            _ => locale_draft(d, full),
        })
        .collect::<Vec<_>>());
    if !full {
        out["note"] = json!("Descriptions are cut to 300 chars; pass full:true for complete text.");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trims_descriptions_unless_full() {
        let long = "x".repeat(1000);
        let s = json!({"projectId":"1","store":"apple","step":"options",
            "current":{"locale":"en-US","description": long},
            "options":[], "locales":[{"locale":"de-DE","market":"de","status":"ready","stale":false,
                "listing":{"locale":"de-DE"},"candidates":[]},{"locale":"fr-FR","market":"fr","status":"pending","stale":false,"listing":{"locale":"fr-FR"},"candidates":[]}]});
        let short = optimizer(&s, false, Some("de-DE"));
        assert!(short["current"]["description"].as_str().unwrap().len() < 400);
        assert!(short["locales"][0].get("listing").is_some());
        assert!(short["locales"][1].get("listing").is_none());
        let full = optimizer(&s, true, None);
        assert_eq!(full["current"]["description"].as_str().unwrap().len(), 1000);
    }

    fn phrase(name: &str, volume: u8, chance: u8, covered: &str) -> Value {
        json!({"phrase": name, "volume": volume, "difficulty": 90, "popularity": null, "rank": null,
            "coveredBy": covered, "fields": [], "chance": chance})
    }

    #[test]
    fn shows_winnable_reach_and_one_phrase_table() {
        let phrases = |fiber: &str| {
            let mut p: Vec<Value> = (0..50).map(|i| phrase(&format!("filler {i}"), 10, 1, "none")).collect();
            p.push(phrase("calorie counter", 89, 1, "title"));
            p.push(phrase("sugar tracker", 38, 23, "shortDescription"));
            p.push(phrase("fiber tracker", 6, 43, fiber));
            p
        };
        let option = |title: &str, fiber: &str, exp: f64, delta: f64| json!({"id":"o","label":"o","recommended":false,"custom":false,
            "listing":{"locale":"en-US","title":title},"reach":404.4,"difficulty":91.0,"reachDelta":19.0,"difficultyDelta":0.0,
            "expectedReach":exp,"expectedReachDelta":delta,"rationale":"","phrases":phrases(fiber),"problems":[]});
        let s = json!({"projectId":"13","store":"google","step":"options","current":{"locale":"en-US","title":"FitFoodie: AI Calorie Counter"},
            "options":[option("Fiber Tracker - FitFoodie AI","title",18.0,8.0), option("FitFoodie: AI Calorie Counter","none",18.0,8.0)],"locales":[]});
        let out = optimizer(&s, false, None);
        assert_eq!(out["currentReach"], json!({"reach": 385.0, "expectedReach": 10.0}));
        let o = &out["options"][0];
        assert_eq!((o["expectedReach"].as_f64(), o["expectedReachDelta"].as_f64()), (Some(18.0), Some(8.0)));
        assert_eq!(o["covered"].as_array().unwrap().len(), 3, "only covered phrases: {o}");
        assert_eq!(o["covered"][0], json!({"phrase":"sugar tracker","coveredBy":"shortDescription"}), "by worth (volume × chance)");
        assert!(o.get("phrases").is_none());
        let table = out["phrases"].as_array().unwrap();
        assert_eq!(table.len(), MAX_PHRASES);
        assert_eq!(out["phrasesTotal"], 53);
        assert_eq!(table[0], json!({"phrase":"sugar tracker","estVol":38,"estDiff":90,"chance":23}));
        assert!(table.iter().any(|p| p["phrase"] == "calorie counter" && p["estVol"] == 89));
        assert_eq!(out["options"][1]["covered"].as_array().unwrap().len(), 2);
        // Without the winnability curve the fields are left out, not null.
        let mut plain = s.clone();
        plain["options"][0]["expectedReach"] = Value::Null;
        plain["options"][0]["expectedReachDelta"] = Value::Null;
        let out = optimizer(&plain, false, None);
        assert!(out["options"][0].get("expectedReach").is_none());
        assert!(out["currentReach"].get("expectedReach").is_none());
    }

    #[test]
    fn surfaces_why_a_run_failed() {
        let s = json!({"projectId":"1","step":"ready","current":{},"options":[],"locales":[],
            "lastError":{"kind":"localize","message":"1 of 2 languages could not be written","at":"x"},
            "failedLocales":[{"locale":"fr-FR","reason":"bad answer"}]});
        let out = optimizer(&s, false, None);
        assert_eq!(out["lastError"]["kind"], "localize");
        assert_eq!(out["failedLocales"][0]["locale"], "fr-FR");
        assert!(out["fix"].as_str().unwrap().contains("remaining"));
        let ok = optimizer(&json!({"step":"idle","failedLocales":[]}), false, None);
        assert!(ok.get("failedLocales").is_none() && ok.get("lastError").is_none());
    }
}
