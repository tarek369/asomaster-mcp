//! tools/list: catalogue size, schema validity and annotations.
#![cfg(unix)]

mod common;

use std::collections::HashSet;

use asomaster_mcp::tools::registry;
use serde_json::{json, Value};

fn tools_list() -> Vec<Value> {
    registry().tools().iter().map(|t| t.to_mcp()).collect()
}

/// Every object schema (recursively) must be closed.
fn assert_closed(name: &str, path: &str, schema: &Value) {
    if schema["type"] == "object" {
        assert_eq!(
            schema["additionalProperties"],
            json!(false),
            "{name}{path}: object schema must set additionalProperties:false"
        );
        if let Some(props) = schema["properties"].as_object() {
            for (k, v) in props {
                assert!(
                    v["description"].is_string() || v["type"] == "object",
                    "{name}{path}.{k}: property needs a description"
                );
                assert_closed(name, &format!("{path}.{k}"), v);
            }
        }
        if let Some(req) = schema["required"].as_array() {
            for r in req {
                assert!(
                    schema["properties"].get(r.as_str().unwrap()).is_some(),
                    "{name}{path}: required {r} is not a property"
                );
            }
        }
    }
    if schema["type"] == "array" {
        assert_closed(name, &format!("{path}[]"), &schema["items"]);
    }
}

#[test]
fn catalogue_size_and_unique_valid_names() {
    let tools = tools_list();
    assert!(
        (40..=70).contains(&tools.len()),
        "expected ~40-70 tools, got {}",
        tools.len()
    );
    let mut names = HashSet::new();
    for t in &tools {
        let name = t["name"].as_str().unwrap();
        assert!(names.insert(name.to_string()), "duplicate tool {name}");
        assert!(
            name.len() <= 64 && name.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
            "bad tool name {name}"
        );
        let desc = t["description"].as_str().unwrap();
        assert!(desc.len() >= 40, "{name}: description too short");
        assert!(t["title"].is_string());
    }
}

#[test]
fn every_schema_is_valid_json_schema_and_closed() {
    for t in tools_list() {
        let name = t["name"].as_str().unwrap();
        let schema = &t["inputSchema"];
        assert_eq!(
            schema["type"], "object",
            "{name}: inputSchema must be an object schema"
        );
        assert!(
            jsonschema::meta::is_valid(schema),
            "{name}: schema is not valid against its metaschema"
        );
        jsonschema::validator_for(schema)
            .unwrap_or_else(|e| panic!("{name}: schema does not compile: {e}"));
        assert_closed(name, "", schema);
    }
}

#[test]
fn schemas_reject_unknown_properties() {
    let reg = registry();
    for t in reg.tools() {
        let err = reg.validate(t.name, &json!({ "definitelyNotAField": 1 }));
        assert!(err.is_err(), "{} accepted an unknown property", t.name);
    }
}

#[test]
fn annotations_are_present_and_consistent() {
    for t in tools_list() {
        let name = t["name"].as_str().unwrap();
        let a = &t["annotations"];
        for k in [
            "readOnlyHint",
            "destructiveHint",
            "idempotentHint",
            "openWorldHint",
        ] {
            assert!(a[k].is_boolean(), "{name}: missing {k}");
        }
        if a["readOnlyHint"] == true {
            assert_eq!(
                a["destructiveHint"], false,
                "{name}: read-only cannot be destructive"
            );
        }
        if name.starts_with("get_")
            || name.starts_with("list_")
            || name == "search_store"
            || name == "wait_for_jobs"
        {
            assert_eq!(a["readOnlyHint"], true, "{name} should be read-only");
        }
        if name.starts_with("remove_") || name.starts_with("disconnect_") {
            assert_eq!(a["destructiveHint"], true, "{name} should be destructive");
        }
    }
    let publish = tools_list()
        .into_iter()
        .find(|t| t["name"] == "publish_listing")
        .unwrap();
    assert_eq!(publish["annotations"]["destructiveHint"], true);
    assert_eq!(publish["annotations"]["idempotentHint"], false);
    assert_eq!(publish["annotations"]["openWorldHint"], true);
    let required: Vec<&str> = publish["inputSchema"]["required"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(required.contains(&"ownerApproved"));
}

#[test]
fn publish_schema_requires_explicit_approval() {
    let reg = registry();
    assert!(reg
        .validate(
            "publish_listing",
            &json!({"projectId":"1","ownerApproved":false,"approval":"yes go"})
        )
        .is_err());
    assert!(reg
        .validate(
            "publish_listing",
            &json!({"projectId":"1","approval":"yes go"})
        )
        .is_err());
    assert!(reg
        .validate(
            "publish_listing",
            &json!({"projectId":"1","ownerApproved":true,"approval":"publish it"})
        )
        .is_ok());
}
