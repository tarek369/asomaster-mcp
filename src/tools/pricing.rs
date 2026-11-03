//! Local pricing: read the plan, edit it, publish it (with the owner's
//! approval and the app's own confirmation).
//!
//! The app speaks whole cents; agents read and write ordinary decimal prices
//! ("9.99", "980") and this module converts.

use std::time::Duration;

use serde_json::{json, Map, Value};

use super::schema as s;
use super::{queued, Annotations, Tool, READ_NET, SET};
use crate::error::BridgeError;
use crate::ipc::Bridge;

/// Longer than the engine's in-app confirmation window (~120 s).
const PUBLISH_CALL_TIMEOUT: Duration = Duration::from_secs(150);
/// Rows returned per product unless `country` narrows them.
const MAX_ROWS: usize = 60;

const PUBLISH: Annotations = Annotations {
    read_only: false,
    destructive: true,
    idempotent: false,
    open_world: true,
};

const METHODS: &[&str] = &["tiers", "purchasing_power", "income", "blended", "equal"];

fn price(desc: &str) -> Value {
    json!({ "type": ["number", "null"], "description": desc, "exclusiveMinimum": 0 })
}

fn price_patch(desc: &str, with_country: bool) -> Value {
    let mut props = vec![
        ("productId", s::id("Product id from get_pricing (e.g. sub:123, otp:pro/buy).")),
        ("price", price("Local price as a decimal (9.99, 980); null clears it.")),
    ];
    let mut required = vec!["productId", "price"];
    if with_country {
        props.push(("country", s::country()));
        required.push("country");
    }
    let mut o = s::object(props, &required);
    o["description"] = json!(desc);
    o
}

pub fn tools() -> Vec<Tool> {
    vec![
        Tool::custom(
            "get_pricing",
            "Local pricing plan",
            "The app's subscriptions / in-app purchases (App Store) or base plans / one-time purchase options (Google Play) with today's price in the base country, which are selected, and for selected ones the plan per country: today, scheduled, proposed (decimal local prices), share of the base price, changed, overridden, skipped reasons. Also the method spread previews (min/median/max % of base), data sources and the last publish report. Reads App Store Connect / Play Console (cached 10 min; refresh re-reads). Never writes.",
            s::object(
                vec![
                    ("projectId", s::project_id()),
                    ("refresh", s::boolean("Re-read the console now (default false).")),
                    ("country", s::country()),
                ],
                &["projectId"],
            ),
            READ_NET,
        ),
        Tool::custom(
            "plan_pricing",
            "Edit the pricing plan",
            "Change the saved pricing plan (nothing is written to the store): select products, pick a method (tiers = five income tiers, purchasing_power = GNI PPP, income = GNI Atlas, blended = both, equal = the store's own conversion), floor / ceiling as percent of the base price (floor 5-100, ceiling 100-300), prefer local price endings, a product's base price, one country's own price (override) or reset all overrides of a product, the start date (YYYY-MM-DD, empty = earliest; App Store only), whether existing subscribers keep their price (default true) and whether in-app purchases change at once. Returns the updated get_pricing summary.",
            s::object(
                vec![
                    ("projectId", s::project_id()),
                    ("select", s::array(s::id("Product id."), "Products to plan and publish (replaces the selection).", 0, 200)),
                    ("method", s::enumeration(METHODS, "Pricing method.")),
                    ("floorPct", json!({"type": "number", "minimum": 5, "maximum": 100, "description": "Lowest share of the base price, percent."})),
                    ("ceilingPct", json!({"type": "number", "minimum": 100, "maximum": 300, "description": "Highest share of the full price, percent."})),
                    ("naturalEndings", s::boolean("Prefer the currency's usual endings (x.99, 980).")),
                    ("basePrice", price_patch("Set or clear one product's base price (base country currency).", false)),
                    ("override", price_patch("Set or clear one product's own price in one country (local currency).", true)),
                    ("resetOverrides", s::id("Clear every override of this product.")),
                    ("startDate", s::text("YYYY-MM-DD, or empty for the earliest date.", 0, 10)),
                    ("keepExistingPrices", s::boolean("App Store: existing subscribers keep their price (default true); false lets decreases reach them, increases never do. No effect on Google Play.")),
                    ("iapNow", s::boolean("In-app purchases change at once instead of on the start date.")),
                ],
                &["projectId"],
            ),
            SET,
        ),
        Tool::custom(
            "publish_pricing",
            "Publish prices to the store",
            "DESTRUCTIVE / LIVE: writes the plan's changed prices for the selected products (or productIds) to App Store Connect or Google Play Console; real customers then pay them. Only call after the owner has seen the per-country before/after (get_pricing) and explicitly told you to publish in this conversation. The app shows the owner a confirmation dialog listing products, countries and the start date; nothing is written unless they confirm there (no answer within ~2 minutes = denied, code not_approved). App Store subscriptions keep existing subscribers on their price by default (a price increase never reaches them); Google Play never moves existing subscribers. Background job; follow with wait_for_jobs, then get_pricing for the report.",
            s::object(
                vec![
                    ("projectId", s::project_id()),
                    ("productIds", s::array(s::id("Product id."), "Only these selected products (default: every selected product with changes).", 1, 200)),
                    ("ownerApproved", json!({ "const": true, "description": "Must be true: the owner explicitly approved these prices." })),
                    ("approval", s::text("Quote the owner's explicit go-ahead.", 3, 500)),
                ],
                &["projectId", "ownerApproved", "approval"],
            ),
            PUBLISH,
        ),
    ]
}

pub async fn run(bridge: &Bridge, name: &str, args: &Value) -> Option<Result<Value, BridgeError>> {
    let r = match name {
        "get_pricing" => get(bridge, args).await,
        "plan_pricing" => plan(bridge, args).await,
        "publish_pricing" => publish(bridge, args).await,
        _ => return None,
    };
    Some(r)
}

async fn get(bridge: &Bridge, args: &Value) -> Result<Value, BridgeError> {
    let view = bridge
        .call(
            "get_pricing",
            json!({ "projectId": args["projectId"], "refresh": args.get("refresh").cloned().unwrap_or(json!(false)) }),
        )
        .await?;
    Ok(summarize(&view, args.get("country").and_then(Value::as_str)))
}

fn cents(v: &Value) -> Value {
    match v.as_f64() {
        Some(p) => json!((p * 100.0).round() as i64),
        None => Value::Null,
    }
}

fn price_of(v: &Value) -> Value {
    match v.as_i64() {
        Some(c) => json!(c as f64 / 100.0),
        None => Value::Null,
    }
}

/// The `pricing.plan` patch for the tool arguments (prices to cents).
pub fn patch_from_args(args: &Value) -> Value {
    let mut p = Map::new();
    let copy = [
        ("select", "selected"),
        ("method", "method"),
        ("floorPct", "floorPct"),
        ("ceilingPct", "ceilingPct"),
        ("naturalEndings", "naturalEndings"),
        ("resetOverrides", "resetOverrides"),
        ("startDate", "startDate"),
        ("keepExistingPrices", "keepExistingPrices"),
        ("iapNow", "iapNow"),
    ];
    for (from, to) in copy {
        if let Some(v) = args.get(from) {
            p.insert(to.into(), v.clone());
        }
    }
    if let Some(b) = args.get("basePrice") {
        p.insert("base".into(), json!({ "productId": b["productId"], "cents": cents(&b["price"]) }));
    }
    if let Some(o) = args.get("override") {
        p.insert(
            "overridePrice".into(),
            json!({ "productId": o["productId"], "country": o["country"], "cents": cents(&o["price"]) }),
        );
    }
    Value::Object(p)
}

async fn plan(bridge: &Bridge, args: &Value) -> Result<Value, BridgeError> {
    let action = json!({ "type": "pricing.plan", "projectId": args["projectId"], "patch": patch_from_args(args) });
    bridge.dispatch(action).await?;
    get(bridge, &json!({ "projectId": args["projectId"] })).await
}

/// `ownerApproved` / `approval` are a speed bump, not the authorization: the
/// engine also needs the owner to confirm in the app (deny by default).
async fn publish(bridge: &Bridge, args: &Value) -> Result<Value, BridgeError> {
    if args["ownerApproved"] != json!(true) {
        return Err(BridgeError::Invalid(
            "publishing prices needs the owner's explicit approval (ownerApproved: true)".into(),
        ));
    }
    let mut action = json!({ "type": "pricing.publish", "projectId": args["projectId"], "approval": args["approval"] });
    if let Some(ids) = args.get("productIds") {
        action["productIds"] = ids.clone();
    }
    bridge
        .call_with_timeout("dispatch", json!({ "action": action }), PUBLISH_CALL_TIMEOUT)
        .await?;
    queued(bridge, "pricing.publish", args.get("projectId").and_then(Value::as_str)).await
}

fn row(r: &Value) -> Value {
    json!({
        "country": r["country"], "currency": r["currency"],
        "today": price_of(&r["currentCents"]), "scheduled": price_of(&r["scheduledCents"]),
        "scheduledStart": r["scheduledStart"], "proposed": price_of(&r["proposedCents"]),
        "sharePct": r["sharePct"], "by": r["by"], "changed": r["changed"],
        "overridden": r["overridden"], "skipped": r["skipped"],
    })
}

fn product(p: &Value, country: Option<&str>) -> Value {
    let rows: Vec<Value> = p["rows"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|r| country.is_none_or(|c| r["country"] == c))
        .collect::<Vec<_>>()
        .into_iter()
        .take(if country.is_some() { usize::MAX } else { MAX_ROWS })
        .map(row)
        .collect();
    let total = p["rows"].as_array().map_or(0, Vec::len);
    json!({
        "id": p["id"], "kind": p["kind"], "name": p["name"], "productId": p["productId"],
        "detail": p["detail"], "state": p["state"], "group": p["group"], "selected": p["selected"],
        "needsStartDate": p["needsStartDate"], "baseToday": price_of(&p["baseCurrentCents"]),
        "basePrice": price_of(&p["baseCents"]), "baseSuggestedFrom": p["baseFrom"],
        "changed": p["changed"], "error": p["error"], "rows": rows,
        "rowsOmitted": total.saturating_sub(rows.len()),
    })
}

/// Compact view for agents: decimal prices, rows only for selected products.
pub fn summarize(view: &Value, country: Option<&str>) -> Value {
    let products: Vec<Value> = view["products"].as_array().into_iter().flatten().map(|p| product(p, country)).collect();
    let mut out = json!({
        "status": view["status"], "message": view["message"], "store": view["store"],
        "baseCountry": view["baseCountry"], "baseCurrency": view["baseCurrency"],
        "plan": view["plan"], "earliestStart": view["earliestStart"], "readAt": view["readAt"],
        "methods": view["methods"], "products": products, "lastRun": view["lastRun"],
        "sources": view["sources"],
    });
    if let Some(base) = out["plan"].get_mut("baseCents").map(Value::take) {
        let prices: Map<String, Value> = base
            .as_object()
            .into_iter()
            .flatten()
            .map(|(k, v)| (k.clone(), price_of(v)))
            .collect();
        out["plan"]["basePrices"] = Value::Object(prices);
        if let Some(o) = out["plan"].as_object_mut() {
            o.remove("baseCents");
        }
    }
    if let Some(overrides) = out["plan"].get_mut("overrides").map(Value::take) {
        // Every price an agent sees is a decimal, like the ones it sends back.
        let prices: Map<String, Value> = overrides
            .as_object()
            .into_iter()
            .flatten()
            .map(|(product, by_country)| {
                let countries: Map<String, Value> = by_country
                    .as_object()
                    .into_iter()
                    .flatten()
                    .map(|(c, v)| (c.clone(), price_of(v)))
                    .collect();
                (product.clone(), Value::Object(countries))
            })
            .collect();
        out["plan"]["overrides"] = Value::Object(prices);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_arguments_become_a_cents_patch() {
        let p = patch_from_args(&json!({
            "projectId": "1", "select": ["sub:1"], "method": "income", "floorPct": 40,
            "basePrice": {"productId": "sub:1", "price": 9.99},
            "override": {"productId": "sub:1", "country": "de", "price": null},
        }));
        assert_eq!(p["selected"], json!(["sub:1"]));
        assert_eq!(p["floorPct"], 40);
        assert_eq!(p["base"], json!({"productId": "sub:1", "cents": 999}));
        assert_eq!(p["overridePrice"], json!({"productId": "sub:1", "country": "de", "cents": null}));
        assert!(p.get("projectId").is_none());
    }

    #[test]
    fn summaries_use_decimal_prices_and_filter_by_country() {
        let view = json!({
            "status": "ok", "plan": {"baseCents": {"sub:1": 999}, "overrides": {"sub:1": {"de": 399}}, "method": "tiers"},
            "products": [{"id": "sub:1", "baseCents": 999, "rows": [
                {"country": "us", "currentCents": 999, "proposedCents": 999},
                {"country": "in", "currentCents": 89_900, "proposedCents": 29_900}
            ]}]
        });
        let s = summarize(&view, Some("in"));
        assert_eq!(s["products"][0]["basePrice"], 9.99);
        assert_eq!(s["products"][0]["rows"].as_array().unwrap().len(), 1);
        assert_eq!(s["products"][0]["rows"][0]["proposed"], 299.0);
        assert_eq!(s["plan"]["basePrices"]["sub:1"], 9.99);
        assert!(s["plan"].get("baseCents").is_none());
        assert_eq!(s["plan"]["overrides"]["sub:1"]["de"], 3.99);
    }
}
