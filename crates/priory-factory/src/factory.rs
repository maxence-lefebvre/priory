use crate::lanes::Lanes;
use crate::lines::{self, Lines};
use schemars::JsonSchema;
use serde::de::{DeserializeOwned, Deserializer, Error as _, IntoDeserializer};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::io;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Debug)]
pub struct Problem {
    pub file: PathBuf,
    pub at: Option<String>,
    pub message: String,
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.at {
            Some(at) => write!(f, "{}:{at}: {}", self.file.display(), self.message),
            None => write!(f, "{}: {}", self.file.display(), self.message),
        }
    }
}

/// `factory.toml` or `factory.yaml`: the factory's name and the daemon's settings.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(title = "factory.toml")]
pub struct Settings {
    /// The factory's name.
    pub name: String,
    /// The bands, highest first, when they are not the default ones.
    pub bands: Option<Vec<String>>,
    #[serde(default)]
    pub sweep: Sweep,
    /// The agent whose model reads a stalled run's screen; no handler runs without one.
    pub handler: Option<String>,
    /// How long a run goes without a hook event or a change on its pane before it counts as stalled.
    #[serde(default, deserialize_with = "duration")]
    #[schemars(with = "String", regex(pattern = r"^0*[1-9][0-9]*[smh]$"))]
    pub stalled: Option<Duration>,
    /// How long one git, gh or tmux command may run before Priory kills it.
    #[serde(default, deserialize_with = "duration")]
    #[schemars(with = "String", regex(pattern = r"^0*[1-9][0-9]*[smh]$"))]
    pub command_timeout: Option<Duration>,
    /// The project's gate: the command `priory gate` runs with `sh -c` in a task's worktree.
    pub gate: Option<String>,
    #[serde(default)]
    pub search: Search,
    pub telemetry: Option<Telemetry>,
    pub notify: Option<Notify>,
    pub serve: Option<Serve>,
    pub webhooks: Option<Webhooks>,
    pub slack: Option<Slack>,
    #[serde(default)]
    pub asks: Asks,
    pub request: Option<RequestDefaults>,
    /// Irreversible commands and MCP tools on top of the built-in ones.
    #[serde(default)]
    pub guard: crate::guard::Guards,
    /// Autolinks, in order: each key a regex, each value an `https://` URL template whose `$1`
    /// names the regex's first group. An entry with a built-in link's regex replaces it.
    #[serde(default, deserialize_with = "links")]
    #[schemars(with = "BTreeMap<String, String>", extend("default" = {}))]
    pub links: Vec<Link>,
}

/// Text that `pattern` matches links to `url`, its `$1`-style groups filled in and `{repo}` the
/// task's repository.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Link {
    pub pattern: String,
    pub url: String,
}

/// The built-in links: `#42` to the issue or pull request 42 in the task's repository.
const LINKS: &[(&str, &str)] = &[(r"\B#(\d+)\b", "https://github.com/{repo}/issues/$1")];

impl Link {
    /// Refuses a pattern that is not a regex, a URL that is not `https:`, and a group the regex
    /// lacks.
    pub fn check(&self) -> Result<(), String> {
        let Link { pattern, url } = self;
        let regex = regex::Regex::new(pattern).map_err(|error| {
            let error = error.to_string();
            let reason = error.lines().last().unwrap_or_default();
            format!("`[links]` `{pattern}` is not a regex: {reason}")
        })?;
        if !url.starts_with("https://") {
            return Err(format!(
                "`[links]` `{pattern}` links to `{url}`: a link starts with `https://`"
            ));
        }
        let rest = &url["https://".len()..];
        let host = &rest[..rest.find(['/', '?', '#']).unwrap_or(rest.len())];
        // A group in the host would let the linked text pick the site.
        let plain = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':');
        if host.is_empty() || !host.chars().all(plain) {
            return Err(format!(
                "`[links]` `{pattern}` links to `{url}`: a link's host is fixed, with no group or `{{repo}}` in it"
            ));
        }
        let groups = regex.captures_len() - 1;
        let missing = url
            .split('$')
            .skip(1)
            .map(|after| {
                &after[..after
                    .find(|c: char| !c.is_ascii_digit())
                    .unwrap_or(after.len())]
            })
            .find(|digits| {
                !digits.is_empty() && digits.parse::<usize>().map_or(true, |group| group > groups)
            });
        match missing {
            Some(group) => Err(format!(
                "`[links]` `{pattern}` names group ${group}, but the regex has {groups} group{}",
                if groups == 1 { "" } else { "s" }
            )),
            None => Ok(()),
        }
    }
}

/// A table in file order, which a map would lose.
fn links<'de, D: Deserializer<'de>>(input: D) -> Result<Vec<Link>, D::Error> {
    struct Ordered;
    impl<'de> serde::de::Visitor<'de> for Ordered {
        type Value = Vec<Link>;
        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a table of regexes and URL templates")
        }
        fn visit_map<A: serde::de::MapAccess<'de>>(
            self,
            mut map: A,
        ) -> Result<Vec<Link>, A::Error> {
            let mut links = Vec::new();
            while let Some((pattern, url)) = map.next_entry()? {
                links.push(Link { pattern, url });
            }
            Ok(links)
        }
    }
    input.deserialize_map(Ordered)
}

/// What `priory add` and `POST /api/request` use when a request names no workflow or band.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RequestDefaults {
    /// The workflow a request starts when it names none.
    pub workflow: String,
    /// The band of a request that names none.
    pub band: Option<String>,
}

/// What happens to the answer of an ask: where an agent's ask can run a command, and how an
/// outside ask's answer reaches the session that asked.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Asks {
    /// The folders, besides the task's worktree, where an answer can run an ask's command, by
    /// name. A relative folder is under the factory folder.
    #[serde(default)]
    pub targets: BTreeMap<String, PathBuf>,
    /// The command that delivers an answer to the session that asked, relative to the factory
    /// folder. It gets the ask's id, revision, session, `answered` or `dismissed`, and the answer
    /// as its arguments. Without it, the answer is only stored.
    pub deliver: Option<PathBuf>,
}

/// The HTTP server over the factory's state, in the daemon and in `priory serve`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Serve {
    /// The address and port to listen on; a loopback address unless `public` is set.
    #[serde(default = "listen")]
    #[schemars(with = "String")]
    pub listen: SocketAddr,
    /// The environment variable that holds the token clients send, never the token itself.
    #[serde(default = "token_env")]
    pub token_env: String,
    /// Whether to serve the web view's files as well as `/api`.
    #[serde(default = "web")]
    pub web: bool,
    /// Whether `listen` may be an address beyond this machine.
    #[serde(default)]
    pub public: bool,
}

fn listen() -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], 7340))
}

fn token_env() -> String {
    "PRIORY_TOKEN".to_owned()
}

fn web() -> bool {
    true
}

impl Serve {
    /// Refuses `listen` beyond this machine unless the factory says `public = true`.
    pub fn allows(&self, listen: SocketAddr) -> Result<(), String> {
        if listen.ip().is_loopback() || self.public {
            return Ok(());
        }
        Err(format!(
            "`[serve] listen` {listen} is beyond this machine: set `public = true` to serve it"
        ))
    }
}

/// The signed webhooks the HTTP server accepts, by source.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Webhooks {
    pub github: Option<GithubWebhook>,
    pub linear: Option<LinearWebhook>,
}

/// GitHub webhooks, at `POST /hooks/github`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GithubWebhook {
    /// The environment variable that holds the webhook's secret, never the secret itself.
    #[serde(default = "secret_env")]
    pub secret_env: String,
}

/// Linear webhooks, at `POST /hooks/linear`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LinearWebhook {
    /// The environment variable that holds the webhook's signing secret, never the secret itself.
    #[serde(default = "linear_secret_env")]
    pub secret_env: String,
}

/// Slack features beyond its triggers.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Slack {
    pub shortcut: Option<SlackShortcut>,
}

/// The "Queue in Priory" message shortcut, which the intake receives over Slack's Socket Mode.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SlackShortcut {
    /// The environment variable that holds the app-level token (`xapp-`), never the token itself.
    #[serde(default = "app_token_env")]
    pub token_env: String,
    /// The Slack user ids, such as `U0123ABCD`, allowed to queue a task; the shortcut refuses
    /// everyone else.
    pub users: Vec<String>,
}

fn app_token_env() -> String {
    "SLACK_APP_TOKEN".to_owned()
}

fn linear_secret_env() -> String {
    "PRIORY_LINEAR_WEBHOOK_SECRET".to_owned()
}

fn secret_env() -> String {
    "PRIORY_GITHUB_WEBHOOK_SECRET".to_owned()
}

/// Where the daemon exports its trace records as OpenTelemetry spans.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Telemetry {
    /// The OTLP/HTTP base URL, such as `http://127.0.0.1:4318`; spans go to its `/v1/traces`.
    pub otlp: String,
}

/// Where the daemon tells a person that the desk's next item changed, besides Web Push.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Notify {
    /// Whether the daemon runs `notify-send` with the title of each new next item that is an ask
    /// or an alarm.
    #[serde(default)]
    pub desktop: bool,
}

/// How tasks are found: who writes their TL;DR, and whether search compares embeddings too.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Search {
    /// The agent whose model writes a new task's TL;DR; no summarizer runs without one.
    pub tldr: Option<String>,
    /// Whether search compares embeddings of TL;DRs as well as keywords.
    #[serde(default)]
    pub embeddings: bool,
    /// The embedding model Priory runs in process, a Hugging Face repository.
    pub model: Option<String>,
    /// A command that reads text on stdin and prints a JSON array of floats, instead of `model`.
    pub command: Option<String>,
}

/// The embedding model when `[search]` names neither a model nor a command.
pub const MODEL: &str = "sentence-transformers/all-MiniLM-L6-v2";

/// What embeds a TL;DR.
#[derive(Debug, Clone, PartialEq)]
pub enum Embedder {
    Model(String),
    Command(String),
}

impl Search {
    /// The embedder, when embeddings are on.
    pub fn embedder(&self) -> Option<Embedder> {
        if !self.embeddings {
            return None;
        }
        Some(match (&self.command, &self.model) {
            (Some(command), _) => Embedder::Command(command.clone()),
            (None, model) => Embedder::Model(model.as_deref().unwrap_or(MODEL).to_owned()),
        })
    }
}

/// How the daemon frees room: the built-in worktree sweep, the person's scripts, and the sweeper
/// agent it starts when they do not free enough.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Sweep {
    /// How often the built-in worktree sweep runs; every 10 minutes when unset.
    #[serde(default, deserialize_with = "duration")]
    #[schemars(with = "String", regex(pattern = r"^0*[1-9][0-9]*[smh]$"))]
    pub every: Option<Duration>,
    /// Scripts run when the resource they name stalls the queue.
    #[serde(default)]
    pub scripts: Vec<Script>,
    /// How long the queue stays stalled after its scripts ran before the sweeper starts; 30 minutes
    /// when unset.
    #[serde(default, deserialize_with = "duration")]
    #[schemars(with = "String", regex(pattern = r"^0*[1-9][0-9]*[smh]$"))]
    pub after: Option<Duration>,
    /// The agent of the built-in `sweep` workflow; no sweeper starts without one.
    pub agent: Option<String>,
    /// The repository the sweeper works in and files its issue on, a GitHub `owner/name`.
    pub repo: Option<String>,
}

impl Sweep {
    pub fn repo(&self) -> &str {
        self.repo.as_deref().unwrap_or("maxence-lefebvre/priory")
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Script {
    /// A shell command, run by `sh -c` in `PRIORY_HOME`.
    pub run: String,
    /// The resource whose shortage runs the script.
    pub when: Resource,
    /// How long it may run before Priory kills it; 5 minutes when unset.
    #[serde(default, deserialize_with = "duration")]
    #[schemars(with = "String", regex(pattern = r"^0*[1-9][0-9]*[smh]$"))]
    pub timeout: Option<Duration>,
}

/// The built-in workflow a sweeper task runs, unless the factory defines its own.
pub const SWEEP: &str = "sweep";

/// The built-in workflow of a task that answers an ask in the person's place, for the lead `lead`.
/// No workflow file has a `/` in its name, so none collides.
pub fn lead_workflow(lead: &str) -> String {
    format!("priory/lead/{lead}")
}

const LEAD_WORKFLOW: &str = r#"
start = "answer"

[[steps]]
id = "answer"
decides = "model"
prompt = """A task of this factory asked a question, and you answer it in the person's place. \
The event below names the task, its ask, the revision, the question, its options, the asker's \
recommendation and trade-off. Read the task's worktree or its pull request if you need to. Then \
end with `answered`, with your answer as the whole `summary`: one of the options, word for word, \
when the ask lists options. End with `skipped` when the ask is no longer open."""
effect = "none"
next = { answered = "done", skipped = "done" }
"#;

/// Opening the issue waits for a person: it posts an agent's text to a repository others read.
const SWEEP_WORKFLOW: &str = r#"
start = "clean"

[[steps]]
id = "clean"
decides = "model"
prompt = """The queue is stalled: its head waits for room that the sweep scripts did not free. \
The event below names the short resource, how much is missing, and each script's output. \
Free that resource on this machine. Remove only what Priory or the sweep scripts manage, such as \
caches and build outputs under PRIORY_HOME. Never remove a worktree with unpushed commits, \
uncommitted changes or untracked files: `priory desk` lists the worktrees the sweep keeps and why. \
End with `cleaned` when you freed room, with a summary for an issue on Priory: its first line is \
the title, then what you removed by hand and the kind of sweep that would have done it. End with \
`nothing` when you freed nothing."""
effect = "irreversible"
autonomous = "it removes only what Priory or the sweep scripts manage, after they failed to free room"
next = { cleaned = "approve-issue", nothing = "done" }

[[steps]]
id = "approve-issue"
decides = "human"
effect = "none"
approves = ["open-issue"]
options = ["open", "skip"]
next = { open = "open-issue", skip = "done" }

[[steps]]
id = "open-issue"
decides = "code"
action = "open-issue"
effect = "irreversible"
next = { opened = "done", failed = "stop" }
"#;

/// A resource the room check measures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Resource {
    Memory,
    Disk,
    Cpu,
}

impl Resource {
    pub fn name(self) -> &'static str {
        match self {
            Resource::Memory => "memory",
            Resource::Disk => "disk",
            Resource::Cpu => "CPU",
        }
    }
}

/// The bands a factory has when `factory.toml` names none, highest first.
pub const BANDS: [&str; 4] = ["blocker", "now", "today", "later"];

/// The frontmatter of `agents/<name>/agent.md`: who runs a model step. The Markdown after it is
/// the agent's instructions.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[schemars(title = "agent.md frontmatter")]
pub struct Agent {
    /// One line on what the agent does.
    pub description: String,
    /// The coding agent CLI that runs it.
    pub harness: Harness,
    /// The harness's model name, such as `sonnet`; the harness default when unset.
    pub model: Option<String>,
    /// The harness's reasoning effort, such as `high`; the harness default when unset.
    pub effort: Option<String>,
    /// The runner it runs on, by Tailscale hostname, or `any`.
    pub runner: Option<String>,
    #[serde(default)]
    pub session: Session,
    /// The context size, in tokens, past which the daemon compacts the conversation between turns.
    pub compact_at: Option<std::num::NonZeroU64>,
    /// What a runner must have free to admit a run of this agent, in bytes.
    #[serde(default, deserialize_with = "size")]
    #[schemars(
        with = "String",
        regex(pattern = r"^(?=[^KMGT]*[1-9])([0-9]+(\.[0-9]*)?|\.[0-9]+)(KB|MB|GB|TB)$")
    )]
    pub memory: Option<u64>,
    /// What a runner keeps free on disk for a run of this agent, past its floor, in bytes.
    #[serde(default, deserialize_with = "size")]
    #[schemars(
        with = "String",
        regex(pattern = r"^(?=[^KMGT]*[1-9])([0-9]+(\.[0-9]*)?|\.[0-9]+)(KB|MB|GB|TB)$")
    )]
    pub disk: Option<u64>,
    /// The MCP servers the run gets, by name.
    #[serde(default)]
    pub mcp_servers: Vec<String>,
    #[serde(default)]
    pub permissions: Permissions,
    /// The Markdown after the frontmatter: the agent's instructions.
    #[serde(skip)]
    pub body: String,
}

/// Claude Code's own `permissions` settings, so a person can copy rules across.
#[derive(Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Permissions {
    /// The run's `--permission-mode`, not a setting: Claude's flag outranks every settings file.
    #[serde(default, deserialize_with = "refuse_bypass", skip_serializing)]
    pub default_mode: Option<PermissionMode>,
    /// Claude Code permission rules the run allows, such as `Bash(cargo test:*)`.
    #[serde(default)]
    pub allow: Vec<String>,
    /// Claude Code permission rules the run denies, such as `Bash(git push:*)`.
    #[serde(default)]
    pub deny: Vec<String>,
}

/// Claude Code's `--permission-mode`; `bypassPermissions` is refused.
#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum PermissionMode {
    /// Claude Code's `--permission-mode` no longer takes `default`, its older name for `manual`.
    #[serde(alias = "default")]
    Manual,
    AcceptEdits,
    Plan,
    Auto,
    DontAsk,
}

impl PermissionMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::AcceptEdits => "acceptEdits",
            Self::Plan => "plan",
            Self::Auto => "auto",
            Self::DontAsk => "dontAsk",
        }
    }
}

fn refuse_bypass<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<PermissionMode>, D::Error> {
    let Some(mode) = Option::<String>::deserialize(deserializer)? else {
        return Ok(None);
    };
    if mode == "bypassPermissions" {
        return Err(D::Error::custom(
            "`bypassPermissions` is not allowed: Priory never skips permission checks",
        ));
    }
    PermissionMode::deserialize(mode.into_deserializer()).map(Some)
}

/// The coding agent CLI that runs an agent.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Harness {
    ClaudeCode,
    Codex,
}

/// `per-task`: one conversation per task. `shared`: one conversation across tasks, such as a lead.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Session {
    #[default]
    PerTask,
    Shared,
}

/// Who answers a task's asks. Unset, every ask goes to a person.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Intervention {
    /// A person approves the plan: a grading step's result waits for an answer before the task
    /// moves on, even for a green grade.
    Plan,
    /// The trigger's `lead` answers asks in the person's place, except an ask whose answer lets an
    /// irreversible step run and an ask whose answer runs a command.
    HandsOff,
}

impl Intervention {
    pub fn as_str(self) -> &'static str {
        match self {
            Intervention::Plan => "plan",
            Intervention::HandsOff => "hands-off",
        }
    }
}

/// `triggers/<name>.toml` or `.yaml`: an event source, its filter and the workflow its events
/// start. The file name is the trigger's name.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(title = "trigger")]
pub struct Trigger {
    /// `github`, `linear`, `linear.view`, `slack` or `schedule`.
    pub source: String,
    /// The repository the trigger's tasks work in, a GitHub `owner/name` with no `.` or `..` name;
    /// the polled one for `github`.
    pub repo: Option<String>,
    /// The event that starts a task, such as `issues.labeled`; depends on `source`.
    #[serde(default)]
    pub on: String,
    /// The GitHub login whose review requests a `pull_request.review_requested` trigger polls.
    pub reviewer: Option<String>,
    /// The saved Linear view a `linear.view` trigger reads: its id, or the slug id ending its URL.
    pub view: Option<String>,
    /// The Slack channel id a `slack` trigger reads, such as `C0123`.
    pub channel: Option<String>,
    /// Values an event must have: `label` for GitHub; `team`, `project` (its name or id), `label`,
    /// `state` for Linear; `text` for Slack.
    pub filter: Option<BTreeMap<String, String>>,
    /// The workflow a matching event starts, by file name.
    pub workflow: String,
    /// Which events are the same work, such as `issue.number`; for a schedule, a fixed key.
    pub key: Option<String>,
    /// The band of its tasks.
    pub band: Option<String>,
    /// The lane of its tasks, `<team>/<theme>`; `lanes.toml` derives it when unset.
    pub lane: Option<String>,
    /// Whether the events of the first poll start tasks too.
    pub backfill: Option<bool>,
    /// The local time of day a schedule fires, `HH:MM`.
    #[serde(default, deserialize_with = "time_of_day")]
    #[schemars(with = "String", regex(pattern = r"^([01]?[0-9]|2[0-3]):[0-5][0-9]$"))]
    pub at: Option<jiff::civil::Time>,
    /// The weekdays a schedule fires on; every day when unset.
    #[serde(default, deserialize_with = "weekdays")]
    #[schemars(
        with = "Vec<String>",
        length(min = 1),
        extend("items" = { "enum": ["mon", "tue", "wed", "thu", "fri", "sat", "sun"] })
    )]
    pub days: Option<Vec<jiff::civil::Weekday>>,
    /// How often a schedule fires again after `at`, the same day; once a day when unset.
    #[serde(default, deserialize_with = "duration")]
    #[schemars(with = "String", regex(pattern = r"^0*[1-9][0-9]*[smh]$"))]
    pub every: Option<Duration>,
    /// When a schedule's day of firings ends; the end of the day when unset.
    #[serde(default, deserialize_with = "until")]
    #[schemars(
        with = "String",
        regex(pattern = r"^(([01]?[0-9]|2[0-3]):[0-5][0-9]|0*[1-9][0-9]*[smh])$")
    )]
    pub until: Option<Until>,
    /// The IANA zone `at` is read in, such as `Europe/Paris`; the machine's own when unset.
    #[serde(default, deserialize_with = "time_zone")]
    #[schemars(with = "String")]
    pub timezone: Option<jiff::tz::TimeZone>,
    /// What a schedule's firing tells the task it wakes.
    pub prompt: Option<String>,
    /// Files each task held, so it waits for a person or a condition before it starts.
    pub hold: Option<Hold>,
    /// Who answers its tasks' asks: `plan` or `hands-off`; a person when unset.
    pub intervention: Option<Intervention>,
    /// The agent that answers asks of a `hands-off` task: a `claude-code` agent with a `shared`
    /// session.
    pub lead: Option<String>,
}

/// Why a task is held, and the conditions that release it once all of them hold. With none, a
/// person does.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Hold {
    /// Why the task waits, shown to a person.
    pub reason: String,
    /// An RFC 3339 instant, or a duration such as `2 hours` after the hold.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<String>,
    /// Other tasks' ids, or Linear issues such as `ENG-7`: until each task ends and each issue is
    /// Done or Canceled. One value or a list.
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        deserialize_with = "one_or_many"
    )]
    #[schemars(with = "Option<OneOrMany<After>>")]
    pub after: Vec<After>,
    /// Pull requests, `owner/name#42` or `42` in the task's repository: until each merges. One
    /// value or a list.
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        deserialize_with = "one_or_many"
    )]
    #[schemars(with = "Option<OneOrMany<String>>")]
    pub merge: Vec<String>,
    /// What a pull request closed without a merge does: `release`, `drop` or `ask`, the default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub if_closed: Option<String>,
    /// Until the task's Linear issue has no open blocker.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub blockers: bool,
}

/// What a hold's `after` waits for: another task to end, or a Linear issue to close.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum After {
    Task(i64),
    Issue(String),
}

impl std::str::FromStr for After {
    type Err = String;

    /// A task id such as `12`, or a Linear issue key such as `ENG-7`.
    fn from_str(text: &str) -> Result<Self, String> {
        if let Ok(task) = text.parse() {
            return Ok(Self::Task(task));
        }
        let issue = text
            .split_once('-')
            .filter(|(team, number)| {
                team.starts_with(|c: char| c.is_ascii_alphabetic())
                    && team.chars().all(|c| c.is_ascii_alphanumeric())
                    && !number.is_empty()
                    && number.chars().all(|c| c.is_ascii_digit())
            })
            .is_some();
        if !issue {
            return Err(format!(
                "after takes a task id or a Linear issue such as ENG-7, not {text:?}"
            ));
        }
        Ok(Self::Issue(text.to_ascii_uppercase()))
    }
}

/// One value, or a list of them; null reads as none, as the single values it replaced did.
#[derive(Deserialize, JsonSchema)]
#[serde(untagged)]
#[schemars(rename = "OneOrMany_{T}")]
enum OneOrMany<T> {
    One(T),
    Many(Vec<T>),
}

fn one_or_many<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(match Option::<OneOrMany<T>>::deserialize(deserializer)? {
        None => vec![],
        Some(OneOrMany::One(one)) => vec![one],
        Some(OneOrMany::Many(many)) => many,
    })
}

/// The end of a schedule's day of firings: a time of day, or a duration after `at`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Until {
    At(jiff::civil::Time),
    After(Duration),
}

fn until<'de, D: Deserializer<'de>>(input: D) -> Result<Option<Until>, D::Error> {
    let text = String::deserialize(input)?;
    if let Ok(at) = jiff::civil::Time::strptime("%H:%M", &text) {
        return Ok(Some(Until::At(at)));
    }
    duration(text.clone().into_deserializer())
        .map(|after| after.map(Until::After))
        .map_err(|_: D::Error| {
            D::Error::custom(format!(
                "{text:?} is not a time of day such as \"18:00\" or a duration such as 8h"
            ))
        })
}

fn time_of_day<'de, D: Deserializer<'de>>(input: D) -> Result<Option<jiff::civil::Time>, D::Error> {
    let text = String::deserialize(input)?;
    jiff::civil::Time::strptime("%H:%M", &text)
        .map(Some)
        .map_err(|_| D::Error::custom(format!("{text:?} is not a time of day such as \"09:00\"")))
}

fn weekdays<'de, D: Deserializer<'de>>(
    input: D,
) -> Result<Option<Vec<jiff::civil::Weekday>>, D::Error> {
    use jiff::civil::Weekday::*;
    const NAMES: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];
    let names = Vec::<String>::deserialize(input)?;
    if names.is_empty() {
        return Err(D::Error::custom("`days` is empty, so it never fires"));
    }
    names
        .iter()
        .map(|name| match NAMES.iter().position(|day| day == name) {
            Some(index) => Ok([
                Monday, Tuesday, Wednesday, Thursday, Friday, Saturday, Sunday,
            ][index]),
            None => Err(D::Error::custom(format!(
                "{name:?} is not a day: use mon, tue, wed, thu, fri, sat or sun"
            ))),
        })
        .collect::<Result<_, _>>()
        .map(Some)
}

fn time_zone<'de, D: Deserializer<'de>>(input: D) -> Result<Option<jiff::tz::TimeZone>, D::Error> {
    let text = String::deserialize(input)?;
    jiff::tz::TimeZone::get(&text).map(Some).map_err(|_| {
        D::Error::custom(format!(
            "{text:?} is not a time zone such as \"Europe/Paris\""
        ))
    })
}

/// `workflows/<name>.toml` or `.yaml`: the typed steps a task goes through. The file name is the
/// workflow's name.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(title = "workflow")]
pub struct Workflow {
    /// The id of the first step.
    pub start: String,
    /// The band of its tasks when neither their request nor their trigger sets one.
    pub band: Option<String>,
    /// The model step a done task goes back to when its open pull request gets feedback; by
    /// default, the step that opened the pull request.
    pub reopen: Option<String>,
    /// The guarded calls every step of the workflow may make with no approval.
    #[serde(default)]
    pub guard: GuardAllow,
    /// The most pull requests a task's stack may hold: `open-pr` refuses a longer one. No cap
    /// when unset.
    pub max_prs: Option<std::num::NonZeroUsize>,
    pub steps: Vec<Step>,
}

/// A workflow's or a step's `guard` table.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GuardAllow {
    /// Guard names, built-in or from `[guard]` in `factory.toml`.
    #[serde(default)]
    pub allow: Vec<String>,
}

impl Workflow {
    pub fn step(&self, id: &str) -> Option<&Step> {
        self.steps.iter().find(|step| step.id == id)
    }

    /// Whether a path from the step `from` reaches the step `to`, `from` itself included.
    pub fn reaches(&self, from: &str, to: &str) -> bool {
        let steps = self
            .steps
            .iter()
            .map(|step| (step.id.as_str(), step))
            .collect();
        crate::rules::reachable(&steps, from, |_| false).contains(to)
    }
}

/// One step: who decides its outcome, what it may change, and where each outcome leads.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Step {
    /// The step's id, unique in the workflow; `done` and `stop` are reserved.
    pub id: String,
    pub decides: Decider,
    pub effect: Effect,
    /// The agent that runs a model step.
    pub agent: Option<String>,
    /// What a model step tells its agent.
    pub prompt: Option<String>,
    /// The answers a person picks from at a human step.
    #[serde(default)]
    pub options: Vec<String>,
    /// The built-in action a code step runs: `open-pr`, `open-issue` or `run`.
    pub action: Option<String>,
    /// The command a `run` action runs with `sh -c`.
    pub run: Option<String>,
    /// The irreversible steps a human step approves.
    #[serde(default)]
    pub approves: Vec<String>,
    /// The command a human step's answer runs, as an ask's `command`.
    pub command: Option<StepCommand>,
    /// Each outcome and the step it leads to, or `done` or `stop`.
    pub next: BTreeMap<String, String>,
    /// Who decides an outcome's edge, where it differs from the step's decider.
    #[serde(default)]
    pub by: BTreeMap<String, By>,
    /// What the daemon checks before it accepts an outcome, by outcome.
    #[serde(default)]
    pub proof: BTreeMap<String, Proof>,
    /// Why an irreversible step may run without a person's approval.
    pub autonomous: Option<String>,
    /// The guarded calls an irreversible step may make while it is approved or autonomous.
    #[serde(default)]
    pub guard: GuardAllow,
    /// How long one turn of a model step, or a `run` action's command, may run before Priory ends
    /// it.
    #[serde(default, deserialize_with = "duration")]
    #[schemars(with = "String", regex(pattern = r"^0*[1-9][0-9]*[smh]$"))]
    pub timeout: Option<Duration>,
}

/// The command a human step's answer runs once: `argv` exec'd in `target`, on the options in
/// `runs`.
#[derive(Debug, Deserialize, serde::Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StepCommand {
    /// The program and its arguments, never parsed by a shell.
    pub argv: Vec<String>,
    /// `worktree`, the task's worktree, or a name in `[asks.targets]`; `worktree` when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    /// The options whose answer runs it.
    pub runs: Vec<String>,
}

/// A duration such as `90s`, `15m` or `2h`.
fn duration<'de, D: serde::Deserializer<'de>>(input: D) -> Result<Option<Duration>, D::Error> {
    let text = String::deserialize(input)?;
    [("s", 1), ("m", 60), ("h", 3600)]
        .into_iter()
        .find_map(|(unit, scale)| {
            text.strip_suffix(unit)?
                .parse::<u64>()
                .ok()?
                .checked_mul(scale)
        })
        // The daemon compares turns in SQLite's signed seconds.
        .filter(|seconds| (1..=i64::MAX as u64).contains(seconds))
        .map(|seconds| Some(Duration::from_secs(seconds)))
        .ok_or_else(|| {
            serde::de::Error::custom(format!("{text:?} is not a duration such as 90s, 15m or 2h"))
        })
}

/// A size such as `512MB`, `6GB` or `1.5TB`, in bytes. Units are binary, as in `/proc/meminfo`.
fn size<'de, D: serde::Deserializer<'de>>(input: D) -> Result<Option<u64>, D::Error> {
    let text = String::deserialize(input)?;
    [("KB", 10), ("MB", 20), ("GB", 30), ("TB", 40)]
        .into_iter()
        .find_map(|(unit, shift)| {
            let number = text.strip_suffix(unit)?.parse::<f64>().ok()?;
            let bytes = number * (1u64 << shift) as f64;
            (bytes >= 1.0 && bytes < u64::MAX as f64).then_some(bytes as u64)
        })
        .map(Some)
        .ok_or_else(|| {
            serde::de::Error::custom(format!("{text:?} is not a size such as 512MB or 6GB"))
        })
}

/// Who decides a step's outcome: a `code` action, a `model` run or a `human`.
#[derive(Debug, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Decider {
    Code,
    Model,
    Human,
}

impl Decider {
    pub fn name(&self) -> &'static str {
        match self {
            Decider::Code => "code",
            Decider::Model => "model",
            Decider::Human => "human",
        }
    }
}

/// Who decides that a task follows an edge: `code`, a `model`, a `human`, or nobody while it is
/// `asleep` on a wait.
#[derive(Debug, Deserialize, Serialize, Clone, Copy, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum By {
    Code,
    Model,
    Human,
    Asleep,
}

/// A check the daemon makes before it accepts a model step's outcome.
#[derive(Debug, Deserialize, Serialize, Clone, Copy, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Proof {
    /// The factory's gate passed at the task's head, run by `priory gate`.
    GatePassed,
}

impl Proof {
    pub fn name(self) -> &'static str {
        match self {
            Proof::GatePassed => "gate-passed",
        }
    }
}

impl Step {
    /// Who decides the edge of `outcome`: its `by`, else the step's decider.
    pub fn by(&self, outcome: &str) -> By {
        self.by.get(outcome).copied().unwrap_or(match self.decides {
            Decider::Code => By::Code,
            Decider::Model => By::Model,
            Decider::Human => By::Human,
        })
    }
}

/// What a step may change: `none`, `read`, `reversible` or `irreversible`.
#[derive(Debug, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Effect {
    None,
    Read,
    Reversible,
    Irreversible,
}

impl Effect {
    pub fn name(&self) -> &'static str {
        match self {
            Effect::None => "none",
            Effect::Read => "read",
            Effect::Reversible => "reversible",
            Effect::Irreversible => "irreversible",
        }
    }
}

/// `runners/<tailscale-hostname>.toml` or `.yaml`: a machine that runs agents. Capacity is
/// measured live, so the file holds the platform and the share its agents may take.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(title = "runner")]
pub struct Runner {
    pub platform: Platform,
    /// The share of the machine its agents may take together; 90% of each when unset.
    pub agents: Option<Share>,
}

/// The machine's operating system and CPU architecture.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Platform {
    pub os: Os,
    /// The CPU architecture, such as `x86_64` or `aarch64`.
    pub arch: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Os {
    Linux,
    Macos,
    Windows,
}

/// Shares such as `60%`, above 0 and at most 100%.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Share {
    /// The fraction of the machine's CPUs the runner's agents may load together.
    #[serde(default, deserialize_with = "percent")]
    #[schemars(
        with = "String",
        regex(pattern = r"^(?=[^%]*[1-9])(0*100(\.0*)?|0*[0-9]{1,2}(\.[0-9]*)?|\.[0-9]+)%$")
    )]
    pub cpu: Option<f64>,
    /// The fraction of the machine's memory the runner's agents may take together.
    #[serde(default, deserialize_with = "percent")]
    #[schemars(
        with = "String",
        regex(pattern = r"^(?=[^%]*[1-9])(0*100(\.0*)?|0*[0-9]{1,2}(\.[0-9]*)?|\.[0-9]+)%$")
    )]
    pub memory: Option<f64>,
}

/// A share such as `60%`, as a fraction above 0 and at most 1.
fn percent<'de, D: serde::Deserializer<'de>>(input: D) -> Result<Option<f64>, D::Error> {
    let text = String::deserialize(input)?;
    text.strip_suffix('%')
        .and_then(|number| number.parse::<f64>().ok())
        .filter(|number| *number > 0.0 && *number <= 100.0)
        .map(|number| Some(number / 100.0))
        .ok_or_else(|| {
            serde::de::Error::custom(format!("{text:?} is not a share such as 60% or 90%"))
        })
}

/// Dollars per million tokens of each kind, for one model.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Price {
    #[serde(deserialize_with = "dollars")]
    pub input: f64,
    #[serde(deserialize_with = "dollars")]
    pub output: f64,
    #[serde(deserialize_with = "dollars")]
    pub cache_read: f64,
    #[serde(deserialize_with = "dollars")]
    pub cache_write: f64,
}

fn dollars<'de, D: Deserializer<'de>>(deserializer: D) -> Result<f64, D::Error> {
    let rate = f64::deserialize(deserializer)?;
    if rate.is_finite() && rate >= 0.0 {
        return Ok(rate);
    }
    Err(D::Error::custom(format!(
        "{rate} is not a price: give dollars per million tokens, zero or more"
    )))
}

/// `views/<name>.toml` or `.yaml`: a layout of the web view, a tree whose leaves are panels.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(title = "view")]
pub struct View {
    pub layout: Node,
}

/// A row or column splits its space between its children; tabs show one child at a time.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Node {
    Row(
        #[serde(deserialize_with = "children")]
        #[schemars(length(min = 1))]
        Vec<Node>,
    ),
    Column(
        #[serde(deserialize_with = "children")]
        #[schemars(length(min = 1))]
        Vec<Node>,
    ),
    Tabs(
        #[serde(deserialize_with = "children")]
        #[schemars(length(min = 1))]
        Vec<Node>,
    ),
    Panel(Panel),
}

fn children<'de, D: Deserializer<'de>>(input: D) -> Result<Vec<Node>, D::Error> {
    let nodes = Vec::<Node>::deserialize(input)?;
    if nodes.is_empty() {
        return Err(D::Error::custom(
            "a row, column or tabs needs at least one child",
        ));
    }
    Ok(nodes)
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Panel {
    Desk,
    Asks,
    Inbox,
    Queue,
    Tasks,
    Runs,
    Terminal,
    Workflow,
    Reports,
    Costs,
}

/// The views Priory ships, by name.
const VIEWS: &[(&str, &str)] = &[("default", include_str!("../views/default.toml"))];

pub struct Loaded<T> {
    pub file: PathBuf,
    pub value: T,
    pub lines: Lines,
}

#[derive(Default)]
pub struct Factory {
    /// The folder the factory was loaded from.
    pub dir: PathBuf,
    pub name: String,
    pub agents: BTreeMap<String, Loaded<Agent>>,
    pub triggers: BTreeMap<String, Loaded<Trigger>>,
    pub workflows: BTreeMap<String, Loaded<Workflow>>,
    pub runners: BTreeMap<String, Loaded<Runner>>,
    pub views: BTreeMap<String, Loaded<View>>,
    /// `prices.toml`: each model's price.
    pub prices: BTreeMap<String, Price>,
    pub lanes: Option<Loaded<Lanes>>,
    pub settings: Option<Loaded<Settings>>,
}

impl Factory {
    /// The factory's lanes, or the built-in ones: no rules and the `{team}/misc` fallback.
    pub fn lanes(&self) -> &Lanes {
        static BUILT_IN: Lanes = Lanes {
            fallback: None,
            rules: Vec::new(),
        };
        self.lanes.as_ref().map_or(&BUILT_IN, |lanes| &lanes.value)
    }

    /// The factory's `[sweep]` table, or the built-in sweep alone.
    pub fn sweep(&self) -> &Sweep {
        static BUILT_IN: Sweep = Sweep {
            every: None,
            scripts: Vec::new(),
            after: None,
            agent: None,
            repo: None,
        };
        self.settings
            .as_ref()
            .map_or(&BUILT_IN, |settings| &settings.value.sweep)
    }

    /// The factory's `[search]` table, or keyword search with no summarizer.
    pub fn search(&self) -> &Search {
        static BUILT_IN: Search = Search {
            tldr: None,
            embeddings: false,
            model: None,
            command: None,
        };
        self.settings
            .as_ref()
            .map_or(&BUILT_IN, |settings| &settings.value.search)
    }

    /// The factory's `[guard]` table, or none on top of the built-in guards.
    pub fn guards(&self) -> crate::guard::Guards {
        self.settings
            .as_ref()
            .map(|settings| settings.value.guard.clone())
            .unwrap_or_default()
    }

    /// How long one git, gh or tmux command may run; 5 minutes when unset.
    pub fn command_timeout(&self) -> Duration {
        self.settings
            .as_ref()
            .and_then(|settings| settings.value.command_timeout)
            .unwrap_or(Duration::from_secs(300))
    }

    /// The OTLP/HTTP base URL the daemon exports spans to; none sends nothing.
    pub fn otlp(&self) -> Option<&str> {
        let telemetry = self.settings.as_ref()?.value.telemetry.as_ref()?;
        Some(telemetry.otlp.as_str())
    }

    /// Whether the daemon sends desktop notifications.
    pub fn desktop_notifications(&self) -> bool {
        let notify = self
            .settings
            .as_ref()
            .and_then(|settings| settings.value.notify.as_ref());
        notify.is_some_and(|notify| notify.desktop)
    }

    /// The defaults of `priory add`; none makes it need `--workflow`.
    pub fn request(&self) -> Option<&RequestDefaults> {
        self.settings.as_ref()?.value.request.as_ref()
    }

    /// The HTTP server's settings; none listens nowhere.
    pub fn serve(&self) -> Option<&Serve> {
        self.settings.as_ref()?.value.serve.as_ref()
    }

    /// The views Priory ships, each replaced by the factory's view of the same name, and the
    /// factory's other views.
    pub fn views(&self) -> BTreeMap<String, View> {
        let mut views: BTreeMap<String, View> = VIEWS
            .iter()
            .map(|(name, text)| {
                let view = toml::from_str(text).expect("a shipped view parses");
                ((*name).to_owned(), view)
            })
            .collect();
        for (name, loaded) in &self.views {
            views.insert(name.clone(), loaded.value.clone());
        }
        views
    }

    /// The command that delivers an outside ask's answer, under the factory folder; none only
    /// stores it.
    pub fn deliver(&self) -> Option<PathBuf> {
        let deliver = self.settings.as_ref()?.value.asks.deliver.as_ref()?;
        Some(self.dir.join(deliver))
    }

    /// The folder of the ask command target `name`, under the factory folder when relative.
    pub fn target(&self, name: &str) -> Option<PathBuf> {
        let folder = self.settings.as_ref()?.value.asks.targets.get(name)?;
        Some(self.dir.join(folder))
    }

    /// The command `priory gate` runs, from `factory.toml`.
    pub fn gate(&self) -> Option<&str> {
        self.settings
            .as_ref()
            .and_then(|settings| settings.value.gate.as_deref())
    }

    /// The names of the ask command targets besides `worktree`.
    pub fn targets(&self) -> Vec<&str> {
        self.settings
            .iter()
            .flat_map(|settings| settings.value.asks.targets.keys())
            .map(String::as_str)
            .collect()
    }

    /// The factory's links in file order, then each built-in link no entry replaces.
    pub fn links(&self) -> Vec<Link> {
        let own = self
            .settings
            .as_ref()
            .map_or(&[][..], |settings| &settings.value.links[..]);
        let built_in = LINKS
            .iter()
            .filter(|(pattern, _)| !own.iter().any(|link| link.pattern == *pattern))
            .map(|(pattern, url)| Link {
                pattern: (*pattern).to_owned(),
                url: (*url).to_owned(),
            });
        own.iter().cloned().chain(built_in).collect()
    }

    /// The GitHub webhook's settings; none takes no GitHub webhook.
    pub fn github_webhook(&self) -> Option<&GithubWebhook> {
        self.settings
            .as_ref()?
            .value
            .webhooks
            .as_ref()?
            .github
            .as_ref()
    }

    /// The Linear webhook's settings; none takes no Linear webhook.
    pub fn linear_webhook(&self) -> Option<&LinearWebhook> {
        self.settings
            .as_ref()?
            .value
            .webhooks
            .as_ref()?
            .linear
            .as_ref()
    }

    /// The Slack message shortcut's settings; none takes no shortcut.
    pub fn slack_shortcut(&self) -> Option<&SlackShortcut> {
        self.settings
            .as_ref()?
            .value
            .slack
            .as_ref()?
            .shortcut
            .as_ref()
    }

    pub fn bands(&self) -> Vec<&str> {
        match self
            .settings
            .as_ref()
            .and_then(|settings| settings.value.bands.as_ref())
        {
            Some(bands) => bands.iter().map(String::as_str).collect(),
            None => BANDS.to_vec(),
        }
    }
}

pub fn load(dir: &Path) -> (Factory, Vec<Problem>) {
    let mut factory = Factory {
        dir: dir.to_path_buf(),
        ..Factory::default()
    };
    let mut problems = Vec::new();
    let mut found = Loader {
        dir,
        problems: &mut problems,
    };

    let mut settings = BTreeMap::new();
    for file in ["factory.toml", "factory.yaml", "factory.yml"].map(|name| dir.join(name)) {
        if let Some((value, lines)) = file
            .is_file()
            .then(|| found.parse::<Settings>(&file))
            .flatten()
        {
            found.define(&mut settings, "factory".into(), &file, value, lines);
        }
    }
    if let Some(settings) = settings.get("factory") {
        factory.name.clone_from(&settings.value.name);
    }
    if settings.is_empty() && found.problems.is_empty() {
        found.problems.push(Problem {
            file: PathBuf::from("factory.toml"),
            at: None,
            message: "missing: a factory needs factory.toml or factory.yaml".into(),
        });
    }

    let mut lanes = BTreeMap::new();
    for file in ["lanes.toml", "lanes.yaml", "lanes.yml"].map(|name| dir.join(name)) {
        if let Some((value, lines)) = file
            .is_file()
            .then(|| found.parse::<Lanes>(&file))
            .flatten()
        {
            found.define(&mut lanes, "lanes".into(), &file, value, lines);
        }
    }
    factory.lanes = lanes.remove("lanes");

    factory.settings = settings.remove("factory");
    for path in found
        .entries("agents")
        .into_iter()
        .filter(|path| path.is_dir())
    {
        let file = path.join("agent.md");
        let Some(source) = found.read(&file) else {
            continue;
        };
        if let Some((mut value, lines)) = found.parse_source::<Agent>(&file, &source) {
            value.body = body(&source);
            found.define(&mut factory.agents, file_name(&path), &file, value, lines);
        }
    }
    for file in found.config_files("triggers") {
        if let Some((value, lines)) = found.parse::<Trigger>(&file) {
            found.define(&mut factory.triggers, stem(&file), &file, value, lines);
        }
    }
    for file in found.config_files("workflows") {
        if let Some((value, lines)) = found.parse::<Workflow>(&file) {
            found.define(&mut factory.workflows, stem(&file), &file, value, lines);
        }
    }
    for file in found.config_files("views") {
        if let Some((value, lines)) = found.parse::<View>(&file) {
            found.define(&mut factory.views, stem(&file), &file, value, lines);
        }
    }
    for file in found.config_files("runners") {
        if let Some((value, lines)) = found.parse::<Runner>(&file) {
            found.define(&mut factory.runners, stem(&file), &file, value, lines);
        }
    }
    if let Some(agent) = factory.sweep().agent.clone()
        && factory.agents.contains_key(&agent)
        && !factory.workflows.contains_key(SWEEP)
    {
        let mut workflow: Workflow =
            toml::from_str(SWEEP_WORKFLOW).expect("the sweep workflow parses");
        workflow.steps[0].agent = Some(agent);
        let file = factory.settings.as_ref().map_or_else(
            || PathBuf::from("factory.toml"),
            |settings| settings.file.clone(),
        );
        let lines = Lines::default();
        factory.workflows.insert(
            SWEEP.to_owned(),
            Loaded {
                file,
                value: workflow,
                lines,
            },
        );
    }
    let leads: BTreeSet<(String, PathBuf)> = factory
        .triggers
        .values()
        .filter_map(|trigger| Some((trigger.value.lead.clone()?, trigger.file.clone())))
        .filter(|(lead, _)| factory.agents.contains_key(lead))
        .collect();
    for (lead, file) in leads {
        let mut workflow: Workflow =
            toml::from_str(LEAD_WORKFLOW).expect("the lead workflow parses");
        workflow.steps[0].agent = Some(lead.clone());
        factory
            .workflows
            .entry(lead_workflow(&lead))
            .or_insert(Loaded {
                file,
                value: workflow,
                lines: Lines::default(),
            });
    }
    if let Some(file) = ["prices.toml", "prices.yaml", "prices.yml"]
        .map(|name| dir.join(name))
        .into_iter()
        .find(|file| file.is_file())
        && let Some((prices, _)) = found.parse(&file)
    {
        factory.prices = prices;
    }
    (factory, problems)
}

struct Loader<'a> {
    dir: &'a Path,
    problems: &'a mut Vec<Problem>,
}

impl Loader<'_> {
    fn relative(&self, file: &Path) -> PathBuf {
        file.strip_prefix(self.dir).unwrap_or(file).to_path_buf()
    }

    fn define<T>(
        &mut self,
        map: &mut BTreeMap<String, Loaded<T>>,
        name: String,
        file: &Path,
        value: T,
        lines: Lines,
    ) {
        let file = self.relative(file);
        if let Some(existing) = map.get(&name) {
            let message = format!("`{name}` is also defined in {}", existing.file.display());
            self.problems.push(Problem {
                file,
                at: None,
                message,
            });
            return;
        }
        map.insert(name, Loaded { file, value, lines });
    }

    /// The entries of a factory folder; a missing folder has none.
    fn entries(&mut self, folder: &str) -> Vec<PathBuf> {
        let read = match fs::read_dir(self.dir.join(folder)) {
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Vec::new(),
            Err(error) => {
                self.problems.push(Problem {
                    file: folder.into(),
                    at: None,
                    message: error.to_string(),
                });
                return Vec::new();
            }
        };
        let mut paths = Vec::new();
        for entry in read {
            match entry {
                Ok(entry) => paths.push(entry.path()),
                Err(error) => self.problems.push(Problem {
                    file: folder.into(),
                    at: None,
                    message: error.to_string(),
                }),
            }
        }
        paths.sort();
        paths
    }

    fn config_files(&mut self, folder: &str) -> Vec<PathBuf> {
        self.entries(folder)
            .into_iter()
            .filter(|path| {
                matches!(
                    path.extension().and_then(|ext| ext.to_str()),
                    Some("toml" | "yaml" | "yml")
                )
            })
            .collect()
    }

    fn read(&mut self, file: &Path) -> Option<String> {
        match fs::read_to_string(file) {
            Ok(source) => Some(source),
            Err(error) => {
                self.problems.push(Problem {
                    file: self.relative(file),
                    at: None,
                    message: error.to_string(),
                });
                None
            }
        }
    }

    fn parse<T: DeserializeOwned>(&mut self, file: &Path) -> Option<(T, Lines)> {
        let source = self.read(file)?;
        self.parse_source(file, &source)
    }

    fn parse_source<T: DeserializeOwned>(
        &mut self,
        file: &Path,
        source: &str,
    ) -> Option<(T, Lines)> {
        let relative = self.relative(file);
        let parsed = match file.extension().and_then(|ext| ext.to_str()) {
            Some("md") => frontmatter(source),
            Some("toml") => Ok(Document {
                format: Format::Toml,
                text: source,
                first_line: 1,
            }),
            _ => Ok(Document {
                format: Format::Yaml,
                text: source,
                first_line: 1,
            }),
        }
        .and_then(|document| {
            let value = document.deserialize()?;
            Ok((value, document.lines()))
        });
        match parsed {
            Ok(parsed) => Some(parsed),
            Err((at, message)) => {
                self.problems.push(Problem {
                    file: relative,
                    at,
                    message,
                });
                None
            }
        }
    }
}

#[derive(Clone, Copy)]
enum Format {
    Toml,
    Yaml,
}

#[derive(Clone, Copy)]
struct Document<'a> {
    format: Format,
    text: &'a str,
    first_line: usize,
}

type ParseError = (Option<String>, String);

impl Document<'_> {
    fn lines(self) -> Lines {
        match self.format {
            Format::Toml => lines::toml(self.text, self.first_line),
            Format::Yaml => lines::yaml(self.text, self.first_line),
        }
        .unwrap_or_default()
    }

    fn deserialize<T: DeserializeOwned>(self) -> Result<T, ParseError> {
        let offset = self.first_line - 1;
        match self.format {
            Format::Toml => toml::from_str(self.text).map_err(|error| {
                let at = error.span().map(|span| {
                    let (line, column) = line_column(self.text, span.start);
                    format!("{}:{column}", line + offset)
                });
                (at, error.message().trim().to_string())
            }),
            Format::Yaml => serde_saphyr::from_str(self.text).map_err(|error| {
                let at = error.location().map(|location| {
                    format!(
                        "{}:{}",
                        location.line() as usize + offset,
                        location.column()
                    )
                });
                let message = error.without_snippet().to_string();
                // saphyr always suffixes its plain message with the location we already print.
                let message = message
                    .rsplit_once(" at line ")
                    .map_or(message.as_str(), |(text, _)| text);
                (at, message.to_string())
            }),
        }
    }
}

fn frontmatter(source: &str) -> Result<Document<'_>, ParseError> {
    let (first, body) = source.split_once('\n').unwrap_or((source, ""));
    let (format, fence) = match first.trim_end() {
        "+++" => (Format::Toml, "+++"),
        "---" => (Format::Yaml, "---"),
        _ => {
            return Err((
                Some("1:1".into()),
                "expected frontmatter fenced by +++ (TOML) or --- (YAML)".into(),
            ));
        }
    };
    let mut end = 0;
    for line in body.split_inclusive('\n') {
        if line.trim_end() == fence {
            return Ok(Document {
                format,
                text: &body[..end],
                first_line: 2,
            });
        }
        end += line.len();
    }
    Err((
        Some("1:1".into()),
        format!("frontmatter is not closed by {fence}"),
    ))
}

/// The text after a frontmatter that `frontmatter` accepted.
fn body(source: &str) -> String {
    let (fence, rest) = source.split_once('\n').unwrap_or((source, ""));
    rest.split_inclusive('\n')
        .skip_while(|line| line.trim_end() != fence.trim_end())
        .skip(1)
        .collect::<String>()
        .trim()
        .to_owned()
}

pub(crate) fn line_column(text: &str, offset: usize) -> (usize, usize) {
    let before = &text[..offset.min(text.len())];
    let line = before.matches('\n').count() + 1;
    let column = before
        .rsplit('\n')
        .next()
        .map_or(0, |last| last.chars().count())
        + 1;
    (line, column)
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn stem(path: &Path) -> String {
    path.file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default()
}
