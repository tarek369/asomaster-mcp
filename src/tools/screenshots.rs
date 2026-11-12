//! Screenshot designer: edit slides and the theme by field, add images
//! (store import or a local file), render in the open app, export, upload.
//! (Heading writing / translation live with the optimizer tools.)

use std::time::Duration;

use serde_json::{json, Value};

use super::schema as s;
use super::{queued, Annotations, Tool, SET, WORK, WRITE};
use crate::error::BridgeError;
use crate::ipc::Bridge;

/// The engine waits up to ~120 s for the owner's click; wait a little longer.
const UPLOAD_CALL_TIMEOUT: Duration = Duration::from_secs(150);

const UPLOAD: Annotations = Annotations { read_only: false, destructive: true, idempotent: false, open_world: true };

const DEVICES: &[&str] = &["iphone", "iphone_65", "ipad", "ipad_129", "ipad_11", "mac", "watch", "phone", "tablet", "feature"];
const LAYOUTS: &[&str] = &["centered", "bleed", "zoomed", "duo", "tilted"];
/// Devices the stores also take in landscape.
const ROTATING: &[&str] = &["iphone", "iphone_65", "ipad", "ipad_129", "ipad_11", "phone", "tablet"];

fn color(desc: &str) -> Value {
    json!({ "type": "string", "pattern": "^#[0-9a-fA-F]{6}$", "description": desc })
}

fn style() -> Value {
    s::object(
        vec![
            ("background", color("Own background color (#rrggbb); omit to use the theme's.")),
            ("backgroundEnd", color("With background: second gradient color.")),
            ("backgroundAngle", s::integer("Gradient angle in degrees (180 = top to bottom).", 0, 359)),
            ("textColor", color("Heading color; omit for the theme's color (black/white when that is hard to read).")),
            ("textSize", s::integer("Heading size, percent of the default.", 60, 150)),
            ("layout", s::enumeration(LAYOUTS, "Device layout of this slide.")),
        ],
        &[],
    )
}

fn layer() -> Value {
    s::object(
        vec![
            ("x", s::integer("Move right (percent of the width; negative = left).", -50, 50)),
            ("y", s::integer("Move down (percent of the height; negative = up).", -50, 50)),
            ("scale", s::integer("Device size, percent of the layout's.", 50, 200)),
            ("rotation", s::integer("Extra turn in degrees, clockwise.", -45, 45)),
        ],
        &[],
    )
}

fn notification() -> Value {
    s::object(
        vec![
            ("style", s::enumeration(&["ios", "android"], "Look of the card.")),
            ("appName", s::text("App name on the card.", 0, 120)),
            ("title", s::text("Title (in the given locale).", 0, 120)),
            ("body", s::text("Body (in the given locale).", 0, 120)),
            ("y", s::integer("Top edge, percent of the height.", 0, 90)),
            ("width", s::integer("Width, percent of the screenshot.", 40, 100)),
        ],
        &[],
    )
}

pub fn tools() -> Vec<Tool> {
    vec![
        Tool::action(
            "screenshots_edit_slide",
            "Edit a screenshot slide",
            "Change one slide of the screenshot design (or add a slide at the end when slideId is omitted): heading (*word* highlights; at most 34 characters, 40 for scripts without case), subheading, plain (show the image as it is), layout (centered, bleed, zoomed, duo, tilted), per-slide style (background / gradient / text color / text size), device position and a notification card; style, device and notification change only the fields given. Texts go to `locale` (default the main locale); other locales get their own translation. Read get_screenshots for slide ids, then render with screenshots_render.",
            s::object(
                vec![
                    ("projectId", s::project_id()),
                    ("slideId", s::id("Slide to change; omit to add a new slide.")),
                    ("locale", s::locale("Locale of the texts (default: main).")),
                    ("heading", s::text("Heading.", 0, 200)),
                    ("subheading", s::text("Smaller line under the heading.", 0, 200)),
                    ("plain", s::boolean("Show the image as it is: no heading, background or frame.")),
                    ("layout", s::enumeration(LAYOUTS, "Device layout of this slide.")),
                    ("style", style()),
                    ("device", layer()),
                    ("notification", notification()),
                    ("clearNotification", s::boolean("Remove the notification card.")),
                ],
                &["projectId"],
            ),
            SET,
            "screenshots.slide.set",
            &[],
            false,
        ),
        Tool::action(
            "screenshots_set_theme",
            "Set the screenshot theme",
            "Change the look shared by every slide (any subset): background, backgroundEnd (null = solid), backgroundAngle, textColor, accent (highlighted words), font (inter, poppins, playfair, nunito, barlow), caps, position (top, bottom), frame (none, dark, light), layout, textSize; and/or the devices to render in store order; and/or `landscape`, the devices drawn in landscape (iPhone 2868×1320, iPhone 6.5″ 2778×1284, iPad 13″ 2752×2064, iPad 12.9″ 2732×2048, iPad 11″ 2388×1668, Play phone 1920×1080, Play 10″ tablet 2560×1600; [] = all portrait; Mac, Watch and the feature graphic have one shape). Then render with screenshots_render.",
            s::object(
                vec![
                    ("projectId", s::project_id()),
                    (
                        "theme",
                        s::object(
                            vec![
                                ("background", color("Background color.")),
                                ("backgroundEnd", json!({ "type": ["string", "null"], "pattern": "^#[0-9a-fA-F]{6}$", "description": "Second gradient color, null for solid." })),
                                ("backgroundAngle", s::integer("Gradient angle in degrees.", 0, 359)),
                                ("textColor", color("Heading color.")),
                                ("accent", color("Color of *highlighted* words.")),
                                ("font", s::enumeration(&["inter", "poppins", "playfair", "nunito", "barlow"], "Bundled font family.")),
                                ("caps", s::boolean("Upper-case headings.")),
                                ("position", s::enumeration(&["top", "bottom"], "Heading above or below the device.")),
                                ("frame", s::enumeration(&["none", "dark", "light"], "Device body.")),
                                ("layout", s::enumeration(LAYOUTS, "Default device layout.")),
                                ("textSize", s::integer("Heading size, percent.", 60, 150)),
                            ],
                            &[],
                        ),
                    ),
                    ("devices", s::array(s::enumeration(DEVICES, "Device."), "Devices to render.", 1, 10)),
                    ("landscape", s::array(s::enumeration(ROTATING, "Device."), "Devices drawn in landscape (the rest portrait).", 0, 7)),
                ],
                &["projectId"],
            ),
            SET,
            "screenshots.theme.set",
            &[],
            false,
        ),
        Tool::custom(
            "screenshots_import_store",
            "Import live store screenshots",
            "Download the screenshots on the app's live store page for a locale (default main) and put them on slides in order, each image once (new slides are plain: they already carry their text). Play images are sorted into phone and tablet by shape (the tablet is switched on when there are tablet images); in the main locale a device whose store images are landscape is turned to landscape. Returns how many images were placed, repeats left out and devices turned.",
            s::object(vec![("projectId", s::project_id()), ("locale", s::locale("Listing locale (default: main)."))], &["projectId"]),
            WRITE,
        ),
        Tool::action(
            "screenshots_add_image",
            "Add a screenshot image",
            "Put a PNG or JPEG from the user's Downloads, Desktop or Documents folder on a slide for one device (and locale; default main, which every locale shares). Use the raw app screenshot taken on that device, not a finished store image.",
            s::object(
                vec![
                    ("projectId", s::project_id()),
                    ("path", s::text("Absolute path of the image file.", 2, 1024)),
                    ("slideId", s::id("Slide to put it on.")),
                    ("device", s::enumeration(DEVICES, "Device the image is for.")),
                    ("locale", s::locale("Only for this locale (default: main, shared).")),
                ],
                &["projectId", "path", "slideId", "device"],
            ),
            SET,
            "screenshots.image.import",
            &[],
            false,
        ),
        Tool::action(
            "screenshots_render",
            "Render screenshots",
            "Ask the open ASOMaster window to draw every screenshot still missing for the current design (bundled fonts, every locale and device). Returns at once; poll get_screenshots until every target is done. A heading too long to fit is reported in the app and stops that render.",
            s::object(vec![("projectId", s::project_id())], &["projectId"]),
            WORK,
            "screenshots.render",
            &[],
            false,
        ),
        Tool::custom(
            "screenshots_export",
            "Export screenshots",
            "Write the complete rendered set as a zip into the app's exports folder: layout \"zip\" (a folder per locale plus manifest.json) or \"fastlane\" (deliver / supply folders). Returns the file path. Every screenshot must be rendered for the current design.",
            s::object(vec![("projectId", s::project_id()), ("layout", s::enumeration(&["zip", "fastlane"], "Folder layout (default zip)."))], &["projectId"]),
            WRITE,
        ),
        Tool::custom(
            "screenshots_upload",
            "Upload screenshots to the store",
            "LIVE: replaces the screenshots of the editable App Store version or the Play listing with the rendered set (unchanged sets are skipped; nothing is submitted for review). `locales` uploads only those languages (e.g. just the main one; only their screenshots must be rendered) and leaves the others as they are in the store. Only call after the owner has seen the screenshots and told you to upload them in this conversation. The app then shows the owner a confirmation dialog; nothing is sent unless they approve there (no answer in ~5 minutes = denied, error not_approved). dryRun checks everything without changing the store and needs no dialog. Background job; follow with wait_for_jobs.",
            s::object(
                vec![
                    ("projectId", s::project_id()),
                    ("dryRun", s::boolean("Check only; change nothing in the store.")),
                    ("locales", s::array(s::locale("Listing locale."), "Only these languages (default: every language).", 1, 60)),
                    ("ownerApproved", json!({ "const": true, "description": "Must be true: the owner explicitly approved this upload." })),
                    ("approval", s::text("Quote the owner's explicit go-ahead.", 3, 500)),
                ],
                &["projectId", "ownerApproved", "approval"],
            ),
            UPLOAD,
        ),
    ]
}

pub async fn run(bridge: &Bridge, name: &str, args: &Value) -> Option<Result<Value, BridgeError>> {
    let r = match name {
        "screenshots_import_store" => import(bridge, args).await,
        "screenshots_export" => export(bridge, args).await,
        "screenshots_upload" => upload(bridge, args).await,
        _ => return None,
    };
    Some(r)
}

async fn import(bridge: &Bridge, args: &Value) -> Result<Value, BridgeError> {
    let mut params = json!({ "projectId": args["projectId"] });
    if let Some(l) = args.get("locale") {
        params["locale"] = l.clone();
    }
    bridge.call("screenshots_import", params).await
}

/// The path only: the zip itself stays on disk (it can be tens of MB).
async fn export(bridge: &Bridge, args: &Value) -> Result<Value, BridgeError> {
    let layout = args.get("layout").cloned().unwrap_or_else(|| json!("zip"));
    let file = bridge.call("export_screenshots", json!({ "projectId": args["projectId"], "layout": layout })).await?;
    Ok(json!({ "fileName": file["fileName"], "path": file["path"], "next": "Tell the owner where the file is." }))
}

/// `ownerApproved` / `approval` are a speed bump, not the gate: the engine
/// asks the owner in a native dialog before an agent's upload starts.
async fn upload(bridge: &Bridge, args: &Value) -> Result<Value, BridgeError> {
    if args["ownerApproved"] != json!(true) {
        return Err(BridgeError::Invalid("uploading needs the owner's explicit approval (ownerApproved: true)".into()));
    }
    let mut action = json!({ "type": "screenshots.upload", "projectId": args["projectId"] });
    if let Some(d) = args.get("dryRun") {
        action["dryRun"] = d.clone();
    }
    if let Some(l) = args.get("locales") {
        action["locales"] = l.clone();
    }
    bridge.call_with_timeout("dispatch", json!({ "action": action }), UPLOAD_CALL_TIMEOUT).await?;
    queued(bridge, "screenshots.upload", args.get("projectId").and_then(Value::as_str)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upload_and_slide_tools_are_marked_honestly() {
        let t = tools();
        let upload = t.iter().find(|t| t.name == "screenshots_upload").unwrap();
        assert!(upload.annotations.destructive && !upload.annotations.read_only);
        assert_eq!(upload.input_schema["required"], json!(["projectId", "ownerApproved", "approval"]));
        let edit = t.iter().find(|t| t.name == "screenshots_edit_slide").unwrap();
        assert_eq!(edit.input_schema["properties"]["layout"]["enum"], json!(LAYOUTS));
        assert_eq!(t.len(), 7);
        assert!(upload.input_schema["properties"]["locales"].is_object(), "main-language uploads");
        let theme = t.iter().find(|t| t.name == "screenshots_set_theme").unwrap();
        assert_eq!(theme.input_schema["properties"]["landscape"]["items"]["enum"], json!(ROTATING));
    }
}
