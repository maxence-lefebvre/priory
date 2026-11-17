use crate::intake::{self, OUTPUT, TASK_REQUESTED};
use herdr_bus::{Bus, Message, Timestamp};
use priory_factory::factory::Factory;
use serde_json::json;
use std::path::PathBuf;

pub const REQUESTED: &str = "priory.task.requested.cli";
const TRIGGER: &str = "cli";

pub struct Request {
    workflow: String,
    key: String,
    band: Option<String>,
    lane: Option<String>,
    repo: Option<String>,
    text: String,
}

pub fn parse(args: Vec<String>) -> Option<(PathBuf, Request)> {
    let (mut workflow, mut key, mut band, mut lane, mut repo) = (None, None, None, None, None);
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
    let request = Request {
        workflow: workflow?,
        key: key?,
        band,
        lane,
        repo,
        text,
    };
    Some((dir, request))
}

pub fn run(factory: &Factory, request: &Request) -> Result<(), String> {
    if !factory.workflows.contains_key(&request.workflow) {
        return Err(format!("unknown workflow `{}`", request.workflow));
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
    let data = json!({
        "workflow": request.workflow,
        "key": key,
        "band": request.band,
        "lane": request.lane,
        "trigger": TRIGGER,
        "source": source.id.to_string(),
    });
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
