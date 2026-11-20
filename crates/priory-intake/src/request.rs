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
    let (mut reason, mut until, mut if_closed) = (None, None, None);
    let (mut after, mut merge) = (vec![], vec![]);
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
            "--until" if until.is_none() => &mut until,
            "--after" => {
                after.push(args.next()?.parse().ok()?);
                continue;
            }
            "--merge" => {
                merge.push(args.next().filter(|value| !value.starts_with("--"))?);
                continue;
            }
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
    let (dir, text) = dir_and_text(positional)?;
    let conditions = until.is_some() || !after.is_empty() || !merge.is_empty() || blockers;
    let hold = match reason {
        Some(reason) => Some(Hold {
            reason,
            until,
            after,
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

/// `[DIR] TEXT`, with the factory folder `.` by default.
fn dir_and_text(positional: Vec<String>) -> Option<(PathBuf, String)> {
    match <[String; 1]>::try_from(positional) {
        Ok([text]) => Some((PathBuf::from("."), text)),
        Err(positional) => {
            let [dir, text] = <[String; 2]>::try_from(positional).ok()?;
            Some((PathBuf::from(dir), text))
        }
    }
}

/// A quick request: `priory add`'s flags, or `POST /api/request`'s body. What it leaves unset
/// comes from the factory's `[request]` table.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Add {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub band: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lane: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
}

pub fn parse_add(args: Vec<String>) -> Option<(PathBuf, Add)> {
    let (mut workflow, mut band, mut lane, mut repo) = (None, None, None, None);
    let mut positional = vec![];
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        let flag = match arg.as_str() {
            "--" => {
                positional.extend(args);
                break;
            }
            "--workflow" => &mut workflow,
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
    let (dir, text) = dir_and_text(positional)?;
    let add = Add {
        text,
        workflow,
        band,
        lane,
        repo,
    };
    Some((dir, add))
}

/// Publishes `add` as a request, with the factory's defaults for what it leaves unset, under the
/// next free key of the day, `cli:<YYYY-MM-DD>-<n>`, and returns that key.
pub fn add(factory: &Factory, add: &Add) -> Result<String, String> {
    let defaults = factory.request();
    let workflow = add
        .workflow
        .clone()
        .or_else(|| defaults.map(|defaults| defaults.workflow.clone()))
        .ok_or("the request names no workflow; pass --workflow, or set workflow in factory.toml's [request] table")?;
    let (keys, repos) = linked(factory, &add.text);
    let repo = match &repos[..] {
        [first, rest @ ..] if rest.iter().all(|repo| repo == first) => Some(first.clone()),
        _ => None,
    };
    let request = Request {
        workflow,
        key: String::new(),
        band: add
            .band
            .clone()
            .or_else(|| defaults.and_then(|defaults| defaults.band.clone())),
        lane: add.lane.clone(),
        repo: add.repo.clone().or(repo),
        hold: None,
        tldr: None,
        text: add.text.clone(),
    };
    check(factory, &request)?;
    let bus = open()?;
    if let [key] = &keys[..] {
        let source = source(
            &request,
            key,
            &format!("{REQUESTED}:{}:{key}:{}", key.len(), add.text),
        );
        let task = task(&request, key, &source);
        for message in [source, task] {
            publish(&bus, &message)?;
        }
        return Ok(key.clone());
    }
    let today = intake::now()?.to_zoned(jiff::tz::TimeZone::system()).date();
    for n in 1.. {
        let key = format!("cli:{today}-{n}");
        // The key alone names the source event, so the first add to publish it owns the key.
        let source = source(&request, &key, &format!("{REQUESTED}:add:{key}"));
        let seq = bus
            .publish(OUTPUT, &source)
            .map_err(|error| format!("publish on {OUTPUT}: {error}"))?;
        let stored = bus.message(seq).map_err(|error| error.to_string())?;
        if stored.message.data != source.data {
            continue;
        }
        publish(&bus, &task(&request, &key, &source))?;
        return Ok(key);
    }
    unreachable!("a day has a free key")
}

/// The issues and pull requests that GitHub and Linear URLs in `text` name, as task keys, and the
/// repositories they are in. A Linear issue's repository is that of the Linear trigger of its team.
fn linked(factory: &Factory, text: &str) -> (Vec<String>, Vec<String>) {
    let (mut keys, mut repos) = (Vec::new(), Vec::new());
    for (at, _) in text.match_indices("https://") {
        let url = text[at + "https://".len()..]
            .split(|c: char| c.is_whitespace() || "()<>[]\"'`".contains(c))
            .next()
            .unwrap_or_default()
            .trim_end_matches(['.', ',', ';', ':', '!', '?']);
        let parts: Vec<&str> = url.split(['/', '#', '?']).collect();
        match parts[..] {
            ["github.com", owner, name, "issues" | "pull", number, ..]
                if number.parse::<u64>().is_ok() =>
            {
                keys.push(format!("github:{owner}/{name}#{number}"));
                repos.push(format!("{owner}/{name}"));
            }
            ["linear.app", _, "issue", issue, ..] => {
                let team = issue.split('-').next().unwrap_or_default();
                let repo = factory.triggers.values().find_map(|trigger| {
                    let trigger = &trigger.value;
                    let filter = trigger.filter.as_ref()?;
                    let linear = trigger.source.starts_with("linear");
                    (linear && filter.get("team").map(String::as_str) == Some(team))
                        .then(|| trigger.repo.clone())?
                });
                keys.push(format!("linear:{issue}"));
                repos.extend(repo);
            }
            _ => {}
        }
    }
    keys.sort();
    keys.dedup();
    (keys, repos)
}

impl Request {
    pub fn key(&self) -> &str {
        &self.key
    }
}

/// Why `factory` refuses `request`, if it does.
pub fn check(factory: &Factory, request: &Request) -> Result<(), String> {
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
    if let Some(repo) = &request.repo {
        priory_runtime::runtime::check_repo(repo)?;
    }
    Ok(())
}

/// Publishes `request`. A `nonce`, such as a report pick's id, makes its ids differ from those of
/// the same request without it or with another nonce.
pub fn run(factory: &Factory, request: &Request, nonce: Option<&str>) -> Result<(), String> {
    check(factory, request)?;
    let key = format!("cli:{}", request.key);
    let name = match nonce {
        None => format!("{REQUESTED}:{}:{key}:{}", key.len(), request.text),
        Some(nonce) => format!(
            "{REQUESTED}:pick:{}:{nonce}:{}:{key}:{}",
            nonce.len(),
            key.len(),
            request.text
        ),
    };
    let source = source(request, &key, &name);
    let task = task(request, &key, &source);
    let bus = open()?;
    for message in [source, task] {
        publish(&bus, &message)?;
    }
    Ok(())
}

/// The source event of `request` under `key`, its id named by `name`.
fn source(request: &Request, key: &str, name: &str) -> Message {
    let person = std::env::var("USER").ok();
    let time = Timestamp::now();
    let data = json!({
        "text": request.text,
        "person": person,
        "time": time.to_string(),
        "repo": request.repo,
    });
    Message::new(REQUESTED, data)
        .id(intake::id(name))
        .subject(key)
        .time(time)
}

/// The task request that follows `source`.
fn task(request: &Request, key: &str, source: &Message) -> Message {
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
    Message::new(TASK_REQUESTED, data)
        .id(intake::id(&format!(
            "{TASK_REQUESTED}:{TRIGGER}:{}",
            source.id
        )))
        .subject(key)
        .time(source.time)
}

fn open() -> Result<Bus, String> {
    let bus = Bus::open_default().map_err(|error| error.to_string())?;
    bus.declare(&intake::output())
        .map_err(|error| error.to_string())?;
    Ok(bus)
}

fn publish(bus: &Bus, message: &Message) -> Result<(), String> {
    bus.publish(OUTPUT, message)
        .map(|_| ())
        .map_err(|error| format!("publish on {OUTPUT}: {error}"))
}
