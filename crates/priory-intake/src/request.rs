use crate::intake::{self, OUTPUT, TASK_REQUESTED};
use herdr_bus::{Bus, Message, Timestamp};
use priory_factory::factory::{Factory, Hold};
use serde_json::json;
use std::path::PathBuf;

pub const REQUESTED: &str = "priory.task.requested.cli";
const TRIGGER: &str = "cli";

/// A task request: `priory request`'s flags, or a report pick option's request.
#[derive(Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Request {
    /// The workflow the task starts.
    workflow: String,
    /// The task's key, which Priory prefixes with `cli:`.
    key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    band: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    lane: Option<String>,
    /// `owner/name`; `triggers/cli.toml`'s `repo` when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    repo: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    hold: Option<Hold>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tldr: Option<String>,
    /// What the task is about.
    text: String,
}

pub fn parse(args: Vec<String>) -> Option<(PathBuf, Request)> {
    let (mut workflow, mut key, mut band, mut lane, mut repo) = (None, None, None, None, None);
    let mut tldr = None;
    let (mut reason, mut until, mut after, mut merge, mut if_closed) =
        (None, None, None, None, None);
    let mut blockers = false;
    let mut positional = vec![];
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        if arg == "--" {
            positional.extend(args);
            break;
        }
        let flag = match arg.as_str() {
            "--workflow" => &mut workflow,
            "--key" => &mut key,
            "--band" => &mut band,
            "--lane" => &mut lane,
            "--repo" => &mut repo,
            "--tldr" => &mut tldr,
            "--hold" => &mut reason,
            "--until" => &mut until,
            "--after" => &mut after,
            "--merge" => &mut merge,
            "--if-closed" => &mut if_closed,
            "--blockers" => {
                blockers = true;
                continue;
            }
            _ => {
                positional.push(arg);
                continue;
            }
        };
        *flag = Some(args.next().filter(|value| !value.starts_with("--"))?);
    }
    let (dir, text) = match <[String; 1]>::try_from(positional) {
        Ok([text]) => (PathBuf::from("."), text),
        Err(positional) => {
            let [dir, text] = <[String; 2]>::try_from(positional).ok()?;
            (PathBuf::from(dir), text)
        }
    };
    let conditions = until.is_some() || after.is_some() || merge.is_some() || blockers;
    let hold = match reason {
        Some(reason) => Some(Hold {
            reason,
            until,
            after: after.map(|after| after.parse()).transpose().ok()?,
            merge,
            if_closed,
            blockers,
        }),
        None if conditions || if_closed.is_some() => return None,
        None => None,
    };
    let request = Request {
        workflow: workflow?,
        key: key?,
        band,
        lane,
        repo,
        hold,
        tldr,
        text,
    };
    Some((dir, request))
}

impl Request {
    pub fn key(&self) -> &str {
        &self.key
    }
}

pub fn run(factory: &Factory, request: &Request) -> Result<(), String> {
    if !factory.workflows.contains_key(&request.workflow) {
        return Err(format!("unknown workflow `{}`", request.workflow));
    }
    let bands = factory.bands();
    if let Some(band) = request.band.as_deref().filter(|band| !bands.contains(band)) {
        return Err(format!("band `{band}` is not one of {}", bands.join(", ")));
    }
    let trigger_repo = factory
        .triggers
        .get(TRIGGER)
        .and_then(|trigger| trigger.value.repo.as_ref());
    if request.repo.is_none() && trigger_repo.is_none() {
        return Err(
            "the request names no repo; pass --repo owner/name, or set repo in triggers/cli.toml"
                .to_owned(),
        );
    }
    let key = format!("cli:{}", request.key);
    let person = std::env::var("USER").ok();
    let time = Timestamp::now();
    let data = json!({
        "text": request.text,
        "person": person,
        "time": time.to_string(),
        "repo": request.repo,
    });
    let source = Message::new(REQUESTED, data)
        .id(intake::id(&format!(
            "{REQUESTED}:{}:{key}:{}",
            key.len(),
            request.text
        )))
        .subject(&key)
        .time(time);
    let mut data = json!({
        "workflow": request.workflow,
        "key": key,
        "band": request.band,
        "lane": request.lane,
        "trigger": TRIGGER,
        "source": source.id.to_string(),
        "repo": request.repo,
    });
    if let Some(hold) = &request.hold {
        data["hold"] = json!(hold);
    }
    if let Some(tldr) = &request.tldr {
        data["tldr"] = json!(tldr);
    }
    let task = Message::new(TASK_REQUESTED, data)
        .id(intake::id(&format!(
            "{TASK_REQUESTED}:{TRIGGER}:{}",
            source.id
        )))
        .subject(&key)
        .time(source.time);

    let bus = Bus::open_default().map_err(|error| error.to_string())?;
    bus.declare(&intake::output())
        .map_err(|error| error.to_string())?;
    for message in [source, task] {
        bus.publish(OUTPUT, &message)
            .map_err(|error| format!("publish on {OUTPUT}: {error}"))?;
    }
    Ok(())
}
