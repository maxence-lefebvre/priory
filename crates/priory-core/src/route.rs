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

/// The task key a message names: a task request's own, a CLI request's subject, or `github:{repo}#{number}` for a GitHub
/// event, its issue or pull request number. GitHub numbers both from one sequence per repository.
pub(crate) fn key(message: &Message) -> Option<String> {
    let data = &message.data;
    if message.kind == TASK_REQUESTED {
        return data["key"].as_str().map(str::to_owned);
    }
    if message.kind == priory_intake::request::REQUESTED {
        return message.subject.clone();
    }
    if !message.kind.starts_with("github.") {
        return None;
    }
    Some(format!(
        "github:{}#{}",
        data["repo"].as_str()?,
        data.get("issue")
            .unwrap_or(&data["pull_request"])
            .as_u64()?
    ))
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
