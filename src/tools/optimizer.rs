//! Listing optimizer: generate options, choose, localize, edit, publish.

use std::time::Duration;

use serde_json::{json, Value};

use super::schema as s;
use super::{queued, Annotations, Tool, DESTRUCTIVE, SET, WORK, WRITE};
use crate::error::BridgeError;
use crate::ipc::Bridge;

/// How long `publish_listing` waits for the app: the engine shows the owner a
/// native confirmation dialog (and gives up after ~120 s), so this must be
/// longer than the engine's confirmation timeout.
const PUBLISH_CALL_TIMEOUT: Duration = Duration::from_secs(150);

/// Publishing changes the live store listing: destructive, not idempotent.
const PUBLISH: Annotations = Annotations {
    read_only: false,
    destructive: true,
    idempotent: false,
    open_world: true,
};

pub fn tools() -> Vec<Tool> {
    vec![
        Tool::action(
            "optimizer_start",
            "Generate listing options",
            "Start the listing optimizer for a project: scouts the board's keywords in the main locale and has the AI write several complete listing options (title, subtitle/short description, keyword field, description) scored for reach and difficulty. Replaces previous options. Background job; then read get_optimizer.",
            s::object(
                vec![
                    ("projectId", s::project_id()),
                    ("locale", s::locale("Main listing locale, e.g. \"en-US\".")),
                    ("directions", s::directions()),
                    ("ai", s::ai_provider()),
                ],
                &["projectId", "locale"],
            ),
            WORK,
            "optimizer.start",
            &[],
            true,
        ),
        Tool::action(
            "screenshots_write_headings",
            "Write screenshot headings",
            "Have the AI write the one-line heading printed on each screenshot slide, in the main locale (verb first, one benefit, at most 34 characters; 40 for scripts without case). Slides that already have a heading are kept unless overwrite is true or slideId names one slide. When the AI can see images it looks at each slide's screenshot. Background job; then read get_screenshots.",
            s::object(
                vec![
                    ("projectId", s::project_id()),
                    ("slideId", s::id("Only rewrite this slide.")),
                    ("overwrite", s::boolean("Rewrite slides that already have a heading.")),
                    ("directions", s::directions()),
                    ("ai", s::ai_provider()),
                ],
                &["projectId"],
            ),
            WORK,
            "screenshots.headings",
            &[],
            true,
        ),
        Tool::action(
            "screenshots_translate",
            "Translate screenshot headings",
            "Adapt the main screenshot headings to the project's other languages (or only `locales`), reusing each language's own search words. Every slide needs a main heading first. Subheadings and notification texts are adapted too. Background job; then read get_screenshots (locales show translated / stale / missing).",
            s::object(
                vec![
                    ("projectId", s::project_id()),
                    ("locales", s::array(s::locale("Listing locale."), "Only these locales.", 1, 60)),
                    ("ai", s::ai_provider()),
                ],
                &["projectId"],
            ),
            WORK,
            "screenshots.translate",
            &[],
            true,
        ),
        Tool::action(
            "optimizer_more_options",
            "More listing options",
            "Generate additional listing options next to the existing ones.",
            s::object(
                vec![
                    ("projectId", s::project_id()),
                    ("count", s::integer("How many more options.", 1, 10)),
                    ("directions", s::directions()),
                ],
                &["projectId", "count"],
            ),
            WORK,
            "optimizer.more",
            &[],
            true,
        ),
        Tool::custom(
            "optimizer_save_option",
            "Add or edit a listing option",
            "Add your own listing option (omit optionId) or replace the listing of an existing option. The app re-scores it and reports validation problems (length limits, repeated words) in get_optimizer.",
            s::object(
                vec![
                    ("projectId", s::project_id()),
                    ("optionId", s::id("Option to replace; omit to add a new one.")),
                    ("listing", s::listing()),
                    ("label", s::text("Name for a new option.", 1, 100)),
                ],
                &["projectId", "listing"],
            ),
            WRITE,
        ),
        Tool::action(
            "optimizer_set_links",
            "What's New and listing links",
            "Set What's New and the listing links written by the next publish. Apple: whatsNew, marketingUrl, supportUrl, privacyUrl for the main language (or every published language with allLanguages). Google Play: marketingUrl is the contact website, plus contactEmail. Omitted fields stay; empty strings clear. publish must be true for publish_listing to write them. Links must start with https://.",
            s::object(
                vec![
                    ("projectId", s::project_id()),
                    ("whatsNew", s::text("App Store What's New for the next version.", 0, 4000)),
                    ("marketingUrl", s::text("Marketing URL (Google: contact website).", 0, 255)),
                    ("supportUrl", s::text("Support URL (Apple).", 0, 255)),
                    ("privacyUrl", s::text("Privacy policy URL (Apple).", 0, 255)),
                    ("contactEmail", s::text("Contact email (Google Play).", 0, 254)),
                    ("allLanguages", s::boolean("Apple: write to every published language, not only the main one.")),
                    ("publish", s::boolean("Include these in the next publish.")),
                ],
                &["projectId"],
            ),
            SET,
            "optimizer.links.set",
            &[],
            false,
        ),
        Tool::action(
            "optimizer_remove_option",
            "Remove listing option",
            "Delete one listing option (generated or custom) from the optimizer.",
            s::object(
                vec![
                    ("projectId", s::project_id()),
                    ("optionId", s::id("Option id.")),
                ],
                &["projectId", "optionId"],
            ),
            DESTRUCTIVE,
            "optimizer.option.remove",
            &[],
            false,
        ),
        Tool::action(
            "optimizer_choose",
            "Choose listing option",
            "Pick the listing option to ship and the core phrases it must carry into every locale. Next step: optimizer_localize.",
            s::object(
                vec![
                    ("projectId", s::project_id()),
                    ("optionId", s::id("Chosen option id.")),
                    ("corePhrases", s::array(s::text("Phrase.", 1, 100), "Main-locale phrases to keep in every locale.", 0, 50)),
                ],
                &["projectId", "optionId", "corePhrases"],
            ),
            SET,
            "optimizer.choose",
            &[],
            false,
        ),
        Tool::action(
            "optimizer_localize",
            "Localize chosen listing",
            "Write per-locale drafts of the chosen listing: the core phrases are translated, candidate local phrasings are measured in each locale's test market, and the best are used. A locale that fails does not stop the others; get_optimizer lists failedLocales and lastError. Background job.",
            s::object(
                vec![
                    ("projectId", s::project_id()),
                    ("locales", s::array(s::locale("Locale."), "Only these locales (default: see scope).", 1, 60)),
                    ("directions", s::directions()),
                    ("remaining", s::boolean("Only locales without a draft yet or that failed last time (finish an interrupted run).")),
                    ("scope", s::enumeration(&["markets", "all"], "Without locales: the tracked markets' locales (markets, default) or every listing locale of the store in fast mode (all: batched AI candidates, parallel, measured in each locale's test market only).")),
                ],
                &["projectId"],
            ),
            WORK,
            "optimizer.localize",
            &[],
            true,
        ),
        Tool::action(
            "optimizer_edit_locale",
            "Edit a locale draft",
            "Change fields of one locale's draft listing (empty string clears a field).",
            s::object(
                vec![
                    ("projectId", s::project_id()),
                    ("locale", s::locale("Locale of the draft.")),
                    ("fields", s::listing_patch()),
                ],
                &["projectId", "locale", "fields"],
            ),
            SET,
            "optimizer.locale.set",
            &[],
            false,
        ),
        Tool::action(
            "optimizer_pick_phrase",
            "Pick local phrasing",
            "In one locale draft, use a different measured candidate phrase for a core phrase (from that locale's candidates in get_optimizer).",
            s::object(
                vec![
                    ("projectId", s::project_id()),
                    ("locale", s::locale("Locale of the draft.")),
                    ("keyword", s::text("Core phrase (main locale) being translated.", 1, 100)),
                    ("phrase", s::text("Candidate phrase to use.", 1, 100)),
                ],
                &["projectId", "locale", "keyword", "phrase"],
            ),
            SET,
            "optimizer.locale.pick",
            &[],
            false,
        ),
        Tool::action(
            "optimizer_review",
            "Final review of the listing",
            "Review the localized listing: names and subtitles of every language against the approved main listing (wrong meaning, dropped topic, doubled word, wrong regional variant or script, stray English, over the limit), a rejection-risk scan of keyword fields and copy (vulgar words, other brands, medical claims, price/free words, missing features, wrong script, changed links) and rule checks. Fixes are applied most severe first when they fit; the rest are listed as looks. Runs by itself after optimizer_localize; get_optimizer shows review.changes / review.looks. Never blocks publishing. Background job.",
            s::object(
                vec![
                    ("projectId", s::project_id()),
                    ("parts", s::array(s::enumeration(&["names", "keywords", "copy"], "Part."), "Only these parts (default all).", 1, 3)),
                ],
                &["projectId"],
            ),
            WORK,
            "optimizer.review",
            &[],
            true,
        ),
        Tool::action(
            "optimizer_review_undo",
            "Undo a review change",
            "Put one final-review change back (changeId from get_optimizer review.changes).",
            s::object(vec![("projectId", s::project_id()), ("changeId", s::id("Review change id."))], &["projectId", "changeId"]),
            SET,
            "optimizer.review.undo",
            &[],
            false,
        ),
        Tool::action(
            "optimizer_reset",
            "Reset optimizer",
            "Discard all options, the choice and every locale draft for the project.",
            s::object(vec![("projectId", s::project_id())], &["projectId"]),
            DESTRUCTIVE,
            "optimizer.clear",
            &[],
            false,
        ),
        Tool::custom(
            "publish_listing",
            "Publish listing to the store",
            "DESTRUCTIVE / LIVE: pushes the chosen listing and every ready locale draft to App Store Connect or Google Play Console, changing what real users see. Only call after the owner has reviewed the exact listing text and explicitly told you to publish in this conversation; never on your own initiative and never as part of a larger plan they approved in general terms. Requires a connected console. The app then shows the owner a confirmation dialog listing the store, app, version, locales and fields; nothing is published unless they click Publish there (no answer within ~2 minutes = denied, error code not_approved). Background job once approved; follow with wait_for_jobs and report the outcome.",
            s::object(
                vec![
                    ("projectId", s::project_id()),
                    ("versionString", s::text("App version to attach the listing to (see get_next_version); default: the app's suggestion.", 1, 32)),
                    ("ownerApproved", json!({ "const": true, "description": "Must be true: the owner explicitly approved publishing this listing." })),
                    ("approval", s::text("Quote the owner's explicit go-ahead.", 3, 500)),
                ],
                &["projectId", "ownerApproved", "approval"],
            ),
            PUBLISH,
        ),
        Tool::custom(
            "upload_app_name",
            "Upload a replacement name",
            "DESTRUCTIVE / LIVE: App Store Connect refused a language's name because another app uses it (get_optimizer renames: locale, the refused name and up to 3 AI proposals; the rest of that language was already uploaded). Nothing is renamed automatically. Show the owner the proposals, let them pick or edit one, and only after they told you which name to use in this conversation call this with that exact name (≤ 30 characters). The app then asks the owner to confirm in a native dialog. Background job; a name that is taken too is reported (renames.tried) and the job fails with conflict. App Store only; never submits for review.",
            s::object(
                vec![
                    ("projectId", s::project_id()),
                    ("locale", s::locale("The language whose name was refused, e.g. \"de-DE\".")),
                    ("name", s::text("The exact name the owner chose.", 1, 30)),
                    ("ownerApproved", json!({ "const": true, "description": "Must be true: the owner chose this exact name." })),
                    ("approval", s::text("Quote the owner's explicit go-ahead.", 3, 500)),
                ],
                &["projectId", "locale", "name", "ownerApproved", "approval"],
            ),
            PUBLISH,
        ),
    ]
}

pub async fn run(bridge: &Bridge, name: &str, args: &Value) -> Option<Result<Value, BridgeError>> {
    let r = match name {
        "optimizer_save_option" => save_option(bridge, args).await,
        "publish_listing" => publish(bridge, args).await,
        "upload_app_name" => upload_name(bridge, args).await,
        _ => return None,
    };
    Some(r)
}

async fn save_option(bridge: &Bridge, args: &Value) -> Result<Value, BridgeError> {
    let action = match args.get("optionId") {
        Some(id) => json!({
            "type": "optimizer.option.update",
            "projectId": args["projectId"], "optionId": id, "listing": args["listing"],
        }),
        None => {
            let mut a = json!({
                "type": "optimizer.option.add",
                "projectId": args["projectId"], "listing": args["listing"],
            });
            if let Some(l) = args.get("label") {
                a["label"] = l.clone();
            }
            a
        }
    };
    bridge.dispatch(action.clone()).await?;
    Ok(
        json!({ "ok": true, "action": action["type"], "next": "Read get_optimizer for scores and validation problems." }),
    )
}

/// `ownerApproved` / `approval` are a speed bump for well-behaved agents, NOT
/// an authorization: any agent (or prompt injection) can set them. The real
/// gate is in the engine: an `optimizer.publish` arriving over IPC needs the
/// owner to approve a native dialog in the app (`HostServices::confirm_publish`,
/// deny by default / on timeout).
async fn publish(bridge: &Bridge, args: &Value) -> Result<Value, BridgeError> {
    if args["ownerApproved"] != json!(true) {
        return Err(BridgeError::Invalid(
            "publishing needs the owner's explicit approval (ownerApproved: true)".into(),
        ));
    }
    let mut action = json!({ "type": "optimizer.publish", "projectId": args["projectId"] });
    if let Some(v) = args.get("versionString") {
        action["versionString"] = v.clone();
    }
    bridge
        .call_with_timeout("dispatch", json!({ "action": action }), PUBLISH_CALL_TIMEOUT)
        .await?;
    queued(
        bridge,
        "optimizer.publish",
        args.get("projectId").and_then(Value::as_str),
    )
    .await
}

/// Same speed bump as [`publish`]: the engine asks the owner to confirm an
/// `optimizer.name.upload` that arrives over IPC.
async fn upload_name(bridge: &Bridge, args: &Value) -> Result<Value, BridgeError> {
    if args["ownerApproved"] != json!(true) {
        return Err(BridgeError::Invalid(
            "uploading a name needs the owner's explicit choice (ownerApproved: true)".into(),
        ));
    }
    let action = json!({
        "type": "optimizer.name.upload",
        "projectId": args["projectId"], "locale": args["locale"], "name": args["name"],
    });
    bridge
        .call_with_timeout("dispatch", json!({ "action": action }), PUBLISH_CALL_TIMEOUT)
        .await?;
    queued(
        bridge,
        "optimizer.name.upload",
        args.get("projectId").and_then(Value::as_str),
    )
    .await
}
