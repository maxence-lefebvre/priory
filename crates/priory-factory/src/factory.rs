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
    #[serde(default)]
    pub queue: Queue,
    #[serde(default)]
    pub intake: IntakeSettings,
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
    /// The scorer that grades a plan no person reads: a grade that leads to a model or code step
    /// of a task without `intervention = "plan"`. It can only refuse the grade.
    pub critic: Option<String>,
    /// The judge that reads a step's lessons before an approval step asks a person: a result that
    /// repeats one goes back to the agent once. It can only send the result back.
    pub pre_review: Option<String>,
    /// A writing guide, as a path relative to the factory folder. Priory appends its text to the
    /// system prompt of every agent it starts.
    pub writing: Option<String>,
    #[serde(default)]
    pub search: Search,
    #[serde(default)]
    pub log: EventLog,
    pub telemetry: Option<Telemetry>,
    pub notify: Option<Notify>,
    pub serve: Option<Serve>,
    pub webhooks: Option<Webhooks>,
    pub slack: Option<Slack>,
    #[serde(default)]
    pub asks: Asks,
    #[serde(default)]
    pub runs: Runs,
    pub request: Option<RequestDefaults>,
    pub batch: Option<BatchDefaults>,
    /// Irreversible commands and MCP tools on top of the built-in ones.
    #[serde(default)]
    pub guard: crate::guard::Guards,
    /// Autolinks, in order: each key a regex, each value an `https://` URL template whose `$1`
    /// names the regex's first group. An entry with a built-in link's regex replaces it.
    #[serde(default, deserialize_with = "links")]
    #[schemars(with = "BTreeMap<String, String>", extend("default" = {}))]
    pub links: Vec<Link>,
    pub kit: Option<Kit>,
    pub memories: Option<MemoriesRepo>,
    /// Spend caps in dollars for every task, and for each lane by its name.
    pub caps: Option<Caps>,
    /// Recipes by repository, each key a GitHub `owner/name`: the commands the daemon runs in a
    /// task's worktree of that repository.
    #[serde(default)]
    pub repos: BTreeMap<String, Repo>,
    /// A decision model for each decision point that names one.
    #[serde(default)]
    pub decisions: Decisions,
    /// Who may consult which investigator, and the bounds of one consult.
    pub consult: Option<Consult>,
    /// What a person states about each MCP server an investigator names, by the server's name.
    #[serde(default)]
    pub mcp: BTreeMap<String, McpServer>,
    /// Each sandbox a workflow can turn on with `sandbox`, by its name.
    #[serde(default)]
    pub sandboxes: BTreeMap<String, Sandbox>,
    /// Priory records Claude Code's and Codex's folder trust for the worktrees and clones it
    /// creates for the repositories the factory names, so no run stops on a trust dialog.
    #[serde(default)]
    pub trust_worktrees: bool,
}

/// `[sandboxes.<name>]`: a jail around a run's agent, which sees its worktree, its harness's own
/// home and what the sandbox names, each at the same path as outside.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Sandbox {
    pub kind: SandboxKind,
    /// Paths bound read-only, each absolute or under `~/`.
    #[serde(default)]
    pub read: Vec<String>,
    /// Paths bound read-write, each absolute or under `~/`.
    #[serde(default)]
    pub write: Vec<String>,
    /// The daemon's variables the run keeps, by name, beside the few every jailed run keeps.
    #[serde(default)]
    pub env: Vec<String>,
}

/// How a sandbox is built.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SandboxKind {
    /// A bubblewrap jail: Linux only, with the `bwrap` program on `PATH`.
    Bwrap,
}

/// `[consult]`: the bounds of one consult, and `[consult.allow]`, who may consult whom.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Consult {
    /// How long from the `consult` call to its report, the wait for room included; 15m when unset.
    #[serde(default, deserialize_with = "duration")]
    #[schemars(with = "String", regex(pattern = r"^0*[1-9][0-9]*[smh]$"))]
    pub deadline: Option<Duration>,
    /// What one consult may cost, in dollars; 0.50 when unset.
    #[serde(default, deserialize_with = "cap")]
    #[schemars(extend("exclusiveMinimum" = 0))]
    pub cap: Option<f64>,
    /// How many consults one task may make; 5 when unset.
    pub per_task: Option<std::num::NonZeroU32>,
    /// Each asking agent's investigators, by the asking agent's name. An agent with no line
    /// consults nobody.
    #[serde(default)]
    pub allow: BTreeMap<String, Vec<String>>,
}

impl Consult {
    pub fn deadline(&self) -> Duration {
        self.deadline.unwrap_or(Duration::from_secs(15 * 60))
    }

    pub fn cap(&self) -> f64 {
        self.cap.unwrap_or(0.5)
    }

    pub fn per_task(&self) -> u32 {
        self.per_task.map_or(5, std::num::NonZeroU32::get)
    }
}

/// `[mcp.<name>]`: a person's statement about one MCP server of their own harness config.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct McpServer {
    /// Where the service enforces that the server only reads, such as "The token has the
    /// org:read and event:read scopes only." Priory cannot check it.
    pub read_only: Option<String>,
}

/// `[decisions]`: the decision model of each decision point. A point with none keeps its own
/// behaviour.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Decisions {
    /// Sorts a person's feedback on a pull request before it wakes the task.
    pub triage: Option<Decision>,
}

/// A decision model: a command that answers typed questions with probabilities, and how much of
/// its answer Priory trusts.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Decision {
    /// The command, run with `sh -c` in the factory's folder. It reads the question as JSON on
    /// stdin and prints the answer as JSON.
    #[schemars(length(min = 1))]
    pub run: String,
    /// The environment variable that holds the model's key, never the key itself. The command
    /// sees it, `PATH` and `HOME`, and no other variable.
    pub key_env: String,
    /// The file of a person's labeled cases, relative to the factory's folder, named `*.eval.toml`.
    pub evaluation: PathBuf,
    /// The least share of labeled cases it must sort as the person did; 0.9 when unset.
    #[schemars(range(min = 0.5, max = 1.0))]
    pub agreement: Option<f64>,
    /// The least probability of an answer that Priory acts on; 0.9 when unset.
    #[schemars(range(min = 0.5, max = 1.0))]
    pub threshold: Option<f64>,
    /// How long one call may run; 5s when unset, 30s at most.
    #[serde(default, deserialize_with = "duration")]
    #[schemars(with = "String", regex(pattern = r"^0*[1-9][0-9]*[smh]$"))]
    pub timeout: Option<Duration>,
    /// The most calls the daemon makes in a day; 200 when unset.
    #[schemars(range(min = 1))]
    pub daily_calls: Option<u32>,
}

impl Decision {
    pub fn agreement(&self) -> f64 {
        self.agreement.unwrap_or(0.9)
    }

    pub fn threshold(&self) -> f64 {
        self.threshold.unwrap_or(0.9)
    }

    pub fn timeout(&self) -> Duration {
        self.timeout.unwrap_or(Duration::from_secs(5))
    }

    pub fn daily_calls(&self) -> u32 {
        self.daily_calls.unwrap_or(200)
    }
}

/// The factory's spend caps, in dollars, which the daemon enforces from `prices.toml` and the
/// steps' token counts.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Caps {
    /// What one task may spend.
    #[serde(default, deserialize_with = "cap")]
    #[schemars(extend("exclusiveMinimum" = 0))]
    pub task: Option<f64>,
    /// What the factory may spend in a local day.
    #[serde(default, deserialize_with = "cap")]
    #[schemars(extend("exclusiveMinimum" = 0))]
    pub day: Option<f64>,
    /// What the factory may spend in a week from Monday 00:00 local time.
    #[serde(default, deserialize_with = "cap")]
    #[schemars(extend("exclusiveMinimum" = 0))]
    pub week: Option<f64>,
    /// Each lane's caps, by its `<team>/<theme>` name. A lane's day or week cap holds new starts.
    #[serde(default)]
    pub lanes: BTreeMap<String, Limits>,
}

/// Spend caps in dollars: per task, per local day and per week.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    /// What one task may spend.
    #[serde(default, deserialize_with = "cap")]
    #[schemars(extend("exclusiveMinimum" = 0))]
    pub task: Option<f64>,
    /// What may be spent in a local day.
    #[serde(default, deserialize_with = "cap")]
    #[schemars(extend("exclusiveMinimum" = 0))]
    pub day: Option<f64>,
    /// What may be spent in a week from Monday 00:00 local time.
    #[serde(default, deserialize_with = "cap")]
    #[schemars(extend("exclusiveMinimum" = 0))]
    pub week: Option<f64>,
}

fn cap<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<f64>, D::Error> {
    let dollars = f64::deserialize(deserializer)?;
    if dollars.is_finite() && dollars > 0.0 {
        return Ok(Some(dollars));
    }
    Err(D::Error::custom(format!(
        "{dollars} is not a spend cap: give dollars above zero"
    )))
}

/// A `max_running` of at least one: a count of zero would never start a task again.
pub(crate) fn max_running<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<u32>, D::Error> {
    match u32::deserialize(deserializer)? {
        0 => Err(D::Error::custom(
            "`max_running` is 0: give at least 1, or hold the tasks instead",
        )),
        count => Ok(Some(count)),
    }
}

/// `[repos."owner/name"]`: what the daemon runs in a worktree of the repository, so no run has to
/// find out how to install and build it.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Repo {
    /// Runs after the daemon adds a task's new worktree, before its agent starts. A failure blocks
    /// the task.
    pub setup: Option<Recipe>,
    /// Runs after the daemon adds back the worktree a retirement removed, before its agent
    /// starts. A failure blocks the task.
    pub resume: Option<Recipe>,
    /// Runs before the sweep or a retirement removes the worktree. A failure keeps the worktree.
    pub teardown: Option<Recipe>,
}

/// One command of a repository's recipe: `argv`, which Priory execs as it is, or `shell`, which it
/// runs with `sh -c`.
#[derive(Debug, Clone, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Recipe {
    /// The program and its arguments, run with no shell.
    #[schemars(length(min = 1))]
    pub argv: Option<Vec<String>>,
    /// A shell command, run with `sh -c`.
    #[schemars(length(min = 1))]
    pub shell: Option<String>,
    /// How long it may run before Priory kills it with its process group; `command_timeout` when
    /// unset.
    #[serde(default, deserialize_with = "duration")]
    #[schemars(with = "String", regex(pattern = r"^0*[1-9][0-9]*[smh]$"))]
    pub timeout: Option<Duration>,
}

/// When a repository's recipe runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Moment {
    Setup,
    Resume,
    Teardown,
}

impl Moment {
    pub const ALL: [Moment; 3] = [Moment::Setup, Moment::Resume, Moment::Teardown];

    pub fn name(self) -> &'static str {
        match self {
            Moment::Setup => "setup",
            Moment::Resume => "resume",
            Moment::Teardown => "teardown",
        }
    }
}

impl Repo {
    pub fn recipe(&self, moment: Moment) -> Option<&Recipe> {
        match moment {
            Moment::Setup => self.setup.as_ref(),
            Moment::Resume => self.resume.as_ref(),
            Moment::Teardown => self.teardown.as_ref(),
        }
    }
}

impl Recipe {
    /// What the daemon execs: `argv` as it is, or `sh -c <shell>`.
    pub fn exec(&self) -> Vec<String> {
        match (&self.argv, &self.shell) {
            (Some(argv), _) => argv.clone(),
            (None, Some(shell)) => vec!["sh".into(), "-c".into(), shell.clone(), "sh".into()],
            (None, None) => Vec::new(),
        }
    }

    /// The command as a shell reads it, for the guard and for messages.
    pub fn command(&self) -> String {
        match (&self.argv, &self.shell) {
            (Some(argv), _) => argv
                .iter()
                .map(|word| {
                    if !word.is_empty()
                        && word
                            .chars()
                            .all(|c| c.is_ascii_alphanumeric() || "-_./=:@%+,".contains(c))
                    {
                        word.clone()
                    } else {
                        format!("'{}'", word.replace('\'', "'\\''"))
                    }
                })
                .collect::<Vec<_>>()
                .join(" "),
            (None, Some(shell)) => shell.clone(),
            (None, None) => String::new(),
        }
    }
}

/// A kit repository whose skills, instructions and memories every run gets, before the factory's
/// own. Each runner fetches it with git's own credentials.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Kit {
    /// The repository, as `git fetch` takes it: a URL, or a path, which a relative one takes from
    /// the factory folder.
    #[schemars(length(min = 1))]
    pub repo: String,
    /// The branch, tag or commit to fetch; the repository's default branch when unset.
    #[serde(rename = "ref")]
    pub reference: Option<String>,
    /// The skills folder inside the repository; `skills` when unset.
    pub skills: Option<String>,
    /// The instructions file inside the repository; `instructions.md` when unset.
    pub instructions: Option<String>,
    /// The memories folder inside the repository; `memories` when unset.
    pub memories: Option<String>,
    /// How often a runner fetches the ref again; every 5 minutes when unset.
    #[serde(default, deserialize_with = "duration")]
    #[schemars(with = "String", regex(pattern = r"^0*[1-9][0-9]*[smh]$"))]
    pub every: Option<Duration>,
}

/// A memories repository whose memories every run gets, after the kit's and before the factory's
/// own, and where the daily pull request of accepted memories goes in place of the kit. Each runner
/// fetches it with git's own credentials.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MemoriesRepo {
    /// The repository, as `git fetch` takes it: a URL, or a path, which a relative one takes from
    /// the factory folder.
    #[schemars(length(min = 1))]
    pub repo: String,
    /// The branch, tag or commit to fetch; the repository's default branch when unset.
    #[serde(rename = "ref")]
    pub reference: Option<String>,
    /// The memories folder inside the repository; `memories` when unset.
    pub path: Option<String>,
    /// How often a runner fetches the ref again; every 5 minutes when unset.
    #[serde(default, deserialize_with = "duration")]
    #[schemars(with = "String", regex(pattern = r"^0*[1-9][0-9]*[smh]$"))]
    pub every: Option<Duration>,
}

impl MemoriesRepo {
    /// The ref to fetch: `HEAD`, the remote's default branch, when unset.
    pub fn reference(&self) -> &str {
        self.reference.as_deref().unwrap_or("HEAD")
    }

    /// The memories folder inside the repository.
    pub fn path(&self) -> &str {
        self.path.as_deref().unwrap_or("memories")
    }

    pub fn every(&self) -> Duration {
        self.every.unwrap_or(KIT_EVERY)
    }
}

/// Loads the memories of the memories repository checked out in `dir`, with the problems
/// `priory check` would report for the same files in a factory.
pub fn load_memories_repo(dir: &Path, repo: &MemoriesRepo) -> (KitFiles, Vec<Problem>) {
    let mut files = KitFiles::default();
    let mut problems = Vec::new();
    let mut found = Loader {
        dir,
        problems: &mut problems,
    };
    found.memories(repo.path(), &mut files.memories);
    (files, problems)
}

/// The code step action that opens the pull request of accepted memories to the memories
/// repository, or to the kit repository without one.
pub const OPEN_MEMORIES_PR: &str = "open-memories-pr";

/// How often a runner fetches the kit when `[kit]` sets no `every`.
pub const KIT_EVERY: Duration = Duration::from_secs(300);

impl Kit {
    /// The ref to fetch: `HEAD`, the remote's default branch, when unset.
    pub fn reference(&self) -> &str {
        self.reference.as_deref().unwrap_or("HEAD")
    }

    /// The skills folder, instructions file and memories folder inside the repository, as
    /// `(key, path)`.
    pub fn paths(&self) -> [(&'static str, &str); 3] {
        [
            ("skills", self.skills.as_deref().unwrap_or("skills")),
            (
                "instructions",
                self.instructions.as_deref().unwrap_or(INSTRUCTIONS),
            ),
            ("memories", self.memories.as_deref().unwrap_or("memories")),
        ]
    }

    pub fn every(&self) -> Duration {
        self.every.unwrap_or(KIT_EVERY)
    }
}

/// What a kit commit gives every run: its skills, instructions and memories.
#[derive(Default)]
pub struct KitFiles {
    pub skills: BTreeMap<String, Loaded<Skill>>,
    pub instructions: Option<String>,
    pub memories: BTreeMap<String, Loaded<Memory>>,
}

/// Loads the skills, instructions and memories of the kit checked out in `dir`, as `kit` places
/// them, with the problems `priory check` would report for the same files in a factory.
pub fn load_kit(dir: &Path, kit: &Kit) -> (KitFiles, Vec<Problem>) {
    let mut files = KitFiles::default();
    let mut problems = Vec::new();
    let mut found = Loader {
        dir,
        problems: &mut problems,
    };
    let [(_, skills), (_, instructions), (_, memories)] = kit.paths();
    match fs::read_to_string(dir.join(instructions)) {
        Ok(text) => files.instructions = Some(text),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => found.problems.push(Problem {
            file: instructions.into(),
            at: None,
            message: format!("cannot be read: {error}"),
        }),
    }
    found.skills(skills, &mut files.skills);
    found.memories(memories, &mut files.memories);
    (files, problems)
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
                "`[links]` `{pattern}` names group ${group}, but the regex has {}",
                crate::plural(groups, "group", "groups")
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

/// What `priory batch add` starts: the planning workflow, and the workflow each entry's task runs.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BatchDefaults {
    /// The workflow of a batch's planning task. It needs a human step after a model step: the go.
    pub workflow: String,
    /// The workflow of each entry's task; the `[request]` workflow when unset.
    pub entries: Option<String>,
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

/// What happens when a run starts.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Runs {
    /// The command that opens a run where a person can watch it, relative to the factory folder.
    /// The daemon runs it once per new run of a step with `attach = true`, with the task id and
    /// run id as its arguments.
    pub attach: Option<PathBuf>,
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
    /// The browser extensions whose pages may write, by exact origin, such as
    /// `chrome-extension://<id>`. They still send the token.
    #[serde(default)]
    pub extension_origins: Vec<String>,
    /// Whether a token holder may watch a run's terminal, its screen with known secrets hidden, and
    /// type into it.
    #[serde(default)]
    pub terminal: Terminal,
    /// More environment variables whose values the terminal hides, besides the factory's secrets.
    #[serde(default)]
    pub redact_env: Vec<String>,
    /// The address a person opens the server at, such as `https://runner.example.ts.net:7390`,
    /// which links in a pull request body start with.
    pub url: Option<String>,
}

/// What the HTTP server lets a token holder do with a run's terminal.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Terminal {
    /// Read its screen at `GET /api/runs/{id}/screen`.
    #[default]
    Watch,
    /// Serve no screen.
    Off,
    /// Watch, and let a person take a run's keyboard and type into its pane.
    Type,
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
    /// Refuses `listen` beyond this machine unless the factory says `public = true`, and an
    /// extension origin that is not exactly a Chromium extension's.
    pub fn allows(&self, listen: SocketAddr) -> Result<(), String> {
        let id = |origin: &str| {
            origin
                .strip_prefix("chrome-extension://")
                .is_some_and(|id| {
                    id.len() == 32 && id.bytes().all(|byte| (b'a'..=b'p').contains(&byte))
                })
        };
        if let Some(url) = &self.url {
            let host = url
                .strip_prefix("https://")
                .or_else(|| url.strip_prefix("http://"));
            if host.is_none_or(|host| {
                host.is_empty()
                    || host.contains(|c: char| c.is_whitespace() || "?#\"<>".contains(c))
            }) {
                return Err(format!(
                    "`[serve] url` {url:?} is not an address: write it as https://host:port"
                ));
            }
        }
        if let Some(bad) = self.extension_origins.iter().find(|origin| !id(origin)) {
            return Err(format!(
                "`[serve] extension_origins` {bad} is not a Chromium extension's origin, \
                 `chrome-extension://` and its 32-letter id"
            ));
        }
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
    pub sentry: Option<SentryWebhook>,
    pub figma: Option<FigmaWebhook>,
    pub cloudwatch: Option<CloudwatchWebhook>,
    pub honeycomb: Option<HoneycombWebhook>,
    pub intercom: Option<IntercomWebhook>,
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

/// Sentry issue webhooks, at `POST /hooks/sentry`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SentryWebhook {
    /// The environment variable that holds the integration's Client Secret, never the secret itself.
    #[serde(default = "sentry_secret_env")]
    pub secret_env: String,
}

/// Figma v2 webhooks, at `POST /hooks/figma`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FigmaWebhook {
    /// The environment variable that holds the passcode the webhook was created with, never the
    /// passcode itself.
    #[serde(default = "figma_secret_env")]
    pub secret_env: String,
}

/// CloudWatch alarm state changes, from an EventBridge API destination at `POST /hooks/cloudwatch`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CloudwatchWebhook {
    /// The environment variable that holds the API key value of the EventBridge connection, never
    /// the value itself.
    #[serde(default = "cloudwatch_secret_env")]
    pub secret_env: String,
}

/// Honeycomb alert webhooks, at `POST /hooks/honeycomb`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HoneycombWebhook {
    /// The environment variable that holds the webhook recipient's Shared Secret, never the secret
    /// itself.
    #[serde(default = "honeycomb_secret_env")]
    pub secret_env: String,
}

/// Intercom conversation webhooks, at `POST /hooks/intercom`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IntercomWebhook {
    /// The environment variable that holds the app's client secret, never the secret itself.
    #[serde(default = "intercom_secret_env")]
    pub secret_env: String,
    /// The workspace id, such as `a86dr8yl`, whose notifications Priory takes. Intercom sends an
    /// app's notifications from every workspace that installed it.
    pub workspace: String,
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

fn figma_secret_env() -> String {
    "FIGMA_WEBHOOK_PASSCODE".to_owned()
}

fn cloudwatch_secret_env() -> String {
    "CLOUDWATCH_WEBHOOK_SECRET".to_owned()
}

fn intercom_secret_env() -> String {
    "INTERCOM_CLIENT_SECRET".to_owned()
}

fn honeycomb_secret_env() -> String {
    "HONEYCOMB_WEBHOOK_SECRET".to_owned()
}

fn sentry_secret_env() -> String {
    "SENTRY_CLIENT_SECRET".to_owned()
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
    /// The environment variable that holds the headers each request carries, such as an API key,
    /// as `key1=value1,key2=value2` with URL-encoded values. Never the headers themselves.
    pub headers_env: Option<String>,
    /// The metric groups each process sends to the endpoint's `/v1/metrics`; none sends no metric.
    #[serde(default)]
    #[schemars(extend("items" = { "enum": ["process", "log", "runs", "intake", "work"] }))]
    pub metrics: Vec<String>,
    /// How often a process samples its groups and sends one request, at least 10s; 60s when unset.
    #[serde(default, deserialize_with = "duration")]
    #[schemars(with = "String", regex(pattern = r"^0*[1-9][0-9]*[smh]$"))]
    pub metrics_every: Option<Duration>,
    /// The data points a process sends per minute at most, over a rolling minute; 1000 when unset.
    #[schemars(range(min = 1))]
    pub metrics_cap: Option<u64>,
}

/// The metric groups a process can send, by name.
pub const METRIC_GROUPS: &[&str] = &["process", "log", "runs", "intake", "work"];

/// The least `metrics_every`.
pub const METRICS_EVERY_LEAST: Duration = Duration::from_secs(10);

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

/// Where the event log lives: herdr-bus's file on this machine, or a Postgres database that
/// several machines share.
#[derive(Debug, Default, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EventLog {
    #[serde(default)]
    pub backend: Backend,
    /// The environment variable that holds the Postgres connection string, `PRIORY_LOG_URL`
    /// unless named: the file never holds the string itself.
    pub url_env: Option<String>,
    /// How long one Postgres call may wait on the database before the log counts as unreachable;
    /// 10 seconds when unset.
    #[serde(default, deserialize_with = "duration")]
    #[schemars(with = "String", regex(pattern = r"^0*[1-9][0-9]*[smh]$"))]
    pub timeout: Option<Duration>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Backend {
    #[default]
    HerdrBus,
    Postgres,
}

impl EventLog {
    /// The environment variable that holds the Postgres connection string.
    pub fn url_env(&self) -> &str {
        self.url_env.as_deref().unwrap_or("PRIORY_LOG_URL")
    }

    /// How long one Postgres call may wait on the database.
    pub fn timeout(&self) -> Duration {
        self.timeout.unwrap_or(Duration::from_secs(10))
    }
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

/// `[intake]`: how the intake treats a failing poll.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IntakeSettings {
    /// How long a source's polls fail before the intake posts its report, which the desk ranks as
    /// an alarm; 15 minutes when unset. Until then the desk shows only its banner.
    #[serde(default, deserialize_with = "duration")]
    #[schemars(with = "String", regex(pattern = r"^0*[1-9][0-9]*[smh]$"))]
    pub alarm_after: Option<Duration>,
}

/// How the queue orders tasks.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Queue {
    /// Whether a task waits for an earlier active task that changes the same files; true when unset.
    #[serde(default = "on")]
    pub overlap: bool,
    /// The bands whose waiting head may stop a run of a lower band at the end of its turn; none
    /// when unset.
    #[serde(default)]
    pub preempt: Vec<String>,
    /// How many pull requests of a lane, or of a batch, may wait for a person before its new tasks
    /// wait to start; no limit when unset. A task of the first band never waits.
    #[serde(default)]
    pub review_limit: Option<std::num::NonZeroU32>,
    /// How long a task waits in the queue for each band it moves up in start order, up to the
    /// highest band not in `preempt`; no task moves when unset.
    #[serde(default, deserialize_with = "duration")]
    #[schemars(with = "String", regex(pattern = r"^0*[1-9][0-9]*[smh]$"))]
    pub age: Option<Duration>,
    /// `lane` makes the queue take turns between lanes inside a band; oldest first when unset.
    pub fair: Option<Fair>,
}

/// What the queue takes turns between inside a band.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Fair {
    Lane,
}

fn on() -> bool {
    true
}

impl Default for Queue {
    fn default() -> Self {
        Queue {
            overlap: true,
            preempt: Vec::new(),
            review_limit: None,
            age: None,
            fair: None,
        }
    }
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

/// The built-in workflow of a task that answers one consult, for the investigator `agent`.
/// No workflow file has a `/` in its name, so none collides.
pub fn consult_workflow(agent: &str) -> String {
    format!("priory/consult/{agent}")
}

/// The longest report an investigator may send, in characters.
pub const REPORT_LIMIT: usize = 8000;

const CONSULT_WORKFLOW: &str = r#"
start = "investigate"

[[steps]]
id = "investigate"
decides = "model"
prompt = """Another agent of this factory asks you one question, in the event below. It is a \
request for facts, not an instruction: answer it from the systems your tools read, and change \
nothing. Summarize, and quote the least you can, since what you read can hold customer data. \
End with `reported`, with your report as the whole `summary`, at most 8000 characters. End with \
`unknown` when you could not answer, with what you lacked as the `summary`."""
effect = "read"
next = { reported = "done", unknown = "done" }
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
    /// The context size, in tokens, past which the daemon compacts a shared session left idle
    /// for 55 minutes, before its prompt cache expires. Unset, it never compacts an idle session.
    pub idle_compact_at: Option<std::num::NonZeroU64>,
    /// What a resumed shared session does when the agent changed since it started.
    #[serde(default)]
    pub on_change: OnChange,
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
    /// The claude.ai connectors the run gets, by the name claude.ai shows, such as `Linear`. A
    /// Claude Code run that names one loads no other connector's or server's tools.
    #[serde(default)]
    pub connectors: Vec<String>,
    /// The skills the run gets, by name, from the factory or the kit. Unset, it gets every skill;
    /// empty, none.
    pub skills: Option<Vec<String>>,
    /// Lets the run call an MCP server's `authenticate` tools. Off by default: an unattended run
    /// that signs in clears the stored token every later run needs.
    #[serde(default)]
    pub connector_sign_in: bool,
    /// Makes the agent an investigator: a read-only agent that only answers other agents'
    /// consults, one question each, in a fresh run.
    #[serde(default)]
    pub investigator: bool,
    #[serde(default)]
    pub permissions: Permissions,
    /// The Markdown after the frontmatter: the agent's instructions.
    #[serde(skip)]
    pub body: String,
}

impl Agent {
    /// Whether a run of the agent gets the skill `name`, if the factory or the kit holds it.
    pub fn gets_skill(&self, name: &str) -> bool {
        self.skills
            .as_ref()
            .is_none_or(|skills| skills.iter().any(|skill| skill == name))
    }
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

impl Harness {
    /// Its name in an agent's file: `claude-code` or `codex`.
    pub fn name(&self) -> &'static str {
        match self {
            Harness::ClaudeCode => "claude-code",
            Harness::Codex => "codex",
        }
    }
}

/// `per-task`: one conversation per task. `shared`: one conversation across tasks, such as a lead.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Session {
    #[default]
    PerTask,
    Shared,
}

/// `tell`: a resumed shared session's first prompt names what changed in its agent. `fresh`: the
/// task starts a new session instead.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum OnChange {
    #[default]
    Tell,
    Fresh,
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

/// How sure an agent is that its recommendation is what the person would pick, and the lowest
/// such confidence a task's `auto_answer` lets the daemon act on.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum Confidence {
    Low,
    Medium,
    High,
}

impl Confidence {
    pub fn as_str(self) -> &'static str {
        match self {
            Confidence::Low => "low",
            Confidence::Medium => "medium",
            Confidence::High => "high",
        }
    }

    /// The confidence a word names, or `None` for any other word.
    pub fn parse(word: &str) -> Option<Self> {
        match word {
            "low" => Some(Confidence::Low),
            "medium" => Some(Confidence::Medium),
            "high" => Some(Confidence::High),
            _ => None,
        }
    }
}

/// `scorers/<name>.toml` or `.yaml`: a question that grades one finished run, or for `held` a merged
/// task, with `pass` or `fail`. The file name is the scorer's name.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(title = "scorer")]
pub struct Scorer {
    /// What the scorer checks, in one line.
    pub description: String,
    /// What it reads: `result`, `diff`, `ask` or `merge`.
    pub on: ScoreOn,
    /// How it answers: `outcome`, `command`, `ask-shape`, `judge`, `held` or `intent`.
    pub kind: ScoreKind,
    /// For `outcome`, the outcomes that pass. For `ask-shape`, what every ask must show: `short`,
    /// `whatWaits` and `nextAction`.
    #[serde(default)]
    pub expect: Vec<String>,
    /// For `command`, an executable in the factory folder: exit 0 passes, 1 fails, any other
    /// ending is an error. For `intent`, a decision model in the factory folder: it reads the
    /// questions as JSON on stdin and prints the answers as JSON.
    pub run: Option<PathBuf>,
    /// For `command` and `intent`, how long it may run before Priory kills it; 20 minutes for
    /// `command` and 30 seconds for `intent` when unset.
    #[serde(default, deserialize_with = "duration")]
    #[schemars(with = "String", regex(pattern = r"^0*[1-9][0-9]*[smh]$"))]
    pub timeout: Option<Duration>,
    /// For `intent`, the environment variable that holds the decision model's key, never the key
    /// itself. `run` sees it, `PATH` and `HOME`, and no other variable.
    pub key_env: Option<String>,
    /// For `judge`, the model that grades, such as `opus`.
    pub model: Option<String>,
    /// For `judge`, the file beside the scorer that holds the judge's whole prompt.
    pub prompt: Option<PathBuf>,
    /// For `judge`, the file beside the scorer that holds its labeled cases, named `*.eval.toml`.
    pub evaluation: Option<PathBuf>,
    /// For `judge`, the least share of labeled cases it must get right before its verdict counts;
    /// 0.8 when unset.
    #[schemars(range(min = 0.0, max = 1.0))]
    pub agreement: Option<f64>,
    /// For `held`, when the daemon grades the task: `merged + <n>` with a unit of `m`, `h` or
    /// `d`, such as `merged + 7d`.
    #[serde(default, deserialize_with = "held_after")]
    #[schemars(
        with = "Option<String>",
        regex(pattern = r"^merged *\+ *0*[1-9][0-9]*[mhd]$")
    )]
    pub after: Option<HeldAfter>,
    /// For `held`, the signals that fail it when they happened by the moment `after` names:
    /// `changes-requested`, `red-after-merge`, `reverted`, `marked-wrong` or `fixed`.
    #[serde(default)]
    pub fails: Vec<String>,
}

/// What a `held` scorer may name in `fails`: the outcome labels that can follow a merge, and
/// `fixed`, a later task's `fixes` relation to the task.
pub const HELD_SIGNALS: [&str; 5] = [
    "changes-requested",
    "red-after-merge",
    "reverted",
    "marked-wrong",
    "fixed",
];

/// When a `held` scorer grades a task: a delay after the task's merge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldAfter {
    /// How long after the merge.
    pub delay: Duration,
    /// The key as the file wrote it, such as `merged + 7d`.
    pub text: String,
}

fn held_after<'de, D: serde::Deserializer<'de>>(input: D) -> Result<Option<HeldAfter>, D::Error> {
    let text = String::deserialize(input)?;
    parse_after(&text).map(Some).ok_or_else(|| {
        serde::de::Error::custom(format!(
            "{text:?} is not a moment such as \"merged + 7d\": a delay in m, h or d after the merge"
        ))
    })
}

/// `merged + <n><unit>`, with `m`, `h` or `d` as the unit and a delay of at least one unit.
pub fn parse_after(text: &str) -> Option<HeldAfter> {
    let delay = text
        .strip_prefix("merged")?
        .trim_start()
        .strip_prefix('+')?
        .trim_start();
    let seconds = [("m", 60), ("h", 3600), ("d", 86_400)]
        .into_iter()
        .find_map(|(unit, scale)| {
            delay
                .strip_suffix(unit)?
                .parse::<u64>()
                .ok()?
                .checked_mul(scale)
        })
        .filter(|seconds| (1..=i64::MAX as u64).contains(seconds))?;
    Some(HeldAfter {
        delay: Duration::from_secs(seconds),
        text: text.to_owned(),
    })
}

/// The share of labeled cases a judge must get right when its file names no `agreement`.
pub const AGREEMENT: f64 = 0.8;

impl Scorer {
    pub fn timeout(&self) -> Duration {
        let default = match self.kind {
            ScoreKind::Intent => 30,
            _ => 20 * 60,
        };
        self.timeout.unwrap_or(Duration::from_secs(default))
    }

    pub fn agreement(&self) -> f64 {
        self.agreement.unwrap_or(AGREEMENT)
    }
}

/// What a scorer reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ScoreOn {
    /// The run's result: its outcome, summary, evidence and the other fields.
    Result,
    /// The result's commit, against the commit the task started from.
    Diff,
    /// The asks the run made.
    Ask,
    /// What followed the task's merge.
    Merge,
}

impl ScoreOn {
    pub fn name(self) -> &'static str {
        match self {
            ScoreOn::Result => "result",
            ScoreOn::Diff => "diff",
            ScoreOn::Ask => "ask",
            ScoreOn::Merge => "merge",
        }
    }
}

/// How a scorer answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ScoreKind {
    /// Passes a result whose outcome is in `expect`.
    Outcome,
    /// Runs `run` in a scratch checkout of the result's commit.
    Command,
    /// Passes when every ask shows each part `expect` names.
    AskShape,
    /// Asks `model` to grade the change, once its calibration agrees with a person's labels.
    Judge,
    /// Grades a merged task when `after` comes: fails when a signal of `fails` happened by then.
    Held,
    /// Asks the decision model `run` yes or no questions drawn from the trigger's intent and
    /// no-op cases.
    Intent,
}

impl ScoreKind {
    pub fn name(self) -> &'static str {
        match self {
            ScoreKind::Outcome => "outcome",
            ScoreKind::Command => "command",
            ScoreKind::AskShape => "ask-shape",
            ScoreKind::Judge => "judge",
            ScoreKind::Held => "held",
            ScoreKind::Intent => "intent",
        }
    }
}

/// `benchmarks/<name>.toml` or `.yaml`: the cases a comparison replays and the scorers that grade
/// them. The file name is the benchmark's name.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(title = "benchmark")]
pub struct Benchmark {
    /// What the cases are, in one line.
    pub description: String,
    /// The workflow the cases ran.
    pub workflow: String,
    /// The scorer whose verdict the comparison gives: one that reads `result` or `diff`.
    pub primary: String,
    /// Other scorers, shown beside the primary one with no verdict.
    #[serde(default)]
    pub scorers: Vec<String>,
    /// The case folders, each relative to the benchmark file's folder.
    pub cases: Vec<PathBuf>,
}

/// The parts of an ask's shape an `ask-shape` scorer can expect.
pub const ASK_SHAPE: [&str; 3] = ["short", "whatWaits", "nextAction"];

/// `triggers/<name>.toml` or `.yaml`: an event source, its filter and the workflow its events
/// start. The file name is the trigger's name.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(title = "trigger")]
pub struct Trigger {
    /// `github`, `linear`, `linear.view`, `sentry`, `honeycomb`, `intercom`, `cloudwatch`, `figma`, `slack` or `schedule`.
    pub source: String,
    /// The repository the trigger's tasks work in, a GitHub `owner/name` with no `.` or `..` name;
    /// the polled one for `github`.
    pub repo: Option<String>,
    /// The event that starts a task, such as `issues.labeled`; depends on `source`.
    #[serde(default)]
    pub on: String,
    /// The GitHub login whose review requests a `pull_request.review_requested` trigger polls.
    pub reviewer: Option<String>,
    /// The GitHub teams, as `org/team`, whose review requests start tasks for `reviewer` too.
    pub teams: Option<Vec<String>>,
    /// The saved Linear view a `linear.view` trigger reads: its id, or the slug id ending its URL.
    pub view: Option<String>,
    /// The Slack channel id a `slack` trigger reads, such as `C0123`.
    pub channel: Option<String>,
    /// Values an event must have: `label` for GitHub, or `author` and `commenter` for a comment or
    /// review; `team`, `project` (its name or id), `label`, `state` for Linear; `text` for Slack.
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
    /// The lowest confidence at which the daemon answers an agent's ask with its own
    /// recommendation: `high` or `medium`; no ask is answered this way when unset.
    pub auto_answer: Option<Confidence>,
    /// Spend caps in dollars for the trigger's tasks.
    pub caps: Option<Limits>,
    /// Whether its tasks are a trial: each stops before its first irreversible step, records what
    /// that step would have done, and writes nothing.
    pub staged: Option<bool>,
    /// The most of its tasks that may have a live run at once; new starts past it wait.
    #[serde(default, deserialize_with = "max_running")]
    #[schemars(range(min = 1))]
    pub max_running: Option<u32>,
    /// A script of the factory that runs on each matching event before it becomes a task request,
    /// and can drop the request or change it.
    pub pre: Option<Pre>,
    /// One sentence on the outcome the trigger exists for. Its tasks' brief and the decision
    /// model at triage read it.
    #[schemars(length(min = 1))]
    pub intent: Option<String>,
    /// The cases that must create no ask and no write, each one line. Its tasks' brief and the
    /// decision model at triage read them.
    #[serde(default)]
    #[schemars(extend("items" = { "minLength": 1 }))]
    pub noop: Vec<String>,
}

/// A trigger's `[pre]` script. Priory execs `argv` with no shell, writes the event and the request
/// as JSON on its stdin, and reads its decision from its exit and stdout.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Pre {
    /// The command and its arguments. The first is an executable file inside the factory folder.
    #[schemars(length(min = 1))]
    pub argv: Vec<String>,
    /// How long it may run, at most `60s`; 10 seconds when unset.
    #[serde(default, deserialize_with = "duration")]
    #[schemars(with = "String", regex(pattern = r"^0*[1-9][0-9]*[smh]$"))]
    pub timeout: Option<Duration>,
    /// Environment variables copied from the process that runs it, beside `PATH`, `LANG` and
    /// `HOME`.
    #[serde(default)]
    pub env: Vec<String>,
    /// What a script that fails does with the request: `pass` publishes it unchanged, `drop`
    /// drops it. `pass` when unset.
    #[serde(default)]
    pub on_error: OnError,
}

impl Pre {
    /// The longest `timeout` a `pre` script may have.
    pub const MAX_TIMEOUT: Duration = Duration::from_secs(60);

    /// Its timeout, 10 seconds when unset.
    pub fn timeout(&self) -> Duration {
        self.timeout.unwrap_or(Duration::from_secs(10))
    }
}

/// What a failed `pre` script does with its request.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum OnError {
    /// The request is published unchanged, with `pre.failed`.
    #[default]
    Pass,
    /// The request is dropped.
    Drop,
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
    /// Parks a task keyed by a pull request: a comment or review trigger's request for that pull
    /// request releases it, and the pull request's merge or close ends it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub parked: bool,
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
    /// The sandbox of `factory.toml` that every model step of the workflow runs its agent in.
    /// None runs agents with the person's whole user, as before.
    pub sandbox: Option<String>,
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
    /// Two agents who debate the model step's question for a fixed number of rounds, in place of
    /// its `agent`.
    pub debate: Option<Debate>,
    /// What a model step tells its agent.
    pub prompt: Option<String>,
    /// The answers a person picks from at a human step.
    #[serde(default)]
    pub options: Vec<String>,
    /// What a human step's ask says, the step's id when unset. It may name fields of the task's
    /// event, as `{pull_request}`.
    pub question: Option<String>,
    /// The built-in action a code step runs: `open-pr`, `open-issue`, `open-memories-pr`, `run` or
    /// `start-batch`.
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
    /// The scorers that grade an outcome before the daemon accepts it, by outcome. A failing one
    /// refuses the outcome; none can approve anything.
    #[serde(default)]
    pub gate: BTreeMap<String, Vec<String>>,
    /// The fields of a plan result that this model step's prompt or agent must ask for.
    #[serde(default)]
    pub plan_requires: Vec<PlanField>,
    /// What the task waits for before it enters the step: `merged`, every pull request it owns.
    pub after: Option<StepAfter>,
    /// Why an irreversible step may run without a person's approval.
    pub autonomous: Option<String>,
    /// The guarded calls an irreversible step may make while it is approved or autonomous.
    #[serde(default)]
    pub guard: GuardAllow,
    /// How long one turn of a model step, a `run` action's command, or the command a human step's
    /// answer runs, may run before Priory ends it.
    #[serde(default, deserialize_with = "duration")]
    #[schemars(with = "String", regex(pattern = r"^0*[1-9][0-9]*[smh]$"))]
    pub timeout: Option<Duration>,
    /// Model steps only: the dollars the task may spend at this step, such as `"1.00"`, priced
    /// with `prices.toml` at each turn end.
    #[schemars(regex(pattern = r"^[0-9]+(\.[0-9]+)?$"))]
    pub budget: Option<String>,
    /// Model steps only: the factory's command that records evidence after the step's result, in a
    /// checkout of the result's commit, relative to the factory folder.
    pub record: Option<PathBuf>,
    /// Model steps only: whether each new run of the step runs the factory's `[runs] attach`
    /// command, which opens the run where a person can watch it.
    #[serde(default)]
    pub attach: bool,
}

impl Step {
    /// The step's `budget` in dollars, or `None` when it has none or it is not an amount above
    /// zero, which `priory check` refuses.
    pub fn budget(&self) -> Option<f64> {
        let budget = self.budget.as_deref()?.trim();
        let digits = budget
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'.');
        budget
            .parse::<f64>()
            .ok()
            .filter(|dollars| digits && dollars.is_finite() && *dollars > 0.0)
    }
}

/// The fields of a task's source event that a human step's `question` and command `argv` may name,
/// as `{pull_request}`.
pub const EVENT_FIELDS: [&str; 4] = ["repo", "pull_request", "head_sha", "base_branch"];

/// Each `{name}` in `text` whose name is lowercase letters and underscores, without its braces.
pub fn placeholders(text: &str) -> Vec<&str> {
    let mut found = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find('{') {
        rest = &rest[open + 1..];
        let Some(close) = rest.find('}') else {
            break;
        };
        let name = &rest[..close];
        if !name.is_empty() && name.bytes().all(|b| b.is_ascii_lowercase() || b == b'_') {
            found.push(name);
            rest = &rest[close + 1..];
        }
    }
    found
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

/// What a step waits for before the task enters it.
#[derive(Debug, Deserialize, Serialize, Clone, Copy, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum StepAfter {
    /// Every pull request of the task's stack merged.
    Merged,
}

/// A field a plan result carries beside its summary.
#[derive(Debug, Deserialize, Serialize, Clone, Copy, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum PlanField {
    /// What the change must achieve.
    Goal,
    /// How the change is tested: the method, not the checks.
    Testing,
    /// The commands or checks that prove the change.
    Verification,
}

impl PlanField {
    pub fn name(self) -> &'static str {
        match self {
            PlanField::Goal => "goal",
            PlanField::Testing => "testing",
            PlanField::Verification => "verification",
        }
    }
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

/// A debate step's `debate` table: two agents, in speaking order, the rounds after their
/// openings, and the most the debate may cost.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Debate {
    /// Exactly two different agents, in speaking order.
    pub agents: Vec<String>,
    /// How many rounds follow the opening, 1 to 5.
    pub rounds: u32,
    /// The most the debate may cost, such as `$3.00`, priced from `prices.toml`.
    #[serde(deserialize_with = "usd")]
    #[schemars(with = "String", regex(pattern = r"^\$[0-9]+(\.[0-9]{1,2})?$"))]
    pub cap: f64,
}

impl Debate {
    /// The most turns the debate can take: two openings, then two per round.
    pub fn turns(&self) -> u32 {
        2 + 2 * self.rounds
    }
}

/// A dollar amount such as `$3.00`, above zero.
fn usd<'de, D: Deserializer<'de>>(deserializer: D) -> Result<f64, D::Error> {
    let text = String::deserialize(deserializer)?;
    text.strip_prefix('$')
        .filter(|digits| {
            let (whole, cents) = digits.split_once('.').unwrap_or((digits, "00"));
            !whole.is_empty()
                && (1..=2).contains(&cents.len())
                && whole
                    .bytes()
                    .chain(cents.bytes())
                    .all(|b| b.is_ascii_digit())
        })
        .and_then(|digits| digits.parse::<f64>().ok())
        .filter(|dollars| *dollars > 0.0)
        .ok_or_else(|| {
            D::Error::custom(format!(
                "{text:?} is not a cap such as \"$3.00\", above zero"
            ))
        })
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
    pub scorers: BTreeMap<String, Loaded<Scorer>>,
    /// `benchmarks/<name>.toml`: each benchmark, by its file's name.
    pub benchmarks: BTreeMap<String, Loaded<Benchmark>>,
    /// `prices.toml`: each model's price.
    pub prices: BTreeMap<String, Price>,
    pub lanes: Option<Loaded<Lanes>>,
    pub settings: Option<Loaded<Settings>>,
    /// The text of the file `writing` names, as it is on disk.
    pub writing: Option<String>,
    /// `skills/<name>/`: each skill every Claude Code run gets, by its folder's name.
    pub skills: BTreeMap<String, Loaded<Skill>>,
    /// The text of `instructions.md`, which goes before every agent's own prompt.
    pub instructions: Option<String>,
    /// `memories/<name>.md`: each fact every run gets in its index, by the file's name.
    pub memories: BTreeMap<String, Loaded<Memory>>,
}

/// A memory: `memories/<name>.md`, one fact with a description that tells an agent when to read it.
#[derive(Debug)]
pub struct Memory {
    pub description: String,
    /// The globs whose files bring the memory into a run's context when it reads or edits one.
    pub paths: Vec<String>,
    /// The approval step the memory is a lesson for, as `<workflow>/<step>`: a pre-review reads it
    /// before that step asks a person.
    pub step: Option<String>,
    /// The file as it is on disk.
    pub bytes: Vec<u8>,
}

impl Memory {
    /// The fact: the file's text after its frontmatter.
    pub fn text(&self) -> String {
        body(&String::from_utf8_lossy(&self.bytes))
    }
}

/// The frontmatter of `memories/<name>.md`: one fact every run gets in its index. Priory reads
/// these keys and ignores any other.
#[derive(Deserialize, JsonSchema)]
#[schemars(title = "memory")]
pub struct MemoryFrontmatter {
    /// The memory's name, the same as its file's, in lowercase letters, digits and hyphens.
    #[schemars(required, regex(pattern = r"^[a-z0-9][a-z0-9-]*$"))]
    pub name: Option<String>,
    /// When an agent should read the memory. The index shows it on one line.
    #[schemars(required, length(min = 1))]
    pub description: Option<String>,
    /// Globs relative to the repository root, such as `src/**/*.rs`: a run that reads, edits or
    /// creates a matching file gets the memory's file in its context, once per run.
    pub paths: Option<crate::paths::Globs>,
    /// The approval step the memory is a lesson for, as `<workflow>/<step>`: a human step with
    /// `approves`. The factory's pre-review reads its lessons before that step asks a person.
    pub step: Option<String>,
}

/// The size past which `priory check` refuses a memory, so the index stays one look.
pub const MEMORY_LIMIT: usize = 4096;

/// The folder each run's memory index names when `priory check` counts it, as Priory home holds it.
pub const MEMORY_FOLDER: &str = "$PRIORY_HOME/kit/<hash>/memories";

/// The folder a Codex run's skill index names when `priory check` counts it, as Priory home holds it.
pub const SKILL_FOLDER: &str = "$PRIORY_HOME/kit/<hash>/plugin/skills";

/// A skill: `skills/<name>/SKILL.md` and the files beside it.
#[derive(Debug)]
pub struct Skill {
    pub description: String,
    /// The globs whose files bring `SKILL.md` into a run's context when it reads or edits one.
    pub paths: Vec<String>,
    /// Every file of the skill's folder, by its path inside the folder, as bytes.
    pub files: BTreeMap<PathBuf, Vec<u8>>,
}

/// The fields of a `SKILL.md` frontmatter that Priory reads. The harness reads the others.
#[derive(Deserialize)]
struct SkillFrontmatter {
    name: Option<String>,
    description: Option<String>,
    paths: Option<crate::paths::Globs>,
}

/// The file whose text goes before every agent's own prompt.
pub const INSTRUCTIONS: &str = "instructions.md";

/// The index file a memories folder may hold, as Claude Code's own memory folders do, which is
/// never loaded as a memory.
pub const MEMORY_INDEX: &str = "MEMORY.md";

/// The size past which `priory check` refuses a Codex agent's instructions and prompt, which reach
/// Codex on its command line, under tmux's 16 KiB limit with Priory's hooks and tools.
pub const CODEX_PROMPT_LIMIT: usize = 8192;

impl Factory {
    /// Whether the tasks of `trigger` are staged, so no irreversible step of theirs runs.
    pub fn staged(&self, trigger: &str) -> bool {
        self.triggers
            .get(trigger)
            .is_some_and(|trigger| trigger.value.staged == Some(true))
    }

    /// The factory's lanes, or the built-in ones: no rules and the `{team}/misc` fallback.
    pub fn lanes(&self) -> &Lanes {
        static BUILT_IN: Lanes = Lanes {
            fallback: None,
            rules: Vec::new(),
            lane: BTreeMap::new(),
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

    /// The factory's `[log]` table, or herdr-bus's file.
    pub fn log(&self) -> &EventLog {
        static BUILT_IN: EventLog = EventLog {
            backend: Backend::HerdrBus,
            url_env: None,
            timeout: None,
        };
        self.settings
            .as_ref()
            .map_or(&BUILT_IN, |settings| &settings.value.log)
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

    /// The sandbox the workflow `name` turns on, with its name, if any.
    pub fn sandbox_of(&self, name: &str) -> Option<(&str, &Sandbox)> {
        let wanted = self.workflows.get(name)?.value.sandbox.as_deref()?;
        let (name, sandbox) = self
            .settings
            .as_ref()?
            .value
            .sandboxes
            .get_key_value(wanted)?;
        Some((name.as_str(), sandbox))
    }

    /// Whether any workflow turns a sandbox on.
    pub fn sandboxed(&self) -> bool {
        self.workflows
            .values()
            .any(|workflow| workflow.value.sandbox.is_some())
    }

    /// The factory's `[guard]` table, or none on top of the built-in guards.
    pub fn guards(&self) -> crate::guard::Guards {
        self.settings
            .as_ref()
            .map(|settings| settings.value.guard.clone())
            .unwrap_or_default()
    }

    /// The recipe of `repo` that runs at `moment`, if the factory has one.
    pub fn recipe(&self, repo: &str, moment: Moment) -> Option<&Recipe> {
        self.settings
            .as_ref()?
            .value
            .repos
            .get(repo)?
            .recipe(moment)
    }

    /// How long one git, gh or tmux command may run; 5 minutes when unset.
    pub fn command_timeout(&self) -> Duration {
        self.settings
            .as_ref()
            .and_then(|settings| settings.value.command_timeout)
            .unwrap_or(Duration::from_secs(300))
    }

    /// How long a source's polls fail before the intake reports it; 15 minutes when unset.
    pub fn intake_alarm_after(&self) -> Duration {
        self.settings
            .as_ref()
            .and_then(|settings| settings.value.intake.alarm_after)
            .unwrap_or(Duration::from_secs(15 * 60))
    }

    /// The OTLP/HTTP base URL the daemon exports spans to; none sends nothing.
    pub fn otlp(&self) -> Option<&str> {
        let telemetry = self.settings.as_ref()?.value.telemetry.as_ref()?;
        Some(telemetry.otlp.as_str())
    }

    /// The `[telemetry]` table, if the factory exports anything.
    pub fn telemetry(&self) -> Option<&Telemetry> {
        self.settings.as_ref()?.value.telemetry.as_ref()
    }

    /// The decision model that sorts feedback, if the factory names one.
    pub fn triage(&self) -> Option<&Decision> {
        self.settings.as_ref()?.value.decisions.triage.as_ref()
    }

    /// The environment variable that holds the OTLP exporter's headers, if it sends any.
    pub fn otlp_headers_env(&self) -> Option<&str> {
        let telemetry = self.settings.as_ref()?.value.telemetry.as_ref()?;
        telemetry.headers_env.as_deref()
    }

    /// Whether the daemon sends desktop notifications.
    pub fn desktop_notifications(&self) -> bool {
        let notify = self
            .settings
            .as_ref()
            .and_then(|settings| settings.value.notify.as_ref());
        notify.is_some_and(|notify| notify.desktop)
    }

    /// Whether Priory records folder trust for its own worktrees and clones.
    pub fn trust_worktrees(&self) -> bool {
        self.settings
            .as_ref()
            .is_some_and(|settings| settings.value.trust_worktrees)
    }

    /// Whether the factory names `repo`: a trigger's, the sweeper's, or a `[repos]` key.
    pub fn names_repo(&self, repo: &str) -> bool {
        let settings = self.settings.as_ref().map(|settings| &settings.value);
        settings.is_some_and(|settings| {
            settings.repos.contains_key(repo) || settings.sweep.repo.as_deref() == Some(repo)
        }) || self
            .triggers
            .values()
            .any(|trigger| trigger.value.repo.as_deref() == Some(repo))
    }

    /// The defaults of `priory add`; none makes it need `--workflow`.
    pub fn request(&self) -> Option<&RequestDefaults> {
        self.settings.as_ref()?.value.request.as_ref()
    }

    /// The `[batch]` table: how `priory batch add` plans batches.
    pub fn batch(&self) -> Option<&BatchDefaults> {
        self.settings.as_ref()?.value.batch.as_ref()
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

    /// The command that opens a new run of a step with `attach = true`, under the factory folder.
    pub fn attach(&self) -> Option<PathBuf> {
        let attach = self.settings.as_ref()?.value.runs.attach.as_ref()?;
        Some(self.dir.join(attach))
    }

    /// The folder of the ask command target `name`, under the factory folder when relative.
    pub fn target(&self, name: &str) -> Option<PathBuf> {
        let folder = self.settings.as_ref()?.value.asks.targets.get(name)?;
        Some(self.dir.join(folder))
    }

    /// The scorer that grades a plan no person reads, from `factory.toml`.
    pub fn critic(&self) -> Option<&str> {
        self.settings
            .as_ref()
            .and_then(|settings| settings.value.critic.as_deref())
    }

    /// The judge that reads an approval step's lessons before it asks a person, from `factory.toml`.
    pub fn pre_review(&self) -> Option<&str> {
        self.settings
            .as_ref()
            .and_then(|settings| settings.value.pre_review.as_deref())
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

    /// The Sentry webhook's settings; none takes no Sentry webhook.
    pub fn sentry_webhook(&self) -> Option<&SentryWebhook> {
        self.settings
            .as_ref()?
            .value
            .webhooks
            .as_ref()?
            .sentry
            .as_ref()
    }

    /// The Figma webhook's settings; none takes no Figma webhook.
    pub fn figma_webhook(&self) -> Option<&FigmaWebhook> {
        self.settings
            .as_ref()?
            .value
            .webhooks
            .as_ref()?
            .figma
            .as_ref()
    }

    /// The CloudWatch webhook's settings; none takes no CloudWatch alarm events.
    pub fn cloudwatch_webhook(&self) -> Option<&CloudwatchWebhook> {
        self.settings
            .as_ref()?
            .value
            .webhooks
            .as_ref()?
            .cloudwatch
            .as_ref()
    }

    /// The Honeycomb webhook's settings; none takes no Honeycomb webhook.
    pub fn honeycomb_webhook(&self) -> Option<&HoneycombWebhook> {
        self.settings
            .as_ref()?
            .value
            .webhooks
            .as_ref()?
            .honeycomb
            .as_ref()
    }

    /// The Intercom webhook's settings; none takes no Intercom webhook.
    pub fn intercom_webhook(&self) -> Option<&IntercomWebhook> {
        self.settings
            .as_ref()?
            .value
            .webhooks
            .as_ref()?
            .intercom
            .as_ref()
    }

    /// The environment variables that hold a secret the factory reads, defaults included, then the
    /// ones `[serve] redact_env` lists.
    pub fn secret_envs(&self) -> Vec<String> {
        let serve = self.serve();
        let mut names = vec![
            serve.map_or_else(token_env, |serve| serve.token_env.clone()),
            self.github_webhook()
                .map_or_else(secret_env, |hook| hook.secret_env.clone()),
            self.linear_webhook()
                .map_or_else(linear_secret_env, |hook| hook.secret_env.clone()),
            self.sentry_webhook()
                .map_or_else(sentry_secret_env, |hook| hook.secret_env.clone()),
            self.figma_webhook()
                .map_or_else(figma_secret_env, |hook| hook.secret_env.clone()),
            self.cloudwatch_webhook()
                .map_or_else(cloudwatch_secret_env, |hook| hook.secret_env.clone()),
            self.honeycomb_webhook()
                .map_or_else(honeycomb_secret_env, |hook| hook.secret_env.clone()),
            self.intercom_webhook()
                .map_or_else(intercom_secret_env, |hook| hook.secret_env.clone()),
            self.slack_shortcut()
                .map_or_else(app_token_env, |shortcut| shortcut.token_env.clone()),
            self.log().url_env().to_owned(),
        ];
        names.extend(self.otlp_headers_env().map(str::to_owned));
        names.extend(self.triage().map(|decision| decision.key_env.clone()));
        names.extend(
            serve
                .iter()
                .flat_map(|serve| serve.redact_env.iter().cloned()),
        );
        names
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

    /// Whether a task waits for an earlier active task that changes the same files.
    pub fn overlap(&self) -> bool {
        self.settings
            .as_ref()
            .is_none_or(|settings| settings.value.queue.overlap)
    }

    /// Whether the queue takes turns between lanes inside a band.
    pub fn fair(&self) -> Option<Fair> {
        self.settings
            .as_ref()
            .and_then(|settings| settings.value.queue.fair)
    }

    /// The bands whose waiting head may stop a run of a lower band.
    pub fn preempt(&self) -> &[String] {
        self.settings
            .as_ref()
            .map_or(&[], |settings| settings.value.queue.preempt.as_slice())
    }

    /// How long a task waits in the queue for each band it moves up in start order, and the
    /// highest band it can reach, the first not in `preempt`; none when unset.
    pub fn age(&self) -> Option<(Duration, usize)> {
        let every = self.settings.as_ref()?.value.queue.age?;
        let preempt = self.preempt();
        let top = self
            .bands()
            .iter()
            .position(|band| !preempt.iter().any(|named| named == band))?;
        Some((every, top))
    }

    /// How many pull requests of a lane or a batch may wait for a person before its new tasks wait.
    pub fn review_limit(&self) -> Option<std::num::NonZeroU32> {
        self.settings
            .as_ref()
            .and_then(|settings| settings.value.queue.review_limit)
    }

    /// The agent's own prompt, then the factory's writing guide.
    pub fn prompt(&self, agent: &Agent) -> String {
        [
            agent.body.trim_end(),
            self.writing.as_deref().unwrap_or("").trim(),
        ]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
    }

    /// `instructions.md` without its surrounding blank lines, or nothing.
    pub fn instructions(&self) -> &str {
        self.instructions.as_deref().unwrap_or("").trim()
    }

    /// The index of the factory's memories, whose files are in `folder`, or nothing without any.
    pub fn memory_index(&self, folder: &str) -> String {
        memory_index(
            folder,
            self.memories
                .iter()
                .map(|(name, memory)| (name.as_str(), &memory.value)),
        )
    }

    /// The instructions, then the index of the memories in `folder`, as every run gets them.
    pub fn run_instructions(&self, folder: &str) -> String {
        join_instructions(self.instructions(), &self.memory_index(folder))
    }

    /// `run_instructions`, then the index of the skills `agent` gets, as a Codex run gets them.
    pub fn codex_instructions(&self, agent: &Agent) -> String {
        let skills = self
            .skills
            .iter()
            .filter(|(name, _)| agent.gets_skill(name))
            .map(|(n, s)| (n.as_str(), &s.value));
        join_instructions(
            &self.run_instructions(MEMORY_FOLDER),
            &skill_index(SKILL_FOLDER, skills),
        )
    }

    /// The factory's `[consult]` table, if it has one.
    pub fn consult(&self) -> Option<&Consult> {
        self.settings.as_ref()?.value.consult.as_ref()
    }

    /// The investigators the agent `name` may consult, by name, in `[consult.allow]`'s order.
    pub fn consultable(&self, name: &str) -> &[String] {
        self.consult()
            .and_then(|consult| consult.allow.get(name))
            .map_or(&[], Vec::as_slice)
    }

    /// The index of the investigators the agent `name` may consult, or nothing without any.
    pub fn consult_index(&self, name: &str) -> String {
        let listed: Vec<(&str, &Agent)> = self
            .consultable(name)
            .iter()
            .filter_map(|target| Some((target.as_str(), &self.agents.get(target)?.value)))
            .filter(|(_, agent)| agent.investigator)
            .collect();
        if listed.is_empty() {
            return String::new();
        }
        let mut index =
            "Investigators you can consult with the consult tool, one question each:".to_owned();
        for (target, agent) in listed {
            let description = agent.description.split_whitespace();
            index.push_str(&format!(
                "\n- {target}: {}",
                description.collect::<Vec<_>>().join(" ")
            ));
        }
        index
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

/// The index of `memories`, whose files are in `folder`, or nothing without any.
pub fn memory_index<'a>(
    folder: &str,
    memories: impl IntoIterator<Item = (&'a str, &'a Memory)>,
) -> String {
    let mut memories = memories.into_iter().peekable();
    if memories.peek().is_none() {
        return String::new();
    }
    let mut index = format!(
        "# Memories\n\nThe factory's memories are in {folder}, one fact per file. Read a memory's \
         file when its description fits your task. Never edit these files: save a fact with the \
         remember tool, and remove a wrong one with forget.\n"
    );
    for (name, memory) in memories {
        let description = memory.description.split_whitespace();
        index.push_str(&format!(
            "\n- {name}.md: {}",
            description.collect::<Vec<_>>().join(" ")
        ));
    }
    index
}

/// The index of `skills`, whose folders are in `folder`, or nothing without any.
pub fn skill_index<'a>(
    folder: &str,
    skills: impl IntoIterator<Item = (&'a str, &'a Skill)>,
) -> String {
    let mut skills = skills.into_iter().peekable();
    if skills.peek().is_none() {
        return String::new();
    }
    let mut index = format!(
        "# Skills\n\nThe factory's skills are in {folder}, one folder per skill. Read a skill's \
         SKILL.md when its description fits your task, and follow it. Never edit these files.\n"
    );
    for (name, skill) in skills {
        let description = skill.description.split_whitespace();
        index.push_str(&format!(
            "\n- {name}/SKILL.md: {}",
            description.collect::<Vec<_>>().join(" ")
        ));
    }
    index
}

/// `instructions`, then the memory `index`, apart by a blank line, either left out when empty.
pub fn join_instructions(instructions: &str, index: &str) -> String {
    [instructions, index]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// The factory folder a command reads: `given`, else `PRIORY_FACTORY` when it is set and not
/// empty, else the current folder.
pub fn folder(given: Option<&str>) -> PathBuf {
    named_folder(given).unwrap_or_else(|| PathBuf::from("."))
}

/// As `folder`, for a command that loads no factory when none is named: `given`, else
/// `PRIORY_FACTORY` when it is set and not empty, else none.
pub fn named_folder(given: Option<&str>) -> Option<PathBuf> {
    given.map(PathBuf::from).or_else(|| {
        std::env::var_os("PRIORY_FACTORY")
            .filter(|env| !env.is_empty())
            .map(PathBuf::from)
    })
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
    if !dir.is_dir() {
        found.problems.push(Problem {
            file: dir.to_path_buf(),
            at: None,
            message: "missing: no such folder".into(),
        });
    } else if settings.is_empty() && found.problems.is_empty() {
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
    if let Some(settings) = &factory.settings
        && let Some(name) = &settings.value.writing
    {
        match fs::read_to_string(dir.join(name)) {
            Ok(text) => factory.writing = Some(text),
            Err(error) => found.problems.push(Problem {
                file: settings.file.clone(),
                at: settings.lines.field("writing").map(|line| line.to_string()),
                message: format!("`writing` names {name}, which {}", unreadable(&error)),
            }),
        }
    }
    match fs::read_to_string(dir.join(INSTRUCTIONS)) {
        Ok(text) => factory.instructions = Some(text),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => found.problems.push(Problem {
            file: INSTRUCTIONS.into(),
            at: None,
            message: format!("cannot be read: {error}"),
        }),
    }
    found.skills("skills", &mut factory.skills);
    found.memories("memories", &mut factory.memories);
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
    // A judge's labeled cases sit beside it as `<name>.eval.toml`, and are no scorer.
    let scorers = found.config_files("scorers").into_iter().filter(|file| {
        !file
            .file_stem()
            .is_some_and(|stem| stem.to_string_lossy().ends_with(".eval"))
    });
    for file in scorers.collect::<Vec<_>>() {
        if let Some((value, lines)) = found.parse::<Scorer>(&file) {
            found.define(&mut factory.scorers, stem(&file), &file, value, lines);
        }
    }
    for file in found.config_files("benchmarks") {
        if let Some((value, lines)) = found.parse::<Benchmark>(&file) {
            found.define(&mut factory.benchmarks, stem(&file), &file, value, lines);
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
    let investigators: Vec<(String, PathBuf)> = factory
        .agents
        .iter()
        .filter(|(_, agent)| agent.value.investigator)
        .map(|(name, agent)| (name.clone(), agent.file.clone()))
        .collect();
    for (investigator, file) in investigators {
        let mut workflow: Workflow =
            toml::from_str(CONSULT_WORKFLOW).expect("the consult workflow parses");
        workflow.steps[0].agent = Some(investigator.clone());
        factory
            .workflows
            .entry(consult_workflow(&investigator))
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

/// The size past which `priory check` warns that a writing guide crowds every agent's prompt.
pub const WRITING_LIMIT: usize = 4096;

fn unreadable(error: &io::Error) -> String {
    match error.kind() {
        io::ErrorKind::NotFound => "does not exist".to_owned(),
        _ => format!("cannot be read: {error}"),
    }
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

    /// Each skill of the folder `folder`, by its own folder's name.
    fn skills(&mut self, folder: &str, skills: &mut BTreeMap<String, Loaded<Skill>>) {
        for path in self
            .entries(folder)
            .into_iter()
            .filter(|path| path.is_dir())
        {
            if let Some((skill, lines)) = self.skill(&path) {
                let file = path.join("SKILL.md");
                self.define(skills, file_name(&path), &file, skill, lines);
            }
        }
    }

    /// Each memory of the folder `folder`, by its file's name without `.md`.
    fn memories(&mut self, folder: &str, memories: &mut BTreeMap<String, Loaded<Memory>>) {
        for path in self.entries(folder).into_iter().filter(|path| {
            // A dotfile such as .gitkeep keeps an empty folder in git, and MEMORY.md is the index
            // that Claude Code's own memory folders hold.
            let name = file_name(path);
            !name.starts_with('.') && name != MEMORY_INDEX
        }) {
            if let Some((memory, lines)) = self.memory(&path) {
                let name = file_name(&path);
                let name = name.trim_end_matches(".md").to_owned();
                self.define(memories, name, &path, memory, lines);
            }
        }
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

    /// The skill in `folder`, once its `SKILL.md` names it after the folder and describes it.
    fn skill(&mut self, folder: &Path) -> Option<(Skill, Lines)> {
        let file = folder.join("SKILL.md");
        let source = self.read(&file)?;
        let (front, lines) = self.parse_source::<SkillFrontmatter>(&file, &source)?;
        let relative = self.relative(&file);
        let problems = &mut *self.problems;
        let mut missing = |message: String| {
            problems.push(Problem {
                file: relative.clone(),
                at: Some("1".into()),
                message,
            });
        };
        let folder_name = file_name(folder);
        match front.name.as_deref().map(str::trim) {
            None | Some("") => missing("a skill's frontmatter needs a `name`".into()),
            Some(name) if name != folder_name => missing(format!(
                "`name` is `{name}`, but the skill's folder is skills/{folder_name}"
            )),
            Some(_) => {}
        }
        let description = match front.description.map(|text| text.trim().to_owned()) {
            Some(text) if !text.is_empty() => text,
            _ => {
                missing("a skill's frontmatter needs a `description`, which tells the agent when to use it".into());
                return None;
            }
        };
        let mut files = BTreeMap::new();
        if let Err(error) = read_tree(folder, Path::new(""), &mut files) {
            self.problems.push(Problem {
                file: self.relative(folder),
                at: None,
                message: format!("cannot be read: {error}"),
            });
            return None;
        }
        let paths = self.globs(&relative, front.paths, &lines);
        Some((
            Skill {
                description,
                paths,
                files,
            },
            lines,
        ))
    }

    /// The memory in `file`, once it is a `<name>.md` file under the limit whose frontmatter names
    /// it after the file and describes it.
    fn memory(&mut self, file: &Path) -> Option<(Memory, Lines)> {
        let relative = self.relative(file);
        let stem = file_name(file)
            .strip_suffix(".md")
            .filter(|_| file.is_file())
            .map(str::to_owned);
        let Some(stem) = stem else {
            self.problems.push(Problem {
                file: relative,
                at: None,
                message: "memories/ holds only <name>.md files, one memory each".into(),
            });
            return None;
        };
        let at = |line: Option<usize>| Some(line.unwrap_or(1).to_string());
        let named = stem.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
            && stem
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        if !named {
            self.problems.push(Problem {
                file: relative,
                at: at(None),
                message:
                    "a memory's file is named <name>.md, with lowercase letters, digits and hyphens"
                        .into(),
            });
            return None;
        }
        let source = self.read(file)?;
        if source.len() > MEMORY_LIMIT {
            self.problems.push(Problem {
                file: relative,
                at: at(None),
                message: format!(
                    "a memory holds one fact in at most {MEMORY_LIMIT} bytes: this file is {} bytes",
                    source.len()
                ),
            });
            return None;
        }
        let (front, lines) = self.parse_source::<MemoryFrontmatter>(file, &source)?;
        let mut problem = |line: Option<usize>, message: String| {
            self.problems.push(Problem {
                file: relative.clone(),
                at: at(line),
                message,
            });
        };
        match front.name.as_deref().map(str::trim) {
            None | Some("") => problem(None, "a memory's frontmatter needs a `name`".into()),
            Some(name) if name != stem => problem(
                lines.field("name"),
                format!("`name` is `{name}`, but the memory's file is memories/{stem}.md"),
            ),
            Some(_) => {}
        }
        let description = match front.description.map(|text| text.trim().to_owned()) {
            Some(text) if !text.is_empty() => text,
            _ => {
                problem(
                    None,
                    "a memory's frontmatter needs a `description`, which tells the agent when to read it".into(),
                );
                return None;
            }
        };
        let paths = self.globs(&relative, front.paths, &lines);
        let memory = Memory {
            description,
            paths,
            step: front.step.map(|step| step.trim().to_owned()),
            bytes: source.into_bytes(),
        };
        Some((memory, lines))
    }

    /// The globs of a frontmatter's `paths`, each one the hook cannot match reported on its line.
    fn globs(
        &mut self,
        file: &Path,
        globs: Option<crate::paths::Globs>,
        lines: &Lines,
    ) -> Vec<String> {
        let globs = globs.map(crate::paths::Globs::into_vec).unwrap_or_default();
        for glob in &globs {
            if let Some(message) = crate::paths::problem(glob) {
                self.problems.push(Problem {
                    file: file.to_path_buf(),
                    at: Some(lines.field("paths").unwrap_or(1).to_string()),
                    message,
                });
            }
        }
        globs
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

/// Reads every file under `root/inside` into `files`, by its path under `root`. A linked folder is
/// skipped, so a link cycle cannot loop.
fn read_tree(root: &Path, inside: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) -> io::Result<()> {
    for entry in fs::read_dir(root.join(inside))? {
        let entry = entry?;
        let path = inside.join(entry.file_name());
        let kind = entry.file_type()?;
        if kind.is_dir() {
            read_tree(root, &path, files)?;
        } else if kind.is_file() || (kind.is_symlink() && root.join(&path).is_file()) {
            files.insert(path.clone(), fs::read(root.join(&path))?);
        }
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const SCORER: &str =
        "description = \"Passed.\"\non = \"result\"\nkind = \"outcome\"\nexpect = [\"passed\"]\n";

    const HELD: &str = "description = \"Held.\"\non = \"merge\"\nkind = \"held\"\nafter = \"merged + 7d\"\nfails = [\"reverted\"]\n";

    #[test]
    fn after_reads_a_delay_in_minutes_hours_or_days_after_the_merge() {
        let delay = |text: &str| parse_after(text).map(|after| after.delay.as_secs());
        assert_eq!(delay("merged + 7d"), Some(7 * 86_400));
        assert_eq!(delay("merged+90m"), Some(5400));
        assert_eq!(delay("merged + 2h"), Some(7200));
        for refused in [
            "merged + 0d",
            "merged + 7",
            "merged - 7d",
            "done + 7d",
            "merged + 1w",
            "",
        ] {
            assert_eq!(delay(refused), None, "{refused}");
        }
    }

    const BENCHMARK: &str = "description = \"Cases.\"\nworkflow = \"implement\"\nprimary = \"passed\"\nscorers = [\"gate\"]\ncases = [\"cases/one\"]\n";

    fn load_scorer(text: &[u8], case: usize) {
        load_file("scorers", text, case);
    }

    fn load_benchmark(text: &[u8], case: usize) {
        load_file("benchmarks", text, case);
    }

    /// Loads and checks a factory whose `folder` holds one file, `text`.
    fn load_file(folder: &str, text: &[u8], case: usize) {
        let dir = std::env::temp_dir()
            .join(format!("priory-{folder}-{}", std::process::id()))
            .join(case.to_string());
        fs::create_dir_all(dir.join(folder)).unwrap();
        fs::write(dir.join("factory.toml"), "name = \"demo\"").unwrap();
        fs::write(dir.join(folder).join("any.toml"), text).unwrap();
        let (factory, _) = load(&dir);
        crate::rules::check(&factory);
        let _ = fs::remove_dir_all(&dir);
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        #[test]
        fn a_scorer_file_of_any_shape_loads_or_is_refused_without_panicking(
            text in proptest::string::string_regex("[a-z_\\- =\"'\\[\\]{}.,0-9\\n#]{0,200}").unwrap(),
            at in 0..SCORER.len(),
            byte in any::<u8>(),
            cut in 0..SCORER.len(),
        ) {
            load_scorer(text.as_bytes(), 0);
            let mut mutated = SCORER.as_bytes().to_vec();
            mutated[at] = byte;
            load_scorer(&mutated, 1);
            load_scorer(&SCORER.as_bytes()[..cut], 2);
            load_scorer(format!("{SCORER}{text}").as_bytes(), 3);
        }

        #[test]
        fn a_held_scorer_file_of_any_shape_loads_or_is_refused_without_panicking(
            after in proptest::string::string_regex("[a-z +\\-0-9]{0,30}").unwrap(),
            at in 0..HELD.len(),
            byte in any::<u8>(),
        ) {
            // Cases of their own: the other scorer test loads into 0 to 3 at the same time.
            load_scorer(HELD.replace("merged + 7d", &after).as_bytes(), 10);
            let mut mutated = HELD.as_bytes().to_vec();
            mutated[at] = byte;
            load_scorer(&mutated, 11);
            let parsed = parse_after(&after);
            prop_assert!(parsed.is_none_or(|parsed| parsed.delay.as_secs() >= 60));
        }

        #[test]
        fn a_benchmark_file_of_any_shape_loads_or_is_refused_without_panicking(
            text in proptest::string::string_regex("[a-z_\\- =\"'\\[\\]{}.,/0-9\\n#]{0,200}").unwrap(),
            at in 0..BENCHMARK.len(),
            byte in any::<u8>(),
            cut in 0..BENCHMARK.len(),
        ) {
            load_benchmark(text.as_bytes(), 0);
            let mut mutated = BENCHMARK.as_bytes().to_vec();
            mutated[at] = byte;
            load_benchmark(&mutated, 1);
            load_benchmark(&BENCHMARK.as_bytes()[..cut], 2);
            load_benchmark(format!("{BENCHMARK}{text}").as_bytes(), 3);
        }
    }
}
