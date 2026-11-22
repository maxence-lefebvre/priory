use crate::intake::{self, OUTPUT, TASK_REQUESTED};
use crate::log::{self, Log};
use herdr_bus::{Message, Timestamp};
use priory_factory::factory::{Factory, Hold, Intervention};
use serde_json::json;
use std::path::PathBuf;

pub const REQUESTED: &str = "priory.task.requested.cli";
const TRIGGER: &str = "cli";

/// How a task can relate to another: it builds on it, fixes what it merged, takes it up after it
/// was dropped or failed, or only relates to it.
pub const RELATIONS: [&str; 4] = ["builds-on", "fixes", "takes-up", "relates"];

/// A relation a request asks its new task to have: `--relates KIND:TASK`.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Relation {
    /// `builds-on`, `fixes`, `takes-up` or `relates`.
    pub kind: String,
    /// The other task's id.
    pub task: i64,
}

impl std::str::FromStr for Relation {
    type Err = ();

    /// `KIND:TASK`, such as `builds-on:7`.
    fn from_str(value: &str) -> Result<Self, ()> {
        let (kind, task) = value.split_once(':').ok_or(())?;
        if !RELATIONS.contains(&kind) {
            return Err(());
        }
        let task = task.parse().map_err(drop)?;
        Ok(Relation {
            kind: kind.to_owned(),
            task,
        })
    }
}

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
    /// How the new task relates to existing ones.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    relates: Vec<Relation>,
    /// Who answers the task's asks; `triggers/cli.toml`'s when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    intervention: Option<Intervention>,
    /// What the task is about.
    text: String,
    /// A planning task's batch: its issues and its entries' workflow.
    #[serde(skip)]
    batch: Option<crate::events::BatchRequest>,
    /// The outside session that added the task.
    #[serde(skip)]
    session: Option<String>,
}

pub fn parse(args: Vec<String>) -> Option<(PathBuf, Request)> {
    let (mut workflow, mut key, mut band, mut lane, mut repo) = (None, None, None, None, None);
    let (mut tldr, mut intervention) = (None, None);
    let (mut reason, mut until, mut if_closed) = (None, None, None);
    let (mut after, mut merge, mut relates) = (vec![], vec![], vec![]);
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
            "--intervention" => &mut intervention,
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
            "--relates" => {
                relates.push(args.next()?.parse().ok()?);
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
        relates,
        intervention: intervention_of(intervention)?,
        text,
        batch: None,
        session: None,
    };
    Some((dir, request))
}

/// The intervention level `--intervention` names, or `None` for an unknown one.
fn intervention_of(level: Option<String>) -> Option<Option<Intervention>> {
    level.map_or(Some(None), |level| {
        serde_json::from_value(json!(level)).ok().map(Some)
    })
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub relates: Vec<Relation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intervention: Option<Intervention>,
    /// The outside session that asked for the task through `priory mcp --outside`. Only the
    /// daemon sets it, so neither a flag nor a request body can.
    #[serde(skip)]
    pub session: Option<String>,
}

pub fn parse_add(args: Vec<String>) -> Option<(PathBuf, Add)> {
    let (mut workflow, mut band, mut lane, mut repo) = (None, None, None, None);
    let (mut positional, mut relates) = (vec![], vec![]);
    let mut intervention = None;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        let flag = match arg.as_str() {
            "--intervention" => &mut intervention,
            "--" => {
                positional.extend(args);
                break;
            }
            "--workflow" => &mut workflow,
            "--band" => &mut band,
            "--lane" => &mut lane,
            "--repo" => &mut repo,
            "--relates" => {
                relates.push(args.next()?.parse().ok()?);
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
    let add = Add {
        text,
        workflow,
        band,
        lane,
        repo,
        relates,
        intervention: intervention_of(intervention)?,
        session: None,
    };
    Some((dir, add))
}

/// Why `add` published nothing.
#[derive(Debug)]
pub enum Refused {
    /// The request does not hold, for the reason given.
    Invalid(String),
    /// The same text already went to the task key given, so the bus dropped it.
    Repeated(String),
}

impl From<String> for Refused {
    fn from(reason: String) -> Self {
        Refused::Invalid(reason)
    }
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refused::Invalid(reason) => f.write_str(reason),
            Refused::Repeated(key) => write!(
                f,
                "{key} already got this exact request, so nothing was added; reword the text to send it again"
            ),
        }
    }
}

/// Publishes `add` as a request, with the factory's defaults for what it leaves unset, under the
/// next free key of the day, `cli:<YYYY-MM-DD>-<n>`, and returns that key.
pub fn add(factory: &Factory, add: &Add) -> Result<String, Refused> {
    let (keys, repos) = linked(factory, &add.text);
    let repo = match &repos[..] {
        [first, rest @ ..] if rest.iter().all(|repo| repo == first) => Some(first.clone()),
        _ => None,
    };
    let request = quick(factory, add, repo)?;
    let bus = open(factory)?;
    if let [key] = &keys[..] {
        let source = source(
            &request,
            key,
            &format!("{REQUESTED}:{}:{key}:{}", key.len(), add.text),
        );
        // Both ids derive from the text, so the bus keeps the first of each. A task request that
        // takes this call's time is new: an earlier add stopped before it, and this one completes it.
        publish(&*bus, &source)?;
        let task = task(&request, key, &source);
        let seq = bus
            .publish(OUTPUT, &task)
            .map_err(|error| format!("publish on {OUTPUT}: {error}"))?;
        let stored = bus.message(seq).map_err(|error| error.to_string())?;
        if stored.message.time != task.time {
            return Err(Refused::Repeated(key.clone()));
        }
        return Ok(key.clone());
    }
    claim_daily(&*bus, &request, "cli")
}

/// Publishes `request` under the next free key of the day, `<prefix>:<YYYY-MM-DD>-<n>`.
fn claim_daily(bus: &dyn Log, request: &Request, prefix: &str) -> Result<String, Refused> {
    let today = intake::now()?.to_zoned(jiff::tz::TimeZone::system()).date();
    for n in 1.. {
        let key = format!("{prefix}:{today}-{n}");
        if claim(bus, request, &key)? {
            return Ok(key);
        }
    }
    unreachable!("a day has a free key")
}

/// `priory batch add`'s flags, or `POST /api/batches`'s body: the issues a batch plans.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Batch {
    pub issues: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entries: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub band: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lane: Option<String>,
}

pub fn parse_batch(args: Vec<String>) -> Option<(PathBuf, Batch)> {
    let (mut repo, mut entries, mut band, mut lane) = (None, None, None, None);
    let mut issues = vec![];
    let mut args = args.into_iter().peekable();
    let dir = match args.peek() {
        Some(dir) if std::path::Path::new(dir).is_dir() => PathBuf::from(args.next()?),
        _ => PathBuf::from("."),
    };
    while let Some(arg) = args.next() {
        let flag = match arg.as_str() {
            "--repo" => &mut repo,
            "--entries" => &mut entries,
            "--band" => &mut band,
            "--lane" => &mut lane,
            _ if arg.starts_with("--") => return None,
            _ => {
                issues.push(arg);
                continue;
            }
        };
        *flag = Some(args.next().filter(|value| !value.starts_with("--"))?);
    }
    if issues.is_empty() {
        return None;
    }
    let batch = Batch {
        issues,
        repo,
        entries,
        band,
        lane,
    };
    Some((dir, batch))
}

/// Publishes the request of `batch`'s planning task on the factory's `[batch] workflow`, under the
/// next free key of the day, `batch:<YYYY-MM-DD>-<n>`, and returns that key.
pub fn add_batch(factory: &Factory, batch: &Batch) -> Result<String, Refused> {
    let Some(defaults) = factory.batch() else {
        return Err(
            "factory.toml has no [batch] workflow; set one to plan batches"
                .to_owned()
                .into(),
        );
    };
    if factory.runners.len() > 1 {
        return Err(format!(
            "batches need one runner; this factory has {}",
            factory.runners.len()
        )
        .into());
    }
    let mut keys = Vec::new();
    let mut repos = Vec::new();
    for issue in &batch.issues {
        let (key, repo) = issue_key(factory, issue)?;
        if !keys.contains(&key) {
            keys.push(key);
        }
        repos.extend(repo);
    }
    let repo = match &repos[..] {
        [first, rest @ ..] if rest.iter().all(|repo| repo == first) => Some(first.clone()),
        _ => None,
    };
    let entries = batch
        .entries
        .clone()
        .or_else(|| defaults.entries.clone())
        .or_else(|| factory.request().map(|request| request.workflow.clone()))
        .ok_or_else(|| "the batch names no entries workflow; pass --entries, or set entries in factory.toml's [batch] table".to_owned())?;
    if !factory.workflows.contains_key(&entries) {
        return Err(format!("unknown workflow `{entries}`").into());
    }
    let add = Add {
        text: format!("Plan these issues into a batch: {}", keys.join(", ")),
        workflow: Some(defaults.workflow.clone()),
        band: batch.band.clone(),
        lane: batch.lane.clone(),
        repo: batch.repo.clone(),
        relates: Vec::new(),
        intervention: None,
        session: None,
    };
    let mut request = quick(factory, &add, repo)?;
    request.batch = Some(crate::events::BatchRequest {
        issues: keys,
        entries,
    });
    claim_daily(&*open(factory)?, &request, "batch")
}

/// The task key of `issue`, `ENG-7`, `owner/name#42` or a GitHub or Linear URL, and the
/// repositories it is in.
fn issue_key(factory: &Factory, issue: &str) -> Result<(String, Vec<String>), String> {
    let (keys, repos) = linked(factory, issue);
    if let [key] = &keys[..] {
        return Ok((key.clone(), repos));
    }
    let linear = issue.split_once('-').is_some_and(|(team, number)| {
        !team.is_empty()
            && team.chars().all(|c| c.is_ascii_alphanumeric())
            && number.parse::<u64>().is_ok()
    });
    if linear {
        return Ok((format!("linear:{issue}"), Vec::new()));
    }
    if let Some((repo, number)) = issue.split_once('#')
        && priory_runtime::runtime::check_repo(repo).is_ok()
        && number.parse::<u64>().is_ok()
    {
        return Ok((format!("github:{issue}"), vec![repo.to_owned()]));
    }
    Err(format!(
        "`{issue}` is not an issue: give ENG-7, owner/name#42 or a URL"
    ))
}

/// Publishes `add` as a request under `key`, such as a Slack thread's `slack:<channel>:<ts>`, with
/// the factory's defaults for what it leaves unset. Only the first request for a key publishes.
pub fn add_to(factory: &Factory, add: &Add, key: &str) -> Result<String, Refused> {
    add_as(factory, add, key, 0, None)
}

/// As `add_to`, once `ended` tasks with `key` have ended, and from `person` rather than `$USER`.
/// Only the first request for a key and a count of ended tasks publishes, so a key whose task
/// ended takes a new one.
pub fn add_as(
    factory: &Factory,
    add: &Add,
    key: &str,
    ended: u64,
    person: Option<&str>,
) -> Result<String, Refused> {
    let request = quick(factory, add, None)?;
    let bus = open(factory)?;
    let name = match ended {
        0 => format!("{REQUESTED}:add:{key}"),
        ended => format!("{REQUESTED}:add:{key}:ended:{ended}"),
    };
    let mut source = source(&request, key, &name);
    if let Some(person) = person {
        source.data["person"] = json!(person);
    }
    let seq = bus
        .publish(OUTPUT, &source)
        .map_err(|error| format!("publish on {OUTPUT}: {error}"))?;
    let first = bus.message(seq).map_err(|error| error.to_string())?.message;
    // The task request's id derives from the first source's, so an add that stopped between the
    // two is completed here, and one that did not keeps its task. Its own time tells them apart.
    let task = task(&request, key, &first).time(Timestamp::now());
    let seq = bus
        .publish(OUTPUT, &task)
        .map_err(|error| format!("publish on {OUTPUT}: {error}"))?;
    let stored = bus.message(seq).map_err(|error| error.to_string())?;
    if stored.message.time != task.time {
        return Err(Refused::Repeated(key.to_owned()));
    }
    Ok(key.to_owned())
}

/// `add` as a request the factory takes, its repo else `repo`, its workflow and band else the
/// factory's `[request]` defaults.
fn quick(factory: &Factory, add: &Add, repo: Option<String>) -> Result<Request, String> {
    let defaults = factory.request();
    let workflow = add
        .workflow
        .clone()
        .or_else(|| defaults.map(|defaults| defaults.workflow.clone()))
        .ok_or_else(|| "the request names no workflow; pass --workflow, or set workflow in factory.toml's [request] table".to_owned())?;
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
        relates: add.relates.clone(),
        intervention: add.intervention,
        text: add.text.clone(),
        batch: None,
        session: add.session.clone(),
    };
    check(factory, &request)?;
    Ok(request)
}

/// Publishes `request` under `key` unless another request owns the key: the key alone names the
/// source event, so the first to publish it owns the key.
fn claim(bus: &dyn Log, request: &Request, key: &str) -> Result<bool, String> {
    let source = source(request, key, &format!("{REQUESTED}:add:{key}"));
    let seq = bus
        .publish(OUTPUT, &source)
        .map_err(|error| format!("publish on {OUTPUT}: {error}"))?;
    let stored = bus.message(seq).map_err(|error| error.to_string())?;
    if stored.message.data != source.data {
        return Ok(false);
    }
    publish(bus, &task(request, key, &source))?;
    Ok(true)
}

/// The issues and pull requests that GitHub and Linear URLs in `text` name, as task keys, and the
/// repositories they are in. A Linear issue's repository is that of the Linear trigger of its team.
fn linked(factory: &Factory, text: &str) -> (Vec<String>, Vec<String>) {
    let (mut keys, mut repos) = (Vec::new(), Vec::new());
    for parts in urls(text) {
        match parts[..] {
            ["github.com", owner, name, "issues" | "pull", number, ..]
                if number.parse::<u64>().is_ok() =>
            {
                keys.push(format!("github:{owner}/{name}#{number}"));
                repos.push(format!("{owner}/{name}"));
            }
            ["linear.app", _, "issue", issue, ..] => {
                let team = issue.split('-').next().unwrap_or_default();
                // Every matching trigger's repo counts, so two that disagree set none.
                let matching = factory.triggers.values().filter_map(|trigger| {
                    let trigger = &trigger.value;
                    let filter = trigger.filter.as_ref()?;
                    let linear = trigger.source.starts_with("linear");
                    (linear && filter.get("team").map(String::as_str) == Some(team))
                        .then(|| trigger.repo.clone())?
                });
                keys.push(format!("linear:{issue}"));
                repos.extend(matching);
            }
            _ => {}
        }
    }
    keys.sort();
    keys.dedup();
    (keys, repos)
}

/// Each `https://` URL in `text`, without its scheme, split on `/`, `#` and `?`.
fn urls(text: &str) -> impl Iterator<Item = Vec<&str>> {
    text.match_indices("https://").map(|(at, _)| {
        text[at + "https://".len()..]
            .split(|c: char| c.is_whitespace() || "()<>[]\"'`".contains(c))
            .next()
            .unwrap_or_default()
            .trim_end_matches(['.', ',', ';', ':', '!', '?'])
            .split(['/', '#', '?'])
            .collect()
    })
}

/// The pull requests that GitHub URLs in `text` name, as `github:owner/name#7`.
pub fn pull_requests(text: &str) -> Vec<String> {
    urls(text)
        .filter_map(|parts| match parts[..] {
            ["github.com", owner, name, "pull", number, ..] if number.parse::<u64>().is_ok() => {
                Some(format!("github:{owner}/{name}#{number}"))
            }
            _ => None,
        })
        .collect()
}

impl Request {
    /// How the new task relates to existing ones.
    pub fn relates(&self) -> &[Relation] {
        &self.relates
    }

    pub fn key(&self) -> &str {
        &self.key
    }

    /// What the request starts, as a person reads it before they trust it: its key, workflow,
    /// repository, who answers its asks, its hold and its text.
    pub fn summary(&self) -> String {
        let repo = self
            .repo
            .as_deref()
            .unwrap_or("the cli trigger's repository");
        let intervention = self
            .intervention
            .map_or("the cli trigger's", |intervention| intervention.as_str());
        let hold = self.hold.as_ref().map_or(String::new(), |hold| {
            format!(
                ", held {}",
                serde_json::to_string(hold).expect("a hold serializes")
            )
        });
        format!(
            "task cli:{} in workflow {} on {repo}, {intervention} intervention{hold}: {}",
            self.key, self.workflow, self.text
        )
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
    let trigger = factory.triggers.get(TRIGGER).map(|trigger| &trigger.value);
    if request.intervention == Some(Intervention::HandsOff)
        && trigger.and_then(|trigger| trigger.lead.as_ref()).is_none()
    {
        return Err(
            "--intervention hands-off needs a lead; set lead in triggers/cli.toml".to_owned(),
        );
    }
    let trigger_repo = trigger.and_then(|trigger| trigger.repo.as_ref());
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
    let bus = open(factory)?;
    for message in [source, task] {
        publish(&*bus, &message)?;
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
    if !request.relates.is_empty() {
        data["relates"] = json!(request.relates);
    }
    if let Some(intervention) = request.intervention {
        data["intervention"] = json!(intervention);
    }
    if let Some(batch) = &request.batch {
        data["batch"] = json!(batch);
    }
    if let Some(session) = &request.session {
        data["session"] = json!(session);
    }
    Message::new(TASK_REQUESTED, data)
        .id(intake::id(&format!(
            "{TASK_REQUESTED}:{TRIGGER}:{}",
            source.id
        )))
        .subject(key)
        .time(source.time)
}

fn open(factory: &Factory) -> Result<Box<dyn Log>, String> {
    let bus = log::open(factory.log()).map_err(|error| error.to_string())?;
    bus.declare(&intake::output())
        .map_err(|error| error.to_string())?;
    Ok(bus)
}

fn publish(bus: &dyn Log, message: &Message) -> Result<(), String> {
    bus.publish(OUTPUT, message)
        .map(|_| ())
        .map_err(|error| format!("publish on {OUTPUT}: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn arguments() -> impl Strategy<Value = Vec<String>> {
        let word = prop_oneof![
            Just("--workflow".to_owned()),
            Just("--key".to_owned()),
            Just("--hold".to_owned()),
            Just("--until".to_owned()),
            Just("--after".to_owned()),
            Just("--merge".to_owned()),
            Just("--relates".to_owned()),
            Just("--if-closed".to_owned()),
            Just("--intervention".to_owned()),
            Just("--blockers".to_owned()),
            Just("--".to_owned()),
            Just("99999999999999999999".to_owned()),
            "\\PC{0,30}",
            "[a-z./#:-]{0,3000}",
        ];
        proptest::collection::vec(word, 0..12)
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(512))]

        #[test]
        fn command_line_arguments_parse_or_are_refused_without_panicking(args in arguments()) {
            let _ = parse(args.clone());
            let _ = parse_add(args);
        }

        #[test]
        fn urls_in_any_text_are_read_without_panicking(
            text in prop_oneof![
                "\\PC{0,300}",
                "(https://|github.com|/pull/|/issues/|linear.app|/issue/|[0-9]|[ .,;:!?#()<>\\[\\]\"'`/é💥]){0,60}",
            ],
        ) {
            pull_requests(&text);
            linked(&Factory::default(), &text);
        }
    }
}
