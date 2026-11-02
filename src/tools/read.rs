//! Read-only tools.

use serde_json::{json, Value};

use super::schema as s;
use super::{arg_str, Tool, READ, READ_NET};
use crate::error::BridgeError;
use crate::ipc::Bridge;
use crate::summarize::{self, BoardOpts, OverviewFilter};

pub fn tools() -> Vec<Tool> {
    vec![
        Tool::custom(
            "get_overview",
            "Overview",
            "Start here. Lists every tracked project (id, store, kind, markets, keyword and top-10 counts, searches checked vs never checked, audit step, last check), folders, running and recently finished jobs, key settings, license state and unread rank alerts. Use the ids it returns with every other tool.",
            s::object(
                vec![
                    ("store", s::store()),
                    ("folderId", s::id("Only projects in this folder.")),
                    ("kind", s::enumeration(&["mine", "competitor", "brainstorm"], "Only this kind of project.")),
                ],
                &[],
            ),
            READ,
        ),
        Tool::custom(
            "get_project_board",
            "Project keyword board",
            "Keyword x market grid for one project, summarised per market: for each keyword its id, label, row summary and, per shown market, rank / previous rank, ESTIMATED volume & difficulty (estVol/estDiff), REAL popularity (pop, only when a provider is connected), verdict and opportunity. Also the top competitors. With more than 4 tracked markets only the main market is shown unless you pass markets. Hidden keywords are excluded unless section is hidden/all.",
            s::object(
                vec![
                    ("projectId", s::project_id()),
                    ("markets", s::markets("Countries to show (default: all if <= 4 tracked, else the main market).")),
                    ("section", s::enumeration(&["active", "listing", "idea", "hidden", "all"], "Which rows: active = listing + idea (default).")),
                    ("sort", s::enumeration(&["board", "opportunity", "rank", "estVolume", "popularity"], "Row order (default: board order).")),
                    ("limit", s::integer("Max rows per section (default 150).", 1, 1000)),
                ],
                &["projectId"],
            ),
            READ,
        ),
        Tool::custom(
            "get_term_detail",
            "Keyword detail in one market",
            "Everything about one keyword in one market: the current cell (rank, estimates, real popularity, verdict), rank/estimate history by day, and the apps currently ranking for it (position, rating count, installs, whether the phrase matches their title/subtitle).",
            s::object(
                vec![
                    ("projectId", s::project_id()),
                    ("keywordId", s::keyword_id()),
                    ("market", s::country()),
                    ("historyPoints", s::integer("Most recent history points to return (default 30).", 0, 400)),
                    ("topApps", s::integer("How many ranking apps to return (default 10).", 0, 50)),
                ],
                &["projectId", "keywordId", "market"],
            ),
            READ,
        ),
        Tool::custom(
            "list_markets",
            "Markets",
            "Markets (countries) available for a store with search language, listing locales and a 0-10 potential score. Use the country codes as market ids.",
            s::object(
                vec![
                    ("store", s::store()),
                    ("featuredOnly", s::boolean("Only the curated featured markets.")),
                ],
                &["store"],
            ),
            READ,
        ),
        Tool::custom(
            "get_ai_status",
            "AI providers",
            "Status of the local AI providers the app uses for brainstorming, suggestions, the optimizer and the assistant: installed / signed in / ready, current model and the next setup step when not ready.",
            s::empty(),
            READ,
        ),
        Tool::custom(
            "get_popularity_status",
            "Popularity sources",
            "Which REAL popularity sources (Apple Ads, Google Ads Keyword Planner, imported files) are connected, for which store, and when they last fetched. Credentials are never returned.",
            s::empty(),
            READ,
        ),
        Tool::custom(
            "list_rank_alerts",
            "Rank alerts",
            "Rank-change alerts raised by checks (keyword, market, from -> to rank), newest first.",
            s::object(
                vec![
                    ("unreadOnly", s::boolean("Only unread alerts (default true).")),
                    ("projectId", s::id("Only alerts for this project.")),
                    ("limit", s::integer("Max alerts (default 50).", 1, 500)),
                ],
                &[],
            ),
            READ,
        ),
        Tool::custom(
            "get_optimizer",
            "Optimizer state",
            "Listing optimizer state for a project: step, current live listing, generated listing options (with reach/difficulty, covered phrases and validation problems), the chosen option, core phrases and per-locale drafts with translation candidates. Descriptions are cut unless full is true.",
            s::object(
                vec![
                    ("projectId", s::project_id()),
                    ("full", s::boolean("Return full descriptions (default false).")),
                    ("locale", s::locale("Only show this locale's draft in full; others are listed by status.")),
                ],
                &["projectId"],
            ),
            READ,
        ),
        Tool::custom(
            "get_publish_preview",
            "What publishing changes",
            "Before publish_listing: reads the live listing fresh from App Store Connect / Play Console and returns, per language the publish writes, each field it changes as from -> to, isNew for a language the app does not have yet, and the languages that already match (unchanged). source is console (fresh read) or stored (no console answered). Tell the owner what changes before publishing.",
            s::object(vec![("projectId", s::project_id())], &["projectId"]),
            READ_NET,
        ),
        Tool::custom(
            "get_audit",
            "App audit",
            "Where the app stands, 0-100, per tracked market and overall (markets weighted by potential): rankings (share of searches its ranks would get, weighted by demand), listing (how much of the demand its name / subtitle / keyword field can catch), competition (how beatable the phrases it does not lead are), a verdict (strong / steady / needs_work), up to 6 takeaways per market (gap: high demand ranked low; uncovered: listing misses a phrase with demand; beatable: easy and not top 10; hold: top 3 to protect) and the daily total history. Demand is real popularity when connected (demandSource). Use it to pick the next optimizer or keyword step.",
            s::object(vec![("projectId", s::project_id())], &["projectId"]),
            READ,
        ),
        Tool::custom(
            "get_screenshots",
            "Screenshot designer state",
            "The project's screenshot design: slides (heading in the main locale, image per device), theme, headings per locale, render status per locale/device/slot, and checks (errors block export and upload: missing images, headings over the limit, stale or missing translations, low text contrast). Edit headings with screenshots_write_headings / screenshots_translate; images, rendering, export and upload are done in the app.",
            s::object(vec![("projectId", s::project_id())], &["projectId"]),
            READ,
        ),
        Tool::custom(
            "popular_searches",
            "Popular searches around a seed",
            "What people search in one App Store market around a seed word, from the store's own autocomplete (suggestions are ordered by search volume): the seed, the seed plus a space and plus common next letters are asked and merged in order of first appearance (position 1 = most searched). tracked marks phrases already in the project. Google Play answers supported:false (no public source). Good for finding high-demand phrases to add with add_keywords.",
            s::object(
                vec![
                    ("store", s::store()),
                    ("country", s::country()),
                    ("seed", s::text("Seed word or phrase.", 1, 60)),
                    ("projectId", s::id("Mark phrases this project already tracks.")),
                ],
                &["store", "country", "seed"],
            ),
            READ_NET,
        ),
        Tool::custom(
            "get_ratings",
            "App ratings history",
            "The app's average rating and rating count per market over the last days (recorded after each rank check; App Store per storefront, Google Play global): latest values, ratings added over the range (newRatings), change of the average (ratingChange) and the daily points.",
            s::object(
                vec![
                    ("projectId", s::project_id()),
                    ("days", s::integer("Range in days (default 90).", 1, 365)),
                ],
                &["projectId"],
            ),
            READ,
        ),
        Tool::custom(
            "get_volume_calibration",
            "Estimate accuracy",
            "How well the estimated volume tracks real popularity, over every tracked phrase that has both (Apple Ads / Google Ads / imported): pairs, spearman rank correlation (-1..1, null under 10 pairs), meanAbsError on 0-100 and bandAgreement (share in the same low/medium/high band). Use it to judge how far to trust estimates where real popularity is missing.",
            s::empty(),
            READ,
        ),
        Tool::custom(
            "list_rival_changes",
            "Rival listing changes",
            "Name and subtitle changes of tracked rivals (kind competitor), noticed when their listing is re-read (every rank check), newest first: app, locale, field (title or subtitle), from -> to, read. A rival renaming around a phrase is a signal to re-check that phrase.",
            s::empty(),
            READ,
        ),
        Tool::custom(
            "get_app_facts",
            "Competitor facts",
            "For App Store ids (e.g. competitors from get_board) in one store: first release date, date of the current version, version, price, rating count and ratings per day since release (how fast the app gathers reviews; a fresh update and high ratings/day mean an active rival). Google Play answers an empty list (its page has no reliable dates). Ids missing from the answer are not in that store. At most 50 ids.",
            s::object(
                vec![
                    ("store", s::store()),
                    ("ids", s::array(s::text("App Store id.", 1, 20), "App ids.", 1, 50)),
                    ("country", s::country()),
                ],
                &["store", "ids", "country"],
            ),
            READ_NET,
        ),
        Tool::custom(
            "get_next_version",
            "Next app version",
            "The editable app version on the developer console (if any) and the version string the app would suggest for publishing.",
            s::object(vec![("projectId", s::project_id())], &["projectId"]),
            READ_NET,
        ),
        Tool::custom(
            "list_console_apps",
            "Developer console apps",
            "Connected developer console accounts for a store (App Store Connect / Google Play Console) and the apps they can see, with whether each app is already tracked.",
            s::object(vec![("store", s::store())], &["store"]),
            READ_NET,
        ),
        Tool::custom(
            "search_store",
            "Search the store",
            "Live store search in one market (10 apps unless limit asks for more: up to 200 on the App Store; Google Play returns about 30 at most). Accepts a search term or a pasted store URL. Apps already tracked carry trackedAs {projectId, kind}. Use it to find app ids before add_app.",
            s::object(
                vec![
                    ("store", s::store()),
                    ("term", s::text("Search term, app name or store URL.", 1, 500)),
                    ("country", s::country()),
                    ("limit", s::integer("How many apps (default 10).", 1, 200)),
                ],
                &["store", "term", "country"],
            ),
            READ_NET,
        ),
        Tool::custom(
            "get_assistant_thread",
            "Assistant conversation",
            "Recent messages of the in-app ASO assistant for a project (keywords or optimizer scope), including keywords it mentioned and the changes it proposed with their status (proposed / applied / dismissed / failed).",
            s::object(
                vec![
                    ("projectId", s::project_id()),
                    ("scope", s::enumeration(&["keywords", "optimizer"], "Conversation scope.")),
                    ("lastMessages", s::integer("How many recent messages (default 10).", 1, 100)),
                ],
                &["projectId", "scope"],
            ),
            READ,
        ),
    ]
}

fn opt_u64(args: &Value, name: &str, default: u64) -> usize {
    args.get(name).and_then(Value::as_u64).unwrap_or(default) as usize
}

pub async fn run(bridge: &Bridge, name: &str, args: &Value) -> Option<Result<Value, BridgeError>> {
    let r = match name {
        "get_overview" => overview(bridge, args).await,
        "get_project_board" => board(bridge, args).await,
        "get_term_detail" => term(bridge, args).await,
        "list_markets" => bridge
            .call("list_markets", json!({ "store": args["store"] }))
            .await
            .map(|m| summarize::markets(&m, args["featuredOnly"] == json!(true))),
        "get_ai_status" => bridge
            .call("ai_status", json!({}))
            .await
            .map(|v| summarize::ai_status(&v)),
        "get_popularity_status" => bridge
            .call("popularity_status", json!({}))
            .await
            .map(|v| json!({ "sources": summarize::prune(v) })),
        "list_rank_alerts" => bridge.call("list_alerts", json!({})).await.map(|v| {
            summarize::alerts(
                &v,
                args.get("unreadOnly")
                    .and_then(Value::as_bool)
                    .unwrap_or(true),
                args.get("projectId").and_then(Value::as_str),
                opt_u64(args, "limit", 50),
            )
        }),
        "get_optimizer" => bridge
            .call("get_optimizer", json!({ "projectId": args["projectId"] }))
            .await
            .map(|v| {
                summarize::optimizer(
                    &v,
                    args["full"] == json!(true),
                    args.get("locale").and_then(Value::as_str),
                )
            }),
        "get_screenshots" => bridge.call("get_screenshots", json!({ "projectId": args["projectId"] })).await,
        "popular_searches" => {
            bridge
                .call(
                    "popular_searches",
                    json!({ "store": args["store"], "country": args["country"], "seed": args["seed"], "projectId": args.get("projectId").cloned().unwrap_or(Value::Null) }),
                )
                .await
        }
        "get_ratings" => {
            bridge
                .call("get_ratings", json!({ "projectId": args["projectId"], "days": args.get("days").cloned().unwrap_or(json!(90)) }))
                .await
        }
        "get_volume_calibration" => bridge.call("volume_calibration", json!({})).await,
        "get_audit" => bridge.call("get_audit", json!({ "projectId": args["projectId"] })).await,
        "list_rival_changes" => bridge.call("list_listing_changes", json!({})).await,
        "get_app_facts" => {
            bridge
                .call(
                    "app_facts",
                    json!({ "store": args["store"], "ids": args["ids"], "country": args["country"] }),
                )
                .await
        }
        "get_publish_preview" => {
            bridge
                .call("publish_preview", json!({ "projectId": args["projectId"] }))
                .await
        }
        "get_next_version" => {
            bridge
                .call("next_version", json!({ "projectId": args["projectId"] }))
                .await
        }
        "list_console_apps" => console_apps(bridge, args).await,
        "search_store" => search_store(bridge, args).await,
        "get_assistant_thread" => bridge
            .call(
                "get_assistant",
                json!({ "projectId": args["projectId"], "scope": args["scope"] }),
            )
            .await
            .map(|v| summarize::assistant(&v, opt_u64(args, "lastMessages", 10))),
        _ => return None,
    };
    Some(r)
}

async fn overview(bridge: &Bridge, args: &Value) -> Result<Value, BridgeError> {
    let (snap, license, alerts) = tokio::join!(
        bridge.call("get_snapshot", json!({})),
        bridge.call("license_status", json!({})),
        bridge.call("list_alerts", json!({})),
    );
    let snap = snap?;
    let f = OverviewFilter {
        store: args.get("store").and_then(Value::as_str),
        folder_id: args.get("folderId").and_then(Value::as_str),
        kind: args.get("kind").and_then(Value::as_str),
    };
    Ok(summarize::overview(
        &snap,
        license.ok().as_ref(),
        alerts.ok().as_ref(),
        &f,
    ))
}

/// Store search results, each marked with the project tracking it (if any).
async fn search_store(bridge: &Bridge, args: &Value) -> Result<Value, BridgeError> {
    let (found, snap) = tokio::join!(
        bridge.call(
            "search_store",
            json!({ "store": args["store"], "term": args["term"], "country": args["country"], "limit": args["limit"] }),
        ),
        bridge.call("get_snapshot", json!({})),
    );
    let projects = snap.ok().map(|s| s["projects"].clone()).unwrap_or(Value::Null);
    let tracked = |id: &str| {
        projects.as_array().and_then(|ps| {
            ps.iter().find(|p| p["store"] == args["store"] && p["appId"].as_str() == Some(id))
        })
    };
    let apps: Vec<Value> = found?
        .as_array()
        .map(|a| {
            a.iter()
                .map(|app| {
                    let mut o = summarize::app(app);
                    if let Some(p) = tracked(app["id"].as_str().unwrap_or_default()) {
                        o["trackedAs"] = json!({ "projectId": p["id"], "kind": p["kind"] });
                    }
                    o
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(json!({ "results": apps }))
}

async fn board(bridge: &Bridge, args: &Value) -> Result<Value, BridgeError> {
    let b = bridge
        .call(
            "get_board",
            json!({ "projectId": arg_str(args, "projectId")? }),
        )
        .await?;
    let defaults = BoardOpts::default();
    let opts = BoardOpts {
        markets: args.get("markets").and_then(Value::as_array).map(|a| {
            a.iter()
                .filter_map(|m| m.as_str().map(str::to_string))
                .collect()
        }),
        section: args
            .get("section")
            .and_then(Value::as_str)
            .map_or(defaults.section, str::to_string),
        limit: opt_u64(args, "limit", defaults.limit as u64),
        sort: args
            .get("sort")
            .and_then(Value::as_str)
            .map_or(defaults.sort, str::to_string),
    };
    Ok(summarize::board(&b, &opts))
}

async fn term(bridge: &Bridge, args: &Value) -> Result<Value, BridgeError> {
    let t = bridge
        .call(
            "get_term",
            json!({ "projectId": args["projectId"], "keywordId": args["keywordId"], "market": args["market"] }),
        )
        .await?;
    Ok(summarize::term(
        &t,
        opt_u64(args, "historyPoints", 30),
        opt_u64(args, "topApps", 10),
    ))
}

async fn console_apps(bridge: &Bridge, args: &Value) -> Result<Value, BridgeError> {
    let store = args["store"].clone();
    let (accounts, apps) = tokio::join!(
        bridge.call("console_accounts", json!({})),
        bridge.call("console_apps", json!({ "store": store })),
    );
    let accounts: Vec<Value> = accounts?
        .as_array()
        .map(|a| {
            a.iter()
                .filter(|x| x["store"] == store)
                .map(|x| summarize::prune(x.clone()))
                .collect()
        })
        .unwrap_or_default();
    let apps = summarize::prune(apps?);
    Ok(json!({ "accounts": accounts, "apps": apps }))
}
