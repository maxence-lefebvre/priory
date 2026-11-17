use crate::factory::{Loaded, Workflow};
use crate::state::Task;
use herdr_bus::Message;
use serde::Deserialize;
use std::collections::BTreeMap;

pub const TASK_REQUESTED: &str = "factory.task.requested";

/// What the daemon does with one inbox entry.
#[derive(Debug, PartialEq)]
pub enum Decision {
    Create(NewTask),
    Keep(i64),
    Ignore,
}

#[derive(Debug, PartialEq, Deserialize)]
pub struct NewTask {
    pub workflow: String,
    #[serde(skip)]
    pub step: String,
    pub key: String,
    pub band: Option<String>,
    pub lane: Option<String>,
    pub trigger: String,
    pub source: String,
}

/// The task key a message names: a task request's own, or `github:{repo}#{issue}` for a GitHub event.
pub fn key(message: &Message) -> Option<String> {
    let data = &message.data;
    if message.kind == TASK_REQUESTED {
        return data["key"].as_str().map(str::to_owned);
    }
    if !message.kind.starts_with("github.") {
        return None;
    }
    Some(format!(
        "github:{}#{}",
        data["repo"].as_str()?,
        data["issue"].as_u64()?
    ))
}

/// Routes `message`, given the active task that owns its key, if any.
pub fn route(
    message: &Message,
    owner: Option<&Task>,
    workflows: &BTreeMap<String, Loaded<Workflow>>,
) -> Decision {
    if let Some(task) = owner {
        return Decision::Keep(task.id);
    }
    if message.kind != TASK_REQUESTED {
        return Decision::Ignore;
    }
    let Ok(mut task) = NewTask::deserialize(&message.data) else {
        return Decision::Ignore;
    };
    let Some(workflow) = workflows.get(&task.workflow) else {
        return Decision::Ignore;
    };
    task.step.clone_from(&workflow.value.start);
    Decision::Create(task)
}
