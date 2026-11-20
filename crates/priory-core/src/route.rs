use crate::state::Task;
use herdr_bus::Message;
use priory_factory::factory::Factory;
use priory_factory::lanes::Subject;
use priory_intake::intake::review_key;
use serde::Deserialize;

const TASK_REQUESTED: &str = "factory.task.requested";
const REVIEW_REQUEST_REMOVED: &str = "github.pull_request.review_request_removed";

/// What the daemon does with one inbox entry.
#[derive(Debug, PartialEq)]
pub(crate) enum Decision {
    Create(Box<NewTask>),
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
    pub team: Option<String>,
    pub project: Option<String>,
    #[serde(skip)]
    pub lane_derived: bool,
    pub trigger: String,
    pub source: String,
    pub repo: Option<String>,
    #[serde(default)]
    pub hold: Option<priory_factory::factory::Hold>,
    #[serde(default)]
    pub tldr: Option<String>,
}

/// The resources a message names, which a task may own: a task request's key, a CLI request's
/// subject, a Linear event's issue, a Slack event's thread, or for a GitHub event its issue or pull
/// request and its pull request's head branch.
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
    if message.kind.starts_with("linear.") {
        let issue = data["issue"].as_str();
        return issue
            .map(|issue| format!("linear:{issue}"))
            .into_iter()
            .collect();
    }
    if message.kind.starts_with("slack.") {
        let (Some(channel), Some(thread)) = (
            data["channel"].as_str(),
            data.get("thread_ts").unwrap_or(&data["ts"]).as_str(),
        ) else {
            return Vec::new();
        };
        return vec![format!("slack:{channel}:{thread}")];
    }
    let Some(repo) = data["repo"]
        .as_str()
        .filter(|_| message.kind.starts_with("github."))
    else {
        return Vec::new();
    };
    let number = data.get("issue").unwrap_or(&data["pull_request"]).as_u64();
    let head = data["head_branch"].as_str();
    let review = data["reviewer"]
        .as_str()
        .zip(number)
        .map(|(reviewer, number)| review_key(repo, number, reviewer));
    number
        .map(|number| pull_request(repo, number))
        .into_iter()
        .chain(head.map(|head| branch(repo, head)))
        .chain(review)
        .collect()
}

/// The key of the review task a removed review request withdraws: one the reviewer did not
/// answer with a review, so someone took it back or the pull request closed.
pub(crate) fn withdraws(message: &Message) -> Option<String> {
    let data = &message.data;
    if message.kind != REVIEW_REQUEST_REMOVED || data["reviewed"] != false {
        return None;
    }
    let number = data["pull_request"].as_u64()?;
    Some(review_key(
        data["repo"].as_str()?,
        number,
        data["reviewer"].as_str()?,
    ))
}

/// An issue or pull request, as a resource: GitHub numbers both from one sequence per repository.
pub(crate) fn pull_request(repo: &str, number: u64) -> String {
    format!("github:{repo}#{number}")
}

pub(crate) fn branch(repo: &str, branch: &str) -> String {
    format!("github:{repo}@{branch}")
}

/// Whether `message` requests a task.
pub(crate) fn requests(message: &Message) -> bool {
    message.kind == TASK_REQUESTED
}

/// Whether `message` is a schedule's firing or a review re-request, which wakes or is coalesced
/// into the task it finds.
pub(crate) fn tick(message: &Message, factory: &Factory) -> bool {
    let data = &message.data;
    let review = || {
        let trigger = data["trigger"].as_str().unwrap_or_default();
        factory
            .triggers
            .get(trigger)
            .is_some_and(|trigger| trigger.value.reviewer.is_some())
    };
    message.kind == TASK_REQUESTED && (data.get("scheduled").is_some() || review())
}

/// Routes `message`, given the active task that owns its key, if any. A task request for a
/// workflow the factory lacks creates its task with no step, which waits until the workflow is back.
/// A request with no lane gets the one `lanes.toml` derives; no branch exists yet to match.
/// A new task's band is its request's, else its trigger's, else its workflow's, else the last one.
pub(crate) fn route(message: &Message, owner: Option<&Task>, factory: &Factory) -> Decision {
    if let Some(task) = owner {
        return Decision::Keep(task.id);
    }
    if message.kind != TASK_REQUESTED {
        return Decision::Ignore;
    }
    let Ok(mut task) = NewTask::deserialize(&message.data) else {
        return Decision::Ignore;
    };
    let workflow = factory.workflows.get(&task.workflow);
    if let Some(workflow) = workflow {
        task.step.clone_from(&workflow.value.start);
    }
    let trigger = factory.triggers.get(&task.trigger);
    task.repo = task
        .repo
        .or_else(|| trigger.and_then(|trigger| trigger.value.repo.clone()));
    let bands = factory.bands();
    // A message published before a band was renamed can carry a band the factory lacks.
    task.band = task
        .band
        .filter(|band| bands.contains(&band.as_str()))
        .or_else(|| trigger.and_then(|trigger| trigger.value.band.clone()))
        .or_else(|| workflow.and_then(|workflow| workflow.value.band.clone()))
        .or_else(|| bands.last().map(|band| (*band).to_owned()));
    if task.lane.is_none() {
        task.lane = Some(factory.lanes().lane(&Subject {
            key: &task.key,
            team: task.team.as_deref(),
            project: task.project.as_deref(),
            branch: None,
        }));
        task.lane_derived = true;
    }
    Decision::Create(Box::new(task))
}

/// Whether an event satisfies a resource wait. `wanted` is a checks wait's head, which needs no
/// check run still pending, or a Linear state wait's state.
pub(crate) fn satisfies(
    wait: &str,
    resource: &str,
    wanted: Option<&str>,
    kind: &str,
    data: &serde_json::Value,
) -> bool {
    let text = |field: &str| data[field].as_str().unwrap_or_default();
    let (expected, names) = match wait {
        "pull_request" | "review" | "checks" => (
            match wait {
                "pull_request" => "github.pull_request.closed",
                "review" => "github.pull_request_review.submitted",
                _ => "github.check_run.completed",
            },
            data["repo"]
                .as_str()
                .zip(data["pull_request"].as_u64())
                .is_some_and(|(repo, number)| pull_request(repo, number) == resource),
        ),
        "linear_comment" | "linear_state" => (
            if wait == "linear_comment" {
                "linear.comment.created"
            } else {
                "linear.issue.state_changed"
            },
            format!("linear:{}", text("issue")) == resource,
        ),
        "slack_reply" => (
            "slack.thread.replied",
            format!("slack:{}:{}", text("channel"), text("thread_ts")) == resource,
        ),
        _ => return false,
    };
    let checked = match wait {
        "checks" => {
            data["pending"].as_u64() == Some(0)
                && wanted.is_none_or(|head| data["head_sha"].as_str() == Some(head))
        }
        "linear_state" => wanted == Some(text("to")),
        _ => true,
    };
    kind == expected && names && checked
}
