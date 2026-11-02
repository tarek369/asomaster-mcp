//! Background jobs: stop one, clear finished ones, or long-poll until job
//! state changes.

use std::collections::HashSet;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::sync::broadcast::error::RecvError;
use tokio::time::Instant;

use super::schema as s;
use super::{Tool, DESTRUCTIVE, READ, SET};
use crate::error::BridgeError;
use crate::ipc::{Bridge, IpcClient};
use crate::summarize::{active_jobs, is_active, is_terminal, job};

/// Hard cap so the agent's tool call never hits client-side timeouts.
pub const MAX_WAIT_SECS: u64 = 50;
const DEFAULT_WAIT_SECS: u64 = 45;
/// Re-read the snapshot this often in case an event was missed.
const POLL_EVERY: Duration = Duration::from_secs(5);
const SUBSCRIBE_TIMEOUT: Duration = Duration::from_secs(10);

pub fn tools() -> Vec<Tool> {
    vec![
        Tool::action(
            "stop_job",
            "Stop a job",
            "Stop a queued or running background job (rank check, AI generation, import, ...). Work already done is kept.",
            s::object(vec![("jobId", s::id("Job id (get_overview / wait_for_jobs)."))], &["jobId"]),
            SET,
            "job.stop",
            &[],
            false,
        ),
        Tool::action(
            "clear_jobs",
            "Clear finished jobs",
            "Remove finished, failed and stopped jobs from the job list. Queued and running jobs are kept; their results (rankings, options, ...) are not affected.",
            s::empty(),
            DESTRUCTIVE,
            "jobs.clear",
            &[],
            false,
        ),
        Tool::custom(
            "wait_for_jobs",
            "Wait for background work",
            "Block until background work changes state: returns as soon as a job finishes (done / failed / stopped), or when nothing is queued or running, or after timeoutSeconds (max 50). With jobId, waits for that job only; with projectId, for that project's jobs. Call it repeatedly after any tool that queued work until idle is true, then read the results. Reports progress of jobs still running.",
            s::object(
                vec![
                    ("jobId", s::id("Wait for this job only.")),
                    ("projectId", s::id("Only this project's jobs (a finished fetch can start brainstorming, then a check).")),
                    ("timeoutSeconds", s::integer("Max seconds to wait (default 45, max 50).", 1, MAX_WAIT_SECS as i64)),
                ],
                &[],
            ),
            READ,
        ),
    ]
}

pub async fn run(bridge: &Bridge, name: &str, args: &Value) -> Option<Result<Value, BridgeError>> {
    match name {
        "wait_for_jobs" => Some(wait(bridge, args).await),
        _ => None,
    }
}

fn jobs_of(snapshot: &Value) -> Vec<Value> {
    snapshot["jobs"].as_array().cloned().unwrap_or_default()
}

fn id_of(j: &Value) -> &str {
    j["id"].as_str().unwrap_or_default()
}

/// Has the awaited condition been reached in this job list?
fn settled(jobs: &[Value], job_id: Option<&str>, baseline: &HashSet<String>) -> bool {
    match job_id {
        Some(id) => jobs.iter().find(|j| id_of(j) == id).is_none_or(is_terminal),
        None => {
            !jobs.iter().any(is_active)
                || jobs
                    .iter()
                    .any(|j| is_terminal(j) && !baseline.contains(id_of(j)))
        }
    }
}

/// Keep only `project_id`'s jobs (all when `None`).
fn for_project(jobs: Vec<Value>, project_id: Option<&str>) -> Vec<Value> {
    jobs.into_iter()
        .filter(|j| project_id.is_none_or(|p| j["projectId"].as_str() == Some(p)))
        .collect()
}

async fn snapshot_jobs(
    client: &IpcClient,
    timeout: Duration,
    project_id: Option<&str>,
) -> Result<Vec<Value>, BridgeError> {
    let snap = client.request("get_snapshot", json!({}), timeout).await?;
    Ok(for_project(jobs_of(&snap), project_id))
}

pub async fn wait(bridge: &Bridge, args: &Value) -> Result<Value, BridgeError> {
    let secs = args
        .get("timeoutSeconds")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_WAIT_SECS)
        .clamp(1, MAX_WAIT_SECS);
    let deadline = Instant::now() + Duration::from_secs(secs);
    let job_id = args.get("jobId").and_then(Value::as_str);
    let project_id = args.get("projectId").and_then(Value::as_str);
    let call_timeout = bridge.config().call_timeout;

    // Dedicated connection: subscribe first, then snapshot, so no change is missed.
    let client = bridge.connect().await?;
    let mut events = client.events();
    let _ = client
        .request("subscribe", json!({}), SUBSCRIBE_TIMEOUT)
        .await;
    let start = snapshot_jobs(&client, call_timeout, project_id).await?;
    let baseline: HashSet<String> = start
        .iter()
        .filter(|j| is_terminal(j))
        .map(|j| id_of(j).to_string())
        .collect();
    if let Some(id) = job_id {
        if !start.iter().any(|j| id_of(j) == id) {
            return Err(BridgeError::Invalid(format!(
                "no job with id {id} (finished jobs may have been pruned; see get_overview)"
            )));
        }
    }

    let mut timed_out = false;
    if !settled(&start, job_id, &baseline) {
        let mut next_poll = Instant::now() + POLL_EVERY;
        loop {
            tokio::select! {
                ev = events.recv() => match ev {
                    Ok(e) if e.channel == "jobs://progress" && is_terminal(&e.payload)
                        && !baseline.contains(id_of(&e.payload))
                        && job_id.is_none_or(|id| id == id_of(&e.payload))
                        && project_id.is_none_or(|p| e.payload["projectId"].as_str() == Some(p)) => break,
                    Ok(_) | Err(RecvError::Lagged(_)) => {}
                    Err(RecvError::Closed) => return Err(BridgeError::Disconnected),
                },
                _ = tokio::time::sleep_until(next_poll.min(deadline)) => {
                    if Instant::now() >= deadline {
                        timed_out = true;
                        break;
                    }
                    next_poll = Instant::now() + POLL_EVERY;
                    if settled(&snapshot_jobs(&client, call_timeout, project_id).await?, job_id, &baseline) {
                        break;
                    }
                }
            }
        }
    }

    let end = snapshot_jobs(&client, call_timeout, project_id).await?;
    let finished: Vec<Value> = end
        .iter()
        .filter(|j| is_terminal(j) && !baseline.contains(id_of(j)))
        .filter(|j| job_id.is_none_or(|id| id == id_of(j)))
        .map(job)
        .collect();
    let all = Value::Array(end.clone());
    let active = active_jobs(&all, None);
    let idle = active.is_empty();
    let failed = finished.iter().any(|j| j["status"] == "failed");
    let next = match (idle, failed) {
        (_, true) => "A job failed; read its error and tell the owner or fix the cause before retrying.",
        (true, false) => "Nothing is running. Read the results (get_project_board / get_optimizer / get_assistant_thread).",
        (false, false) => "Work is still running; call wait_for_jobs again.",
    };
    let mut out = json!({
        "idle": idle,
        "timedOut": timed_out,
        "finished": finished,
        "activeJobs": active,
        "next": next,
    });
    if let Some(id) = job_id {
        if let Some(j) = end.iter().find(|j| id_of(j) == id) {
            out["job"] = job(j);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clear_jobs_dispatches_jobs_clear() {
        let t = tools().into_iter().find(|t| t.name == "clear_jobs").expect("clear_jobs");
        assert!(matches!(t.exec, super::super::Exec::Action { ty: "jobs.clear", .. }));
        assert!(t.annotations.destructive);
    }

    #[test]
    fn project_filter_keeps_only_that_projects_jobs() {
        let jobs = vec![
            json!({"id":"a","projectId":"1","status":"running"}),
            json!({"id":"b","projectId":"2","status":"running"}),
            json!({"id":"c","status":"running"}),
        ];
        let mine = for_project(jobs.clone(), Some("1"));
        assert_eq!(mine.len(), 1);
        assert!(!settled(&mine, None, &HashSet::new()));
        assert!(settled(&for_project(jobs.clone(), Some("9")), None, &HashSet::new()));
        assert_eq!(for_project(jobs, None).len(), 3);
    }

    #[test]
    fn settles_on_idle_new_terminal_or_target() {
        let base: HashSet<String> = ["old".to_string()].into();
        let running = json!({"id":"a","status":"running"});
        let old = json!({"id":"old","status":"done"});
        let new_done = json!({"id":"b","status":"done"});
        assert!(!settled(&[running.clone(), old.clone()], None, &base));
        assert!(settled(std::slice::from_ref(&old), None, &base));
        assert!(settled(&[running.clone(), new_done], None, &base));
        assert!(!settled(std::slice::from_ref(&running), Some("a"), &base));
        assert!(settled(&[old], Some("old"), &base));
    }
}
