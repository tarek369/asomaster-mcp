//! tools/call against a fake app on a unix socket: success, action mapping,
//! ApiError mapping, license errors, timeouts, waiting for jobs.
#![cfg(unix)]

mod common;

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use asomaster_mcp::tools::registry;
use common::{api_error, call_tool, default_reply, job, snapshot, FakeApp, Reply};
use serde_json::{json, Value};

/// Minimal valid arguments generated from a schema (required fields only).
fn sample(schema: &Value) -> Value {
    if let Some(c) = schema.get("const") {
        return c.clone();
    }
    if let Some(e) = schema["enum"].as_array() {
        return e[0].clone();
    }
    match schema["type"].as_str() {
        Some("object") => {
            let mut o = serde_json::Map::new();
            let mut req: Vec<String> = schema["required"]
                .as_array()
                .map(|r| r.iter().map(|v| v.as_str().unwrap().to_string()).collect())
                .unwrap_or_default();
            if let Some(first) = schema["anyOf"][0]["required"].as_array() {
                req.extend(first.iter().map(|v| v.as_str().unwrap().to_string()));
            }
            if req.is_empty() && schema["minProperties"].as_u64().unwrap_or(0) > 0 {
                req.extend(
                    schema["properties"]
                        .as_object()
                        .unwrap()
                        .keys()
                        .take(1)
                        .cloned(),
                );
            }
            for k in req {
                o.insert(k.clone(), sample(&schema["properties"][&k]));
            }
            Value::Object(o)
        }
        Some("array") => json!([sample(&schema["items"])]),
        Some("integer") => schema["minimum"].clone(),
        Some("boolean") => json!(true),
        _ => match schema["pattern"].as_str() {
            Some(p) if p.contains("A-Za-z]{2}$") => json!("us"),
            Some(_) => json!("en-US"),
            None => json!("abc"),
        },
    }
}

#[tokio::test]
async fn every_tool_runs_with_minimal_valid_args() {
    let app = FakeApp::start(default_reply).await;
    let server = app.server();
    let reg = registry();
    for t in reg.tools() {
        let args = sample(&t.input_schema);
        reg.validate(t.name, &args)
            .unwrap_or_else(|e| panic!("{}: generated args invalid: {e}", t.name));
        let (is_error, payload) = call_tool(&server, t.name, args.clone()).await;
        // Credential tools legitimately reject sample args (no key given).
        if ["connect_console", "connect_popularity", "remove_project"].contains(&t.name) {
            assert!(is_error, "{}", t.name);
            assert_eq!(payload["error"]["code"], "invalid_input");
            continue;
        }
        if t.name == "import_file" {
            continue; // reads a real file in preview mode; covered below
        }
        assert!(!is_error, "{} failed with {payload} (args {args})", t.name);
    }
}

#[tokio::test]
async fn tools_cover_every_agent_facing_engine_action() {
    let app = FakeApp::start(default_reply).await;
    let server = app.server();
    let reg = registry();
    for t in reg.tools() {
        let args = sample(&t.input_schema);
        let _ = call_tool(&server, t.name, args).await;
    }
    // Variants of composite tools.
    let dir = app.downloads();
    let key = dir.join("AuthKey.p8");
    std::fs::write(
        &key,
        "-----BEGIN PRIVATE KEY-----\nAAA\n-----END PRIVATE KEY-----",
    )
    .unwrap();
    let csv = dir.join("k.csv");
    std::fs::write(&csv, "keyword\nhabit\n").unwrap();
    let k = key.to_string_lossy().to_string();
    let extra = [
        ("save_folder", json!({"folderId":"f1","name":"Renamed"})),
        ("edit_keyword", json!({"keywordId":"k1","hidden":true})),
        (
            "optimizer_save_option",
            json!({"projectId":"1","optionId":"o1","listing":{"locale":"en-US","title":"T"}}),
        ),
        ("acknowledge_alerts", json!({"clear":true})),
        (
            "export_data",
            json!({"projectId":"1","what":"history","format":"csv","saveToFile":true}),
        ),
        (
            "export_data",
            json!({"projectId":"1","what":"optimizer","format":"json","saveToFile":true}),
        ),
        (
            "export_data",
            json!({"projectId":"1","what":"board","format":"csv","saveToFile":true}),
        ),
        (
            "import_file",
            json!({"path": csv.to_string_lossy(), "format":"csv"}),
        ),
        (
            "connect_console",
            json!({"store":"apple","keyId":"K","issuerId":"I","privateKeyPath":k}),
        ),
        (
            "connect_console",
            json!({"store":"google","accountId":"a1","packageNames":["com.a.b"]}),
        ),
        (
            "connect_popularity",
            json!({"source":"apple_ads","clientId":"c","teamId":"t","keyId":"k","privateKeyPath":k}),
        ),
    ];
    for (name, args) in extra {
        let (is_error, payload) = call_tool(&server, name, args).await;
        assert!(!is_error, "{name}: {payload}");
    }
    let seen: HashSet<String> = app
        .actions()
        .iter()
        .filter_map(|a| a["type"].as_str().map(str::to_string))
        .collect();
    let expected = [
        "project.add",
        "project.addAll",
        "project.brainstorm",
        "project.update",
        "project.markets",
        "project.move",
        "project.resync",
        "folder.create",
        "folder.rename",
        "folder.remove",
        "keyword.add",
        "keyword.update",
        "keyword.hide",
        "keyword.remove",
        "keyword.localize",
        "keywords.suggest",
        "term.set",
        "check.run",
        "popularity.fetch",
        "optimizer.start",
        "optimizer.more",
        "optimizer.option.add",
        "optimizer.option.update",
        "optimizer.option.remove",
        "optimizer.choose",
        "optimizer.localize",
        "optimizer.locale.set",
        "optimizer.locale.pick",
        "optimizer.clear",
        "optimizer.publish",
        "optimizer.export",
        "assistant.ask",
        "assistant.clear",
        "console.connect",
        "console.test",
        "console.disconnect",
        "console.packages.add",
        "popularity.connect",
        "popularity.disconnect",
        "job.stop",
        "settings.save",
        "import.file",
        "export.board",
        "export.history",
        "ai.select",
        "ai.test",
        "alerts.read",
        "alerts.clear",
        "reviews.sync",
        "reviews.draft",
    ];
    let missing: Vec<&str> = expected
        .iter()
        .copied()
        .filter(|t| !seen.contains(*t))
        .collect();
    assert!(missing.is_empty(), "actions without a tool: {missing:?}");
    let methods: HashSet<String> = app.methods().into_iter().collect();
    for cmd in [
        "get_snapshot",
        "get_board",
        "get_term",
        "get_optimizer",
        "list_markets",
        "search_store",
        "console_apps",
        "console_accounts",
        "ai_status",
        "license_status",
        "popularity_status",
        "next_version",
        "list_alerts",
        "get_assistant",
        "export_data",
        "get_reviews",
    ] {
        assert!(methods.contains(cmd), "command {cmd} never called");
    }
}

#[tokio::test]
async fn overview_is_compact_and_includes_license_and_alerts() {
    let app = FakeApp::start(default_reply).await;
    let (is_error, v) = call_tool(&app.server(), "get_overview", json!({})).await;
    assert!(!is_error, "{v}");
    assert_eq!(v["projects"][0]["id"], "1");
    assert_eq!(v["projects"][0]["keywords"], 3);
    assert!(
        v["projects"][0].get("folderId").is_none(),
        "nulls are pruned"
    );
    assert_eq!(v["license"]["state"], "active");
    assert_eq!(v["unreadRankAlerts"], 1);
}

#[tokio::test]
async fn board_is_summarised_with_estimate_and_popularity_apart() {
    let app = FakeApp::start(default_reply).await;
    let (is_error, v) = call_tool(
        &app.server(),
        "get_project_board",
        json!({"projectId":"1","markets":["US"]}),
    )
    .await;
    assert!(!is_error, "{v}");
    let cell = &v["sections"]["listing"][0]["cells"]["us"];
    assert_eq!(cell["rank"], 4);
    assert_eq!(cell["estVol"], 61);
    assert!(cell.get("pop").is_none());
    assert!(
        cell.get("history").is_none(),
        "history is only in term detail"
    );
    assert!(v["legend"].as_str().unwrap().contains("REAL"));
}

#[tokio::test]
async fn actions_pass_through_with_renames_and_lowercased_markets() {
    let app = FakeApp::start(default_reply).await;
    let server = app.server();
    let (e, v) = call_tool(
        &server,
        "update_project",
        json!({"projectId":"3","name":"New"}),
    )
    .await;
    assert!(!e, "{v}");
    let (e, _) = call_tool(
        &server,
        "set_project_markets",
        json!({"projectId":"3","markets":["US","De"]}),
    )
    .await;
    assert!(!e);
    let (e, v) = call_tool(
        &server,
        "run_rank_check",
        json!({"projectId":"3","force":true}),
    )
    .await;
    assert!(!e);
    assert_eq!(v["queued"], true);
    let actions = app.actions();
    assert_eq!(
        actions[0],
        json!({"type":"project.update","id":"3","name":"New"})
    );
    assert_eq!(
        actions[1],
        json!({"type":"project.markets","id":"3","markets":["us","de"]})
    );
    assert_eq!(
        actions[2],
        json!({"type":"check.run","projectId":"3","force":true})
    );
}

#[tokio::test]
async fn api_errors_become_tool_errors_with_hints() {
    let app = FakeApp::start(|m, p| match m {
        "get_board" => api_error(
            "not_found",
            &format!("project {} not found", p["projectId"].as_str().unwrap()),
            None,
        ),
        "dispatch" => api_error(
            "ai_not_configured",
            "No AI provider is set up.",
            Some("setup_ai"),
        ),
        _ => default_reply(m, p),
    })
    .await;
    let server = app.server();
    let (is_error, v) = call_tool(&server, "get_project_board", json!({"projectId":"9"})).await;
    assert!(is_error);
    assert_eq!(v["error"]["code"], "not_found");
    assert_eq!(v["error"]["message"], "project 9 not found");
    assert!(v["hint"].as_str().unwrap().contains("get_overview"));
    let (is_error, v) = call_tool(&server, "suggest_keywords", json!({"projectId":"1"})).await;
    assert!(is_error);
    assert_eq!(v["error"]["action"], "setup_ai");
}

#[tokio::test]
async fn license_required_is_reported_once_and_not_retried() {
    let app = FakeApp::start(|_, _| {
        api_error(
            "license_required",
            "Activate ASOMaster to continue.",
            Some("activate"),
        )
    })
    .await;
    let server = app.server();
    let (is_error, v) = call_tool(
        &server,
        "add_keywords",
        json!({"projectId":"1","keywords":["a","b","c"]}),
    )
    .await;
    assert!(is_error);
    assert_eq!(v["error"]["code"], "license_required");
    assert_eq!(v["error"]["action"], "activate");
    assert!(v["hint"].as_str().unwrap().contains("license"));
    assert_eq!(
        app.actions().len(),
        1,
        "stops after the first license error"
    );
}

#[tokio::test]
async fn batch_keyword_add_reports_partial_failures() {
    let app = FakeApp::start(|m, p| {
        if m == "dispatch" && p["action"]["label"] == "dup" {
            return api_error("conflict", "keyword already exists", None);
        }
        default_reply(m, p)
    })
    .await;
    let (is_error, v) = call_tool(
        &app.server(),
        "add_keywords",
        json!({"projectId":"1","keywords":["habit","dup","streak"],"section":"listing"}),
    )
    .await;
    assert!(!is_error);
    assert_eq!(v["added"], json!(["habit", "streak"]));
    assert_eq!(v["failed"][0]["item"], "dup");
    assert_eq!(app.actions()[0]["section"], "listing");
}

#[tokio::test]
async fn timeouts_are_reported() {
    let app = FakeApp::start(|m, p| match m {
        "get_snapshot" => Reply::Silent,
        _ => default_reply(m, p),
    })
    .await;
    let mut cfg = app.config();
    cfg.call_timeout = Duration::from_millis(300);
    let (is_error, v) = call_tool(&app.server_with(cfg), "get_overview", json!({})).await;
    assert!(is_error);
    assert_eq!(v["error"]["code"], "timeout");
}

#[tokio::test]
async fn invalid_arguments_are_tool_errors_and_never_reach_the_app() {
    let app = FakeApp::start(default_reply).await;
    let server = app.server();
    let (is_error, v) = call_tool(&server, "get_project_board", json!({})).await;
    assert!(is_error);
    assert_eq!(v["error"]["code"], "invalid_input");
    let (is_error, _) = call_tool(
        &server,
        "publish_listing",
        json!({"projectId":"1","ownerApproved":false,"approval":"no"}),
    )
    .await;
    assert!(is_error);
    let (is_error, _) = call_tool(
        &server,
        "set_market_phrase",
        json!({"keywordId":"k","market":"usa","phrase":"x"}),
    )
    .await;
    assert!(is_error);
    assert!(app.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn publish_dispatches_only_with_approval() {
    let app = FakeApp::start(default_reply).await;
    let (is_error, v) = call_tool(
        &app.server(),
        "publish_listing",
        json!({"projectId":"1","ownerApproved":true,"approval":"Owner: yes, publish v1.2","versionString":"1.2.0"}),
    )
    .await;
    assert!(!is_error, "{v}");
    let a = &app.actions()[0];
    assert_eq!(
        a,
        &json!({"type":"optimizer.publish","projectId":"1","versionString":"1.2.0"})
    );
}

#[tokio::test]
async fn a_replacement_name_is_uploaded_only_as_the_owner_chose_it() {
    let app = FakeApp::start(default_reply).await;
    let (is_error, _) = call_tool(
        &app.server(),
        "upload_app_name",
        json!({"projectId":"1","locale":"de-DE","name":"Neu","ownerApproved":false,"approval":"no"}),
    )
    .await;
    assert!(is_error);
    assert!(app.calls.lock().unwrap().is_empty());
    let (is_error, v) = call_tool(
        &app.server(),
        "upload_app_name",
        json!({"projectId":"1","locale":"de-DE","name":"Neu Pro","ownerApproved":true,"approval":"Owner: use Neu Pro"}),
    )
    .await;
    assert!(!is_error, "{v}");
    assert_eq!(
        &app.actions()[0],
        &json!({"type":"optimizer.name.upload","projectId":"1","locale":"de-DE","name":"Neu Pro"})
    );
}

#[tokio::test]
async fn secrets_are_sent_to_the_app_but_never_returned() {
    let app = FakeApp::start(default_reply).await;
    let dir = app.downloads();
    let key = dir.join("AuthKey_ABC.p8");
    std::fs::write(&key, "TOPSECRETPEM").unwrap();
    let req = json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"connect_console","arguments":{
        "store":"apple","keyId":"ABC","issuerId":"ISS","privateKeyPath": key.to_string_lossy()}}});
    let resp = app.server().handle_value(req).await.unwrap();
    assert!(!resp.to_string().contains("TOPSECRETPEM"));
    let a = &app.actions()[0];
    assert_eq!(a["credentials"]["privateKeyPem"], "TOPSECRETPEM");
    assert_eq!(a["credentials"]["store"], "apple");
}

#[tokio::test]
async fn connect_console_with_an_account_id_adds_packages_without_a_key() {
    let app = FakeApp::start(default_reply).await;
    let (is_error, v) = call_tool(
        &app.server(),
        "connect_console",
        json!({"store":"google","accountId":"7","packageNames":["com.cressu.bombrun","com.cressu.other"]}),
    )
    .await;
    assert!(!is_error, "{v}");
    assert_eq!(v["added"], json!(["com.cressu.bombrun", "com.cressu.other"]));
    let actions = app.actions();
    assert_eq!(
        actions,
        vec![
            json!({"type":"console.packages.add","accountId":"7","packageName":"com.cressu.bombrun"}),
            json!({"type":"console.packages.add","accountId":"7","packageName":"com.cressu.other"}),
        ]
    );
    // A key, the apple store or no packages with accountId are refused before the app is asked.
    for args in [
        json!({"store":"google","accountId":"7","packageNames":["com.a.b"],"serviceAccountJson":"{}"}),
        json!({"store":"apple","accountId":"7","packageNames":["com.a.b"]}),
        json!({"store":"google","accountId":"7"}),
    ] {
        let (is_error, v) = call_tool(&app.server(), "connect_console", args.clone()).await;
        assert!(is_error, "{args}");
        assert_eq!(v["error"]["code"], "invalid_input", "{args}");
    }
    assert_eq!(app.actions().len(), 2);
}

#[tokio::test]
async fn a_missing_play_package_error_keeps_its_fix_for_the_agent() {
    let app = FakeApp::start(|m, p| match m {
        "dispatch" => {
            let mut data = json!({"code":"console_not_connected","message":"The connected Play Console account x doesn't include com.cressu.bombrun.","action":"add_package"});
            data["details"] = json!({"accountId":"7","account":"x","packageName":"com.cressu.bombrun"});
            Reply::Err(json!({"code": -32000, "message": "missing", "data": data}))
        }
        _ => default_reply(m, p),
    })
    .await;
    let (is_error, v) = call_tool(&app.server(), "resync_project", json!({"projectId":"1"})).await;
    assert!(is_error);
    assert_eq!(v["error"]["action"], "add_package");
    assert_eq!(v["error"]["details"]["accountId"], "7");
    assert_eq!(v["error"]["details"]["packageName"], "com.cressu.bombrun");
    assert!(v["hint"].as_str().unwrap().contains("accountId"), "{v}");
}

#[tokio::test]
async fn wait_returns_when_a_job_finishes() {
    let finished = Arc::new(AtomicBool::new(false));
    let f = finished.clone();
    let app = FakeApp::start(move |m, p| match m {
        "get_snapshot" if f.load(Ordering::SeqCst) => {
            Reply::Ok(snapshot(json!([job("j1", "done")])))
        }
        "get_snapshot" => Reply::Ok(snapshot(json!([job("j1", "running")]))),
        _ => default_reply(m, p),
    })
    .await;
    let app = Arc::new(app);
    let pusher = app.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(400)).await;
        finished.store(true, Ordering::SeqCst);
        pusher.push_event("jobs://progress", job("j1", "done"));
    });
    let started = std::time::Instant::now();
    let (is_error, v) = call_tool(
        &app.server(),
        "wait_for_jobs",
        json!({"timeoutSeconds": 10}),
    )
    .await;
    assert!(!is_error, "{v}");
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "returned on the event, not the timeout"
    );
    assert_eq!(v["idle"], true);
    assert_eq!(v["timedOut"], false);
    assert_eq!(v["finished"][0]["id"], "j1");
    assert_eq!(v["finished"][0]["status"], "done");
}

#[tokio::test]
async fn wait_returns_immediately_when_idle_and_times_out_when_busy() {
    let app = FakeApp::start(default_reply).await;
    let (_, v) = call_tool(&app.server(), "wait_for_jobs", json!({})).await;
    assert_eq!(v["idle"], true);
    assert_eq!(v["timedOut"], false);

    let busy = FakeApp::start(|m, p| match m {
        "get_snapshot" => Reply::Ok(snapshot(json!([job("j2", "running")]))),
        _ => default_reply(m, p),
    })
    .await;
    let (_, v) = call_tool(
        &busy.server(),
        "wait_for_jobs",
        json!({"timeoutSeconds": 1, "jobId": "j2"}),
    )
    .await;
    assert_eq!(v["timedOut"], true);
    assert_eq!(v["idle"], false);
    assert_eq!(v["job"]["progress"], "1/4");
}

#[tokio::test]
async fn export_returns_content_and_import_preview_reads_the_file() {
    let app = FakeApp::start(default_reply).await;
    let server = app.server();
    let (e, v) = call_tool(
        &server,
        "export_data",
        json!({"projectId":"1","what":"board","format":"csv"}),
    )
    .await;
    assert!(!e);
    assert_eq!(v["content"], "a,b\n1,2\n");
    let dir = app.downloads();
    let f = dir.join("k.csv");
    std::fs::write(&f, "keyword\nhabit\n").unwrap();
    let (e, v) = call_tool(
        &server,
        "import_file",
        json!({"path": f.to_string_lossy(), "format":"csv","previewOnly":true}),
    )
    .await;
    assert!(!e, "{v}");
    assert_eq!(v["preview"]["keywords"][0], "x");
    let calls = app.calls.lock().unwrap();
    let preview = calls.iter().find(|(m, _)| m == "import_preview").unwrap();
    assert_eq!(preview.1["content"], "keyword\nhabit\n");
    assert_eq!(preview.1["fileName"], "k.csv");
}

#[tokio::test]
async fn remove_project_needs_the_projects_name() {
    let app = FakeApp::start(default_reply).await;
    let server = app.server();
    let (is_error, payload) = call_tool(
        &server,
        "remove_project",
        json!({"projectId": "1", "confirmName": "FitFoodie+"}),
    )
    .await;
    assert!(is_error, "{payload}");
    assert_eq!(payload["error"]["code"], "invalid_input");
    assert!(app.actions().is_empty(), "nothing dispatched on a name mismatch");

    let (is_error, payload) = call_tool(
        &server,
        "remove_project",
        json!({"projectId": "1", "confirmName": " habit app "}),
    )
    .await;
    assert!(!is_error, "{payload}");
    assert_eq!(app.actions(), vec![json!({"type": "project.remove", "id": "1"})]);
}

#[tokio::test]
async fn search_results_say_which_apps_are_already_tracked() {
    let app = FakeApp::start(|m, p| match m {
        "search_store" => Reply::Ok(json!([
            {"store": p["store"], "id":"123","name":"Habit App","ratingCount":10},
            {"store": p["store"], "id":"456","name":"Other","ratingCount":5}
        ])),
        _ => default_reply(m, p),
    })
    .await;
    let (is_error, payload) = call_tool(
        &app.server(),
        "search_store",
        json!({"store":"apple","term":"habit","country":"us"}),
    )
    .await;
    assert!(!is_error, "{payload}");
    assert_eq!(payload["results"][0]["trackedAs"]["projectId"], "1");
    assert!(payload["results"][1].get("trackedAs").is_none());
}

#[tokio::test]
async fn search_passes_the_requested_limit_to_the_app() {
    let app = FakeApp::start(|m, p| match m {
        // Echo the limit back so the test sees what the bridge sent.
        "search_store" => Reply::Ok(json!([
            {"store": p["store"], "id": p["limit"].to_string(), "name":"Echo","ratingCount":1}
        ])),
        _ => default_reply(m, p),
    })
    .await;
    let (is_error, payload) = call_tool(
        &app.server(),
        "search_store",
        json!({"store":"apple","term":"habit","country":"us","limit":50}),
    )
    .await;
    assert!(!is_error, "{payload}");
    assert_eq!(payload["results"][0]["id"], "50");
}

#[tokio::test]
async fn publish_preview_reads_from_the_app_and_never_dispatches() {
    let app = FakeApp::start(|m, p| match m {
        "publish_preview" => Reply::Ok(json!({
            "store": "apple", "source": "console", "readAt": "2026-10-29T00:00:00Z",
            "rows": [{"locale": "en-US", "isNew": false, "changes": [{"field": "subtitle", "from": "Old", "to": "New"}]}],
            "unchanged": [], "projectId": p["projectId"]
        })),
        _ => default_reply(m, p),
    })
    .await;
    let (is_error, payload) = call_tool(&app.server(), "get_publish_preview", json!({"projectId": "1"})).await;
    assert!(!is_error, "{payload}");
    assert_eq!(payload["rows"][0]["changes"][0]["to"], "New");
    assert_eq!(payload["projectId"], "1");
    assert!(app.actions().is_empty(), "a preview never dispatches");
}

#[tokio::test]
async fn add_app_passes_main_listing_language_and_ai() {
    let app = FakeApp::start(default_reply).await;
    let (is_error, payload) = call_tool(
        &app.server(),
        "add_app",
        json!({"store":"apple","app":"123","kind":"competitor","mainLocale":"de-DE","ai":"codex"}),
    )
    .await;
    assert!(!is_error, "{payload}");
    let action = &app.actions()[0];
    assert_eq!(action["type"], "project.add");
    assert_eq!(action["appIdOrUrl"], "123");
    assert_eq!(action["mainLocale"], "de-DE");
    assert_eq!(action["ai"], "codex");
}

#[tokio::test]
async fn optimizer_set_links_dispatches_the_links_action() {
    let app = FakeApp::start(default_reply).await;
    let (is_error, payload) = call_tool(
        &app.server(),
        "optimizer_set_links",
        json!({"projectId":"1","whatsNew":"Fixes","marketingUrl":"https://example.com","publish":true}),
    )
    .await;
    assert!(!is_error, "{payload}");
    assert_eq!(
        app.actions()[0],
        json!({"type":"optimizer.links.set","projectId":"1","whatsNew":"Fixes","marketingUrl":"https://example.com","publish":true})
    );
}

#[tokio::test]
async fn fastlane_export_is_saved_to_a_file_and_only_for_the_listing() {
    let app = FakeApp::start(default_reply).await;
    let server = app.server();
    let (is_error, payload) = call_tool(&server, "export_data", json!({"projectId":"1","what":"optimizer","format":"fastlane"})).await;
    assert!(!is_error, "{payload}");
    assert_eq!(app.actions()[0], json!({"type":"optimizer.export","projectId":"1","format":"fastlane"}));
    let (is_error, _) = call_tool(&server, "export_data", json!({"projectId":"1","what":"board","format":"fastlane"})).await;
    assert!(is_error, "fastlane is only for the listing");
}

#[tokio::test]
async fn app_facts_reads_from_the_app() {
    let app = FakeApp::start(|m, p| match m {
        "app_facts" => Reply::Ok(json!([{"id": p["ids"][0], "ratingCount": 10, "ratingsPerDay": 0.5}])),
        _ => default_reply(m, p),
    })
    .await;
    let (is_error, payload) = call_tool(&app.server(), "get_app_facts", json!({"store":"apple","ids":["123"],"country":"us"})).await;
    assert!(!is_error, "{payload}");
    assert_eq!(payload[0]["id"], "123");
    assert!(app.actions().is_empty());
}

#[tokio::test]
async fn rival_changes_come_from_the_app() {
    let app = FakeApp::start(|m, p| match m {
        "list_listing_changes" => Reply::Ok(json!([{"id":"1","appName":"Calm","field":"subtitle","from":"A","to":"B"}])),
        _ => default_reply(m, p),
    })
    .await;
    let (is_error, payload) = call_tool(&app.server(), "list_rival_changes", json!({})).await;
    assert!(!is_error, "{payload}");
    assert_eq!(payload[0]["to"], "B");
}

#[tokio::test]
async fn change_impact_reads_from_the_app_with_the_window_asked_for() {
    let app = FakeApp::start(|m, p| match m {
        "get_change_impact" => Reply::Ok(json!({"projectId": p["projectId"], "days": p["days"], "events": [
            {"id": "2", "kind": "manual", "label": "New screenshots", "verdict": "too_early", "fields": [], "overlaps": ["1"],
             "markets": [{"country": "us", "verdict": "too_early", "ranks": {"before": {"top10": 2}, "after": {"top10": 4}}, "traffic": {}, "ratings": {}}]},
            {"id": "1", "kind": "publish", "label": "Listing published", "verdict": "improved", "fields": [{}], "overlaps": ["2"], "markets": []}
        ]})),
        _ => default_reply(m, p),
    })
    .await;
    let (is_error, payload) = call_tool(&app.server(), "get_change_impact", json!({"projectId": "4", "days": 28})).await;
    assert!(!is_error, "{payload}");
    assert_eq!(payload["days"], 28);
    assert_eq!(payload["events"][0]["markets"][0]["top10"], json!([2, 4]));
    assert_eq!(payload["events"][1]["fields"], 1, "older changes are one line");
    let (is_error, _) = call_tool(&app.server(), "get_change_impact", json!({"projectId": "4", "days": 10})).await;
    assert!(is_error, "only 7, 14 or 28 days");
    assert!(app.actions().is_empty(), "read-only");
}

#[tokio::test]
async fn get_audit_reads_from_the_app() {
    let app = FakeApp::start(|m, p| match m {
        "get_audit" => Reply::Ok(json!({"total": 61, "projectId": p["projectId"], "mainCountry": "us", "countries": [
            {"country": "us", "top10": [], "rankings": {"score": 50}},
            {"country": "de", "top10": [], "rankings": {"score": 30}}
        ]})),
        _ => default_reply(m, p),
    })
    .await;
    let (is_error, payload) = call_tool(&app.server(), "get_audit", json!({"projectId":"4"})).await;
    assert!(!is_error, "{payload}");
    assert_eq!(payload["total"], 61);
    assert_eq!(payload["projectId"], "4");
    assert!(payload["countries"][0]["top10"].is_array(), "main country in full");
    assert_eq!(payload["countries"][1]["rankings"], 30, "others compact");
}

#[tokio::test]
async fn get_search_terms_reads_the_cached_report_for_one_market() {
    let app = FakeApp::start(|m, p| match m {
        "get_search_terms" => Reply::Ok(json!({"state": "ready", "projectId": p["projectId"], "days": p["days"], "markets": [
            {"country": "us", "lowVolume": {"impressions": 40}, "terms": [
                {"term": "white noise", "impressions": 900, "avoided": false},
                {"term": "free sleep", "impressions": 100, "avoided": true}
            ]},
            {"country": "gb", "terms": [{"term": "rain", "impressions": 10, "avoided": false}]}
        ]})),
        _ => default_reply(m, p),
    })
    .await;
    let (is_error, payload) =
        call_tool(&app.server(), "get_search_terms", json!({"projectId": "4", "days": 90, "market": "us"})).await;
    assert!(!is_error, "{payload}");
    assert_eq!((payload["projectId"].clone(), payload["days"].clone()), (json!("4"), json!(90)));
    let markets = payload["markets"].as_array().unwrap();
    assert_eq!(markets.len(), 1);
    assert_eq!(markets[0]["terms"].as_array().unwrap().len(), 1, "don't-target terms left out");
    assert_eq!(markets[0]["avoidedCount"], 1);
    assert_eq!(markets[0]["lowVolume"]["impressions"], 40);
    assert!(app.actions().is_empty(), "read-only");
}

#[tokio::test]
async fn add_keywords_can_track_a_phrase_in_one_market() {
    let app = FakeApp::start(default_reply).await;
    let (is_error, payload) = call_tool(
        &app.server(),
        "add_keywords",
        json!({"projectId": "1", "keywords": ["regengeräusche"], "localize": true, "market": "DE"}),
    )
    .await;
    assert!(!is_error, "{payload}");
    assert_eq!(app.actions()[0]["market"], "de");
    let (_, _) = call_tool(&app.server(), "add_keywords", json!({"projectId": "1", "keywords": ["x"]})).await;
    assert!(app.actions()[1].get("market").is_none());
}

#[tokio::test]
async fn run_audit_and_dismiss_dispatch_audit_actions() {
    let app = FakeApp::start(default_reply).await;
    let (is_error, payload) = call_tool(
        &app.server(),
        "run_audit",
        json!({"projectId": "4", "options": {"countries": ["DE"], "recheck": true, "note": "focus on sleep"}}),
    )
    .await;
    assert!(!is_error, "{payload}");
    assert_eq!(payload["queued"], true);
    let (is_error, payload) = call_tool(&app.server(), "dismiss_audit_competitor", json!({"projectId": "4", "appId": "42", "restore": true})).await;
    assert!(!is_error, "{payload}");
    assert_eq!(payload["action"], "audit.restore");
    let sent = app.actions();
    assert_eq!(sent[0]["type"], "audit.run");
    assert_eq!(sent[0]["options"]["recheck"], true);
    assert_eq!(sent[1], json!({"type": "audit.restore", "projectId": "4", "appId": "42"}));
}

#[tokio::test]
async fn reviews_are_read_and_drafted_but_never_sent() {
    let app = FakeApp::start(|m, p| match m {
        "get_reviews" => Reply::Ok(json!({"projectId": p["projectId"], "query": p["query"], "reviews": []})),
        _ => default_reply(m, p),
    })
    .await;
    let server = app.server();
    let (is_error, payload) = call_tool(&server, "get_reviews", json!({"projectId": "4", "stars": [1], "market": "DE"})).await;
    assert!(!is_error, "{payload}");
    assert_eq!(payload["query"], json!({"days": 90, "limit": 50, "stars": [1], "market": "de"}));
    let (is_error, payload) = call_tool(&server, "draft_review_reply", json!({"projectId": "4", "reviewId": "r1", "directions": "mention the 2.1 fix"})).await;
    assert!(!is_error, "{payload}");
    assert_eq!(payload["queued"], true);
    assert_eq!(app.actions()[0], json!({"type": "reviews.draft", "projectId": "4", "reviewId": "r1", "directions": "mention the 2.1 fix"}));
    for name in ["send_review_reply", "reply_to_review", "delete_review_reply"] {
        assert!(registry().get(name).is_none(), "{name} must not exist");
    }
}

#[tokio::test]
async fn link_released_app_dispatches_project_link() {
    let app = FakeApp::start(default_reply).await;
    let (is_error, payload) = call_tool(&app.server(), "link_released_app", json!({"projectId":"7","app":"123","kind":"mine"})).await;
    assert!(!is_error, "{payload}");
    assert_eq!(app.actions()[0], json!({"type":"project.link","id":"7","appIdOrUrl":"123","kind":"mine"}));
}

#[tokio::test]
async fn pricing_tools_convert_prices_and_publish_only_with_approval() {
    let app = FakeApp::start(|m, p| match m {
        "get_pricing" => Reply::Ok(json!({
            "status": "ok", "store": "apple", "baseCountry": "us", "plan": {"baseCents": {}},
            "products": [{"id": "sub:1", "name": "Pro", "baseCents": 999, "rows": [
                {"country": "in", "currentCents": 89_900, "proposedCents": 29_900, "changed": true}
            ]}], "projectId": p["projectId"]
        })),
        _ => default_reply(m, p),
    })
    .await;
    let server = app.server();
    let (is_error, v) = call_tool(&server, "get_pricing", json!({"projectId": "1", "country": "IN"})).await;
    assert!(!is_error, "{v}");
    assert_eq!(v["products"][0]["rows"][0]["proposed"], 299.0);
    assert!(app.actions().is_empty(), "reading never dispatches");

    let (is_error, v) = call_tool(
        &server,
        "plan_pricing",
        json!({"projectId": "1", "select": ["sub:1"], "override": {"productId": "sub:1", "country": "de", "price": 4.49}}),
    )
    .await;
    assert!(!is_error, "{v}");
    assert_eq!(
        app.actions()[0],
        json!({"type": "pricing.plan", "projectId": "1", "patch": {"selected": ["sub:1"], "overridePrice": {"productId": "sub:1", "country": "de", "cents": 449}}})
    );

    let (is_error, _) = call_tool(&server, "publish_pricing", json!({"projectId": "1", "ownerApproved": false, "approval": "no"})).await;
    assert!(is_error);
    assert_eq!(app.actions().len(), 1, "no publish without ownerApproved");
    let (is_error, v) = call_tool(
        &server,
        "publish_pricing",
        json!({"projectId": "1", "ownerApproved": true, "approval": "Owner: publish the new prices"}),
    )
    .await;
    assert!(!is_error, "{v}");
    assert_eq!(
        app.actions()[1],
        json!({"type": "pricing.publish", "projectId": "1", "approval": "Owner: publish the new prices"})
    );
}
