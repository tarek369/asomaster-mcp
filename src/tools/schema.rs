//! Small JSON Schema builders. Every object schema is closed
//! (`additionalProperties: false`).

use serde_json::{json, Map, Value};

pub fn object(props: Vec<(&str, Value)>, required: &[&str]) -> Value {
    let mut p = Map::new();
    for (k, v) in props {
        p.insert(k.to_string(), v);
    }
    let mut o =
        json!({ "type": "object", "properties": Value::Object(p), "additionalProperties": false });
    if !required.is_empty() {
        o["required"] = json!(required);
    }
    o
}

pub fn empty() -> Value {
    object(vec![], &[])
}

pub fn string(desc: &str) -> Value {
    json!({ "type": "string", "description": desc })
}

pub fn text(desc: &str, min: u64, max: u64) -> Value {
    json!({ "type": "string", "description": desc, "minLength": min, "maxLength": max })
}

pub fn id(desc: &str) -> Value {
    json!({ "type": "string", "description": desc, "minLength": 1, "maxLength": 64 })
}

pub fn boolean(desc: &str) -> Value {
    json!({ "type": "boolean", "description": desc })
}

pub fn integer(desc: &str, min: i64, max: i64) -> Value {
    json!({ "type": "integer", "description": desc, "minimum": min, "maximum": max })
}

pub fn enumeration(values: &[&str], desc: &str) -> Value {
    json!({ "type": "string", "enum": values, "description": desc })
}

pub fn array(items: Value, desc: &str, min: u64, max: u64) -> Value {
    json!({ "type": "array", "items": items, "description": desc, "minItems": min, "maxItems": max })
}

pub fn nullable_id(desc: &str) -> Value {
    json!({ "type": ["string", "null"], "description": desc, "maxLength": 64 })
}

// ------------------------------------------------------------ domain fields

pub fn store() -> Value {
    enumeration(
        &["apple", "google"],
        "Store: \"apple\" (App Store) or \"google\" (Google Play).",
    )
}

pub fn project_id() -> Value {
    id("Project id (from get_overview).")
}

pub fn keyword_id() -> Value {
    id("Keyword id (from get_project_board).")
}

pub fn country() -> Value {
    json!({
        "type": "string", "pattern": "^[A-Za-z]{2}$",
        "description": "Market = ISO 3166-1 alpha-2 country code, e.g. \"us\", \"de\"."
    })
}

pub fn markets(desc: &str) -> Value {
    array(country(), desc, 1, 200)
}

pub fn locale(desc: &str) -> Value {
    json!({ "type": "string", "pattern": "^[A-Za-z]{2,3}(-[A-Za-z0-9]{2,8})*$", "description": desc })
}

pub fn ai_provider() -> Value {
    enumeration(
        &["claude_code", "codex", "opencode", "ollama", "gemini", "cursor", "grok", "antigravity"],
        "AI provider used by the app for generation (see get_ai_status).",
    )
}

pub fn directions() -> Value {
    text(
        "Optional creative direction for the AI, e.g. \"emphasise offline use\".",
        1,
        2000,
    )
}

/// A full store listing for the optimizer.
pub fn listing() -> Value {
    object(
        vec![
            ("locale", locale("Listing locale, e.g. \"en-US\".")),
            (
                "title",
                text("App name / title (Apple 30 chars, Google 30).", 0, 200),
            ),
            ("subtitle", text("Apple subtitle (30 chars).", 0, 200)),
            (
                "keywords",
                text("Apple keyword field, comma separated (100 chars).", 0, 400),
            ),
            (
                "shortDescription",
                text("Google short description (80 chars).", 0, 400),
            ),
            (
                "description",
                text("Full description (4000 chars).", 0, 8000),
            ),
            (
                "promotionalText",
                text("Apple promotional text (170 chars).", 0, 800),
            ),
        ],
        &["locale"],
    )
}

/// `Partial<Listing>` without locale; empty string clears a field.
pub fn listing_patch() -> Value {
    let mut p = listing();
    if let Some(props) = p["properties"].as_object_mut() {
        props.remove("locale");
    }
    if let Some(o) = p.as_object_mut() {
        o.remove("required");
        o.insert("minProperties".into(), json!(1));
        o.insert(
            "description".into(),
            json!("Fields to change; an empty string clears the field."),
        );
    }
    p
}
