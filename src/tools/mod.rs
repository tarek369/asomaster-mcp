//! The curated MCP tool catalogue and its execution.
//!
//! Most mutating tools map 1:1 onto an engine action (`Exec::Action`): the
//! tool arguments are passed through (with a few renames) as the action's
//! camelCase fields. Reads and composite tools are `Exec::Custom` and are
//! implemented in the group modules.

mod audit;
mod connections;
mod jobs;
mod keywords;
mod optimizer;
mod pricing;
mod projects;
mod read;
mod screenshots;
pub mod schema;
mod system;

use std::collections::HashMap;
use std::sync::OnceLock;

use serde_json::{json, Map, Value};

use crate::error::BridgeError;
use crate::ipc::Bridge;
use crate::summarize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Annotations {
    pub read_only: bool,
    pub destructive: bool,
    pub idempotent: bool,
    pub open_world: bool,
}

/// Read local app state.
pub const READ: Annotations = Annotations {
    read_only: true,
    destructive: false,
    idempotent: true,
    open_world: false,
};
/// Read that queries the stores / providers over the network.
pub const READ_NET: Annotations = Annotations {
    open_world: true,
    ..READ
};
/// Additive / editing change.
pub const WRITE: Annotations = Annotations {
    read_only: false,
    destructive: false,
    idempotent: false,
    open_world: false,
};
/// Change that converges (setting a value twice = once).
pub const SET: Annotations = Annotations {
    idempotent: true,
    ..WRITE
};
/// Change that kicks off store / AI work over the network.
pub const WORK: Annotations = Annotations {
    open_world: true,
    ..WRITE
};
/// Deletes or overwrites data.
pub const DESTRUCTIVE: Annotations = Annotations {
    read_only: false,
    destructive: true,
    idempotent: true,
    open_world: false,
};

#[derive(Debug, Clone)]
pub enum Exec {
    /// Dispatch one action. `rename` maps tool argument → action field.
    /// `job` = the action queues background work.
    Action {
        ty: &'static str,
        rename: &'static [(&'static str, &'static str)],
        job: bool,
    },
    Custom,
}

#[derive(Debug, Clone)]
pub struct Tool {
    pub name: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub input_schema: Value,
    pub annotations: Annotations,
    pub exec: Exec,
}

impl Tool {
    pub fn custom(
        name: &'static str,
        title: &'static str,
        description: &'static str,
        input_schema: Value,
        annotations: Annotations,
    ) -> Self {
        Self {
            name,
            title,
            description,
            input_schema,
            annotations,
            exec: Exec::Custom,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn action(
        name: &'static str,
        title: &'static str,
        description: &'static str,
        input_schema: Value,
        annotations: Annotations,
        ty: &'static str,
        rename: &'static [(&'static str, &'static str)],
        job: bool,
    ) -> Self {
        Self {
            name,
            title,
            description,
            input_schema,
            annotations,
            exec: Exec::Action { ty, rename, job },
        }
    }

    /// MCP `tools/list` entry.
    pub fn to_mcp(&self) -> Value {
        json!({
            "name": self.name,
            "title": self.title,
            "description": self.description,
            "inputSchema": self.input_schema,
            "annotations": {
                "title": self.title,
                "readOnlyHint": self.annotations.read_only,
                "destructiveHint": self.annotations.destructive,
                "idempotentHint": self.annotations.idempotent,
                "openWorldHint": self.annotations.open_world,
            }
        })
    }
}

pub struct Registry {
    tools: Vec<Tool>,
    index: HashMap<&'static str, usize>,
    validators: Vec<jsonschema::Validator>,
}

impl Registry {
    fn build() -> Self {
        let tools: Vec<Tool> = [
            read::tools(),
            audit::tools(),
            projects::tools(),
            keywords::tools(),
            optimizer::tools(),
            pricing::tools(),
            screenshots::tools(),
            connections::tools(),
            system::tools(),
            jobs::tools(),
        ]
        .into_iter()
        .flatten()
        .collect();
        let index = tools.iter().enumerate().map(|(i, t)| (t.name, i)).collect();
        let validators = tools
            .iter()
            .map(|t| {
                jsonschema::validator_for(&t.input_schema)
                    .unwrap_or_else(|e| panic!("schema of {} does not compile: {e}", t.name))
            })
            .collect();
        Self {
            tools,
            index,
            validators,
        }
    }

    pub fn tools(&self) -> &[Tool] {
        &self.tools
    }

    pub fn get(&self, name: &str) -> Option<&Tool> {
        self.index.get(name).map(|&i| &self.tools[i])
    }

    /// Validate arguments against the tool's schema; returns readable errors.
    pub fn validate(&self, name: &str, args: &Value) -> Result<(), String> {
        let Some(&i) = self.index.get(name) else {
            return Err(format!("unknown tool \"{name}\""));
        };
        let errors: Vec<String> = self.validators[i]
            .iter_errors(args)
            .take(8)
            .map(|e| {
                let path = e.instance_path.to_string();
                if path.is_empty() {
                    e.to_string()
                } else {
                    format!("{path}: {e}")
                }
            })
            .collect();
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; "))
        }
    }
}

/// The process-wide catalogue (schemas compiled once).
pub fn registry() -> &'static Registry {
    static REG: OnceLock<Registry> = OnceLock::new();
    REG.get_or_init(Registry::build)
}

/// Run a validated tool call.
pub async fn run(bridge: &Bridge, tool: &Tool, args: Value) -> Result<Value, BridgeError> {
    let args = normalize(args);
    match &tool.exec {
        Exec::Action { ty, rename, job } => {
            let action = action_from_args(ty, rename, &args);
            bridge.dispatch(action).await?;
            if *job {
                queued(bridge, ty, args.get("projectId").and_then(Value::as_str)).await
            } else {
                Ok(json!({ "ok": true, "action": ty }))
            }
        }
        Exec::Custom => run_custom(bridge, tool.name, &args).await,
    }
}

async fn run_custom(bridge: &Bridge, name: &str, args: &Value) -> Result<Value, BridgeError> {
    if let Some(r) = read::run(bridge, name, args).await {
        return r;
    }
    if let Some(r) = audit::run(bridge, name, args).await {
        return r;
    }
    if let Some(r) = projects::run(bridge, name, args).await {
        return r;
    }
    if let Some(r) = keywords::run(bridge, name, args).await {
        return r;
    }
    if let Some(r) = optimizer::run(bridge, name, args).await {
        return r;
    }
    if let Some(r) = pricing::run(bridge, name, args).await {
        return r;
    }
    if let Some(r) = screenshots::run(bridge, name, args).await {
        return r;
    }
    if let Some(r) = connections::run(bridge, name, args).await {
        return r;
    }
    if let Some(r) = system::run(bridge, name, args).await {
        return r;
    }
    Err(BridgeError::Internal(format!("no handler for {name}")))
}

/// Build `{type, ...fields}` from tool arguments.
pub fn action_from_args(ty: &str, rename: &[(&str, &str)], args: &Value) -> Value {
    let mut obj = Map::new();
    if let Some(a) = args.as_object() {
        for (k, v) in a {
            let key = rename
                .iter()
                .find(|(from, _)| from == k)
                .map_or(k.as_str(), |(_, to)| *to);
            obj.insert(key.to_string(), v.clone());
        }
    }
    obj.insert("type".into(), Value::String(ty.to_string()));
    Value::Object(obj)
}

/// Lower-case market / country codes wherever they appear at the top level.
pub fn normalize(args: Value) -> Value {
    let Value::Object(map) = args else {
        return Value::Object(Map::new());
    };
    let out = map
        .into_iter()
        .map(|(k, v)| {
            let v = match (k.as_str(), v) {
                ("market" | "country", Value::String(s)) => Value::String(s.to_lowercase()),
                ("markets", Value::Array(a)) => Value::Array(
                    a.into_iter()
                        .map(|m| match m {
                            Value::String(s) => Value::String(s.to_lowercase()),
                            other => other,
                        })
                        .collect(),
                ),
                (_, v) => v,
            };
            (k, v)
        })
        .collect();
    Value::Object(out)
}

/// Result for an action that queued background work: the active jobs and
/// what to do next.
pub async fn queued(
    bridge: &Bridge,
    ty: &str,
    project_id: Option<&str>,
) -> Result<Value, BridgeError> {
    let jobs = bridge
        .call("get_snapshot", json!({}))
        .await
        .ok()
        .and_then(|s| s.get("jobs").cloned())
        .map(|j| summarize::active_jobs(&j, project_id))
        .unwrap_or_default();
    Ok(json!({
        "ok": true,
        "action": ty,
        "queued": true,
        "activeJobs": jobs,
        "next": "Work runs in the background. Call wait_for_jobs (repeat until idle), then read the result (get_overview / get_project_board / get_optimizer)."
    }))
}

/// Required string argument (already schema-validated).
pub fn arg_str<'a>(args: &'a Value, name: &str) -> Result<&'a str, BridgeError> {
    args.get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| BridgeError::Invalid(format!("missing \"{name}\"")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renames_and_tags_actions() {
        let a = action_from_args(
            "project.update",
            &[("projectId", "id")],
            &json!({"projectId": "3", "name": "X"}),
        );
        assert_eq!(a, json!({"type": "project.update", "id": "3", "name": "X"}));
    }

    #[test]
    fn normalizes_market_codes() {
        let v = normalize(json!({"market": "US", "markets": ["DE", "fr"], "label": "Hi"}));
        assert_eq!(
            v,
            json!({"market": "us", "markets": ["de", "fr"], "label": "Hi"})
        );
    }
}
