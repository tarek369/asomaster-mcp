//! Optimizer ("listing studio") state, trimmed: long descriptions are cut
//! unless `full`, phrase / candidate lists are capped.

use serde_json::{json, Value};

use super::{arr, pick, round1, truncate};

const DESCRIPTION_PREVIEW: usize = 300;
const RATIONALE_MAX: usize = 600;
const MAX_PHRASES: usize = 25;
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

fn option(o: &Value, full: bool) -> Value {
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
    out["listing"] = listing(&o["listing"], full);
    for k in ["reach", "difficulty", "reachDelta", "difficultyDelta"] {
        out[k] = round1(&o[k]);
    }
    if let Some(r) = o["rationale"].as_str() {
        out["rationale"] = json!(truncate(r, RATIONALE_MAX));
    }
    let phrases = arr(&o["phrases"]);
    out["phrases"] = json!(phrases
        .iter()
        .take(MAX_PHRASES)
        .map(|p| pick(
            p,
            &[
                ("phrase", "phrase"),
                ("volume", "estVol"),
                ("difficulty", "estDiff"),
                ("popularity", "pop"),
                ("rank", "rank"),
                ("coveredBy", "coveredBy"),
            ]
        ))
        .collect::<Vec<_>>());
    if phrases.len() > MAX_PHRASES {
        out["phrasesTotal"] = json!(phrases.len());
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
        ],
    );
    if !arr(&s["failedLocales"]).is_empty() {
        out["failedLocales"] = s["failedLocales"].clone();
        out["fix"] = json!("optimizer_localize with remaining:true writes only the missing / failed locales.");
    }
    if let Some(r) = s["rationale"].as_str() {
        out["rationale"] = json!(truncate(r, RATIONALE_MAX));
    }
    out["current"] = listing(&s["current"], full);
    out["options"] = json!(arr(&s["options"])
        .iter()
        .map(|o| option(o, full))
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
