use crate::state::Task;
use herdr_bus::Message;
use priory_factory::factory::{Loaded, Workflow};
use serde::Deserialize;
use std::collections::BTreeMap;

const TASK_REQUESTED: &str = "factory.task.requested";

/// What the daemon does with one inbox entry.
#[derive(Debug, PartialEq)]
pub(crate) enum Decision {
    Create(NewTask),
    Keep(i64),
    Ignore,
}

#[derive(Debug, PartialEq, Deserialize)]
pub(crate) struct NewTask {
    pub workflow: String,
    #[serde(skip)]
    pub step: String,
    pub key: String,
    pub band: Option<String>,
    pub lane: Option<String>,
    pub trigger: String,
    pub source: String,
}

/// The resources a message names, which a task may own: a task request's key, a CLI request's
/// subject, or for a GitHub event its issue or pull request and its pull request's head branch.
pub(crate) fn resources(message: &Message) -> Vec<String> {
    let data = &message.data;
    if message.kind == TASK_REQUESTED {
        return data["key"]
            .as_str()
            .map(str::to_owned)
            .into_iter()
            .collect();
    }
    if message.kind == priory_intake::request::REQUESTED {
        return message.subject.clone().into_iter().collect();
    }
    let Some(repo) = data["repo"]
        .as_str()
        .filter(|_| message.kind.starts_with("github."))
    else {
        return Vec::new();
    };
    let number = data.get("issue").unwrap_or(&data["pull_request"]).as_u64();
    let head = data["head_branch"].as_str();
    number
        .map(|number| pull_request(repo, number))
        .into_iter()
        .chain(head.map(|head| branch(repo, head)))
        .collect()
}

/// An issue or pull request, as a resource: GitHub numbers both from one sequence per repository.
pub(crate) fn pull_request(repo: &str, number: u64) -> String {
    format!("github:{repo}#{number}")
}

pub(crate) fn branch(repo: &str, branch: &str) -> String {
    format!("github:{repo}@{branch}")
}

/// Whether `message` is a schedule's firing, which wakes or is coalesced into the task it finds.
pub(crate) fn tick(message: &Message) -> bool {
    message.kind == TASK_REQUESTED && message.data.get("scheduled").is_some()
}

/// Routes `message`, given the active task that owns its key, if any. A task request for a
/// workflow the factory lacks is an error, so that it waits unacked until the workflow is back.
pub(crate) fn route(
    message: &Message,
    owner: Option<&Task>,
    workflows: &BTreeMap<String, Loaded<Workflow>>,
) -> Result<Decision, String> {
    if let Some(task) = owner {
        return Ok(Decision::Keep(task.id));
    }
    if message.kind != TASK_REQUESTED {
        return Ok(Decision::Ignore);
    }
    let Ok(mut task) = NewTask::deserialize(&message.data) else {
        return Ok(Decision::Ignore);
    };
    let Some(workflow) = workflows.get(&task.workflow) else {
        return Err(format!(
            "task request {} names workflow {:?}, which the factory lacks",
            message.id, task.workflow
        ));
    };
    task.step.clone_from(&workflow.value.start);
    Ok(Decision::Create(task))
}

/// Whether an event satisfies a pull request wait. A checks wait needs no check run still pending.
pub(crate) fn satisfies(
    wait: &str,
    resource: &str,
    head: Option<&str>,
    kind: &str,
    data: &serde_json::Value,
) -> bool {
    let wanted = match wait {
        "pull_request" => "github.pull_request.closed",
        "review" => "github.pull_request_review.submitted",
        "checks" => "github.check_run.completed",
        _ => return false,
    };
    let names = data["repo"]
        .as_str()
        .zip(data["pull_request"].as_u64())
        .is_some_and(|(repo, number)| pull_request(repo, number) == resource);
    let checked = wait != "checks"
        || (data["pending"].as_u64() == Some(0)
            && head.is_none_or(|head| data["head_sha"].as_str() == Some(head)));
    kind == wanted && names && checked
}
