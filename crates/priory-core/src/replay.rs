//! `priory bench replay`: plays one benchmark case again under one configuration. It runs this
//! build's daemon in a scratch folder, with the real harness, a fake `gh` that answers from the
//! case, and a scratch origin at the case's start commit, and it ends at the case's first
//! irreversible step without running it. docs/daemon/bench.md describes it.

use herdr_bus::{Bus, Message};
use priory_factory::case::{self, Answer, Case, Loaded, Spent, Tokens};
use priory_factory::factory::{Factory, Price};
use rusqlite::{Connection, OptionalExtension};
use serde_json::Value;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// How long a replay lets one turn run when neither `--turn-timeout` nor a shorter step timeout says.
pub const TURN: Duration = Duration::from_secs(30 * 60);

/// How often the replay reads the scratch state.
const POLL: Duration = Duration::from_millis(200);

/// The variables of the caller's environment a replay keeps: what a shell, tmux, git and the
/// harness's own sign-in need. Every other one, such as a GitHub, Linear or Slack token, is dropped.
const KEPT: [&str; 18] = [
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "SHELL",
    "TERM",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "TZ",
    "TMPDIR",
    "ANTHROPIC_API_KEY",
    "CLAUDE_CODE_OAUTH_TOKEN",
    "CLAUDE_CONFIG_DIR",
    "OPENAI_API_KEY",
    "CODEX_HOME",
    "PRIORY_EXE",
    "PRIORY_PROC",
];

pub struct Options {
    /// The most the replay may spend, in dollars.
    pub budget: f64,
    /// The longest one turn may run.
    pub turn: Duration,
}

/// What the case's recorded steps would cost under the configuration.
#[derive(Debug, PartialEq)]
pub struct Estimate {
    pub dollars: f64,
    /// The most one turn in progress when the budget ends could add: the case's costliest minute,
    /// times the turn timeout.
    pub overrun: f64,
}

/// How a replay ended.
#[derive(Debug, Clone, PartialEq)]
pub enum Ending {
    /// The task got to the irreversible step, which never ran.
    Reached(String),
    /// The workflow reached `stop` or `done` before any irreversible step.
    Stopped,
    /// The agent asked, or a human step had no recorded answer left.
    Asked,
    /// The task waited on something other than a time.
    Waited,
    TimedOut,
    OverBudget,
    AgentError(String),
    /// A model step ended with no tokens, or with a model `prices.toml` does not price.
    CostUnknown(String),
}

impl Ending {
    /// Whether the replay stopped short of where it would have ended, so its spending is cut.
    pub fn incomplete(&self) -> bool {
        matches!(self, Ending::OverBudget | Ending::CostUnknown(_))
    }
}

impl fmt::Display for Ending {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Ending::Reached(step) => write!(f, "reached {step}"),
            Ending::Stopped => f.write_str("stopped"),
            Ending::Asked => f.write_str("asked"),
            Ending::Waited => f.write_str("waited"),
            Ending::TimedOut => f.write_str("timed out"),
            Ending::OverBudget => f.write_str("over budget"),
            Ending::AgentError(why) => write!(f, "agent error: {why}"),
            Ending::CostUnknown(why) => write!(f, "cost unknown: {why}"),
        }
    }
}

/// What a replay did.
pub struct Replayed {
    pub ending: Ending,
    pub spent: f64,
    /// The `gh` calls the case held no answer for, each as its arguments joined by spaces.
    pub unrecorded: Vec<String>,
    pub took: Duration,
}

/// The model a replay's run of `spent.step` uses: the configured model of the step's agent, else
/// the model the case recorded.
fn model<'a>(factory: &'a Factory, workflow: &str, spent: &'a Spent) -> Option<&'a str> {
    factory
        .workflows
        .get(workflow)
        .and_then(|flow| flow.value.step(&spent.step))
        .and_then(|step| step.agent.as_ref())
        .and_then(|agent| factory.agents.get(agent))
        .and_then(|agent| agent.value.model.as_deref())
        .or(spent.model.as_deref())
}

fn dollars(tokens: &Tokens, price: &Price) -> f64 {
    (tokens.input as f64 * price.input
        + tokens.output as f64 * price.output
        + tokens.cache_read as f64 * price.cache_read
        + tokens.cache_write as f64 * price.cache_write)
        / 1_000_000.0
}

/// Prices the case's recorded steps for the configuration's models. Refuses a case with no
/// recorded tokens and a model with no price: a cap in dollars needs every price.
pub fn estimate(case: &Case, factory: &Factory, turn: Duration) -> Result<Estimate, String> {
    if case.spent.is_empty() {
        return Err(
            "the case records no model step's tokens, so nothing can be estimated: capture it again"
                .to_owned(),
        );
    }
    let (mut total, mut rate) = (0.0, 0.0f64);
    for spent in &case.spent {
        let Some(tokens) = &spent.tokens else {
            return Err(format!(
                "the case holds no tokens for step {}, so nothing can be estimated: fill them in case.toml",
                spent.step
            ));
        };
        let Some(model) = model(factory, &case.workflow, spent) else {
            return Err(format!(
                "step {} names no model, so no price applies: set its agent's model",
                spent.step
            ));
        };
        let Some(price) = factory.prices.get(model) else {
            return Err(format!(
                "prices.toml has no price for {model}, the model of step {}: a cap in dollars needs every price",
                spent.step
            ));
        };
        let cost = dollars(tokens, price);
        total += cost;
        rate = rate.max(cost / spent.minutes.max(1) as f64);
    }
    Ok(Estimate {
        dollars: total,
        overrun: rate * turn.as_secs_f64() / 60.0,
    })
}

/// Refuses an estimate over the budget, and names the budget that would fit.
pub fn fits(estimate: &Estimate, budget: f64) -> Result<(), String> {
    if estimate.dollars <= budget {
        return Ok(());
    }
    Err(format!(
        "the estimate, ${:.4}, is over the ${budget:.4} budget: give --budget {:.2} or more",
        estimate.dollars,
        (estimate.dollars * 100.0).ceil() / 100.0
    ))
}

/// What the replay read of the scratch state at one moment.
#[derive(Debug, Default)]
pub struct Seen {
    /// The task's state, step and reason, once it exists.
    pub task: Option<(String, String, Option<String>)>,
    /// The task's open asks: their id, kind and step.
    pub asks: Vec<(i64, String, String)>,
    /// The kinds of the task's open waits.
    pub waits: Vec<String>,
    /// How long the live run's turn has run, in seconds, and its step.
    pub turn: Option<(i64, String)>,
    /// How long the task has slept or queued, in seconds.
    pub since: i64,
    pub spent: f64,
    /// Why the spending is not known.
    pub unknown: Option<String>,
}

/// What the replay does next.
#[derive(Debug, PartialEq)]
pub enum Next {
    Wait,
    /// Answers the ask with the text.
    Answer(i64, String),
    /// Ends the replay, cancelling the task first when `true`.
    End(Ending, bool),
}

/// Decides what the replay does next from what it saw. `answered` counts the answers already given
/// at each human step; `limit` is the turn timeout of a step.
pub fn next(
    seen: &Seen,
    answers: &[Answer],
    answered: &BTreeMap<String, usize>,
    budget: f64,
    limit: &dyn Fn(&str) -> Duration,
    irreversible: &dyn Fn(&str) -> bool,
) -> Next {
    if let Some(why) = &seen.unknown {
        return Next::End(Ending::CostUnknown(why.clone()), true);
    }
    if seen.spent > budget {
        return Next::End(Ending::OverBudget, true);
    }
    let Some((state, step, reason)) = &seen.task else {
        return Next::Wait;
    };
    let why = || {
        reason
            .clone()
            .unwrap_or_else(|| format!("the task is {state}"))
    };
    match state.as_str() {
        "done" | "dropped" => return Next::End(Ending::Stopped, false),
        "blocked" if irreversible(step) => return Next::End(Ending::Reached(step.clone()), false),
        "failed" if why().contains("timed out") => return Next::End(Ending::TimedOut, false),
        "blocked" | "failed" => return Next::End(Ending::AgentError(why()), false),
        _ => {}
    }
    if let Some((turn, at)) = &seen.turn
        && *turn as u64 >= limit(at).as_secs()
    {
        return Next::End(Ending::TimedOut, true);
    }
    if state == "asleep" {
        if let Some((id, kind, at)) = seen.asks.first() {
            if kind != "step" {
                return Next::End(Ending::Asked, true);
            }
            let given = answered.get(at).copied().unwrap_or(0);
            return match answers
                .iter()
                .filter(|answer| answer.step == *at)
                .nth(given)
            {
                Some(answer) => Next::Answer(*id, answer.answer.clone()),
                None => Next::End(Ending::Asked, true),
            };
        }
        if seen.waits.iter().any(|kind| kind != "time") {
            return Next::End(Ending::Waited, true);
        }
        if seen.since as u64 >= limit(step).as_secs() {
            return Next::End(Ending::Waited, true);
        }
    }
    if state == "queued" && seen.since as u64 >= limit(step).as_secs() {
        return Next::End(
            Ending::AgentError(format!("the task never started: {}", why())),
            true,
        );
    }
    Next::Wait
}

/// Reads what the replay needs of the scratch state: the task, its asks, waits and live turn, and
/// what its model steps spent, priced from `prices`.
fn see(db: &Connection, prices: &BTreeMap<String, Price>) -> rusqlite::Result<Seen> {
    let mut seen = Seen::default();
    let task: Option<(i64, String, String, Option<String>, i64)> = db
        .query_row(
            "SELECT id, state, step, reason,
               CAST((julianday('now') - julianday(coalesce(entered, changed))) * 86400 AS INTEGER)
             FROM tasks ORDER BY id LIMIT 1",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .optional()?;
    let Some((id, state, step, reason, since)) = task else {
        return Ok(seen);
    };
    seen.task = Some((state, step, reason));
    seen.since = since;
    let mut statement = db.prepare(
        "SELECT id, kind, step FROM asks
         WHERE task = ?1 AND answer IS NULL AND withdrawn IS NULL ORDER BY id",
    )?;
    seen.asks = statement
        .query_map([id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let mut statement = db.prepare(
        "SELECT kind FROM waits WHERE task = ?1 AND fired IS NULL AND withdrawn IS NULL",
    )?;
    seen.waits = statement
        .query_map([id], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    seen.turn = db
        .query_row(
            "SELECT CAST((julianday('now') - julianday(turn)) * 86400 AS INTEGER), step
             FROM runs WHERE task = ?1 AND ended IS NULL AND turn IS NOT NULL",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let mut statement = db.prepare(
        "SELECT json_extract(event, '$.data.step'), json_extract(event, '$.data.model'),
           json_extract(event, '$.data.tokens')
         FROM trace WHERE type = 'factory.step.ended'
           AND json_extract(event, '$.data.decider') = 'model'
         UNION ALL
         SELECT step, model, tokens FROM tasks WHERE id = ?1 AND tokens IS NOT NULL",
    )?;
    let rows = statement
        .query_map([id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (step, model, tokens) in rows {
        let Some(tokens) = tokens.and_then(|tokens| serde_json::from_str::<Tokens>(&tokens).ok())
        else {
            seen.unknown = Some(format!("step {step} ended with no tokens"));
            continue;
        };
        match model.as_deref().and_then(|model| prices.get(model)) {
            Some(price) => seen.spent += dollars(&tokens, price),
            None => {
                let model = model.unwrap_or_else(|| "no model".to_owned());
                seen.unknown = Some(format!("prices.toml has no price for {model}"));
            }
        }
    }
    Ok(seen)
}

/// The scratch folder of one replay. Dropping it stops the daemon and the tmux server, then
/// removes the task's worktree and the folder, each by its exact path.
struct Scratch {
    dir: PathBuf,
    env: Vec<(OsString, OsString)>,
    daemon: Option<Child>,
}

impl Scratch {
    fn command(&self, program: impl AsRef<std::ffi::OsStr>) -> Command {
        let mut command = Command::new(program);
        command.env_clear().envs(self.env.iter().cloned());
        command
    }

    fn priory(&self, args: &[&str]) -> Result<(), String> {
        let exe = std::env::current_exe().map_err(|error| error.to_string())?;
        let output = self
            .command(exe)
            .args(args)
            .stdin(Stdio::null())
            .output()
            .map_err(|error| error.to_string())?;
        if output.status.success() {
            return Ok(());
        }
        Err(format!(
            "priory {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        if let Some(mut daemon) = self.daemon.take() {
            let _ = daemon.kill();
            let _ = daemon.wait();
        }
        let _ = self
            .command("tmux")
            .arg("kill-server")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let worktree = Connection::open(self.dir.join("priory.db"))
            .and_then(|db| {
                db.query_row(
                    "SELECT worktree FROM tasks WHERE worktree IS NOT NULL ORDER BY id LIMIT 1",
                    [],
                    |row| row.get::<_, String>(0),
                )
            })
            .ok();
        // Kit folders are read-only, and git leaves its objects read-only too.
        for path in worktree
            .map(PathBuf::from)
            .into_iter()
            .chain([self.dir.clone()])
        {
            if path.starts_with(&self.dir) && path.exists() {
                priory_runtime::runtime::remove_sealed(&path);
            }
        }
    }
}

fn io(path: &Path) -> impl Fn(std::io::Error) -> String + '_ {
    move |error| format!("{}: {error}", path.display())
}

/// Copies the folder `from` to `to`, leaving out each name `skip` refuses at its top.
fn copy(from: &Path, to: &Path, skip: &[&str]) -> Result<(), String> {
    fs::create_dir_all(to).map_err(io(to))?;
    for entry in fs::read_dir(from).map_err(io(from))? {
        let entry = entry.map_err(io(from))?;
        if skip.iter().any(|name| entry.file_name() == **name) {
            continue;
        }
        let (source, target) = (entry.path(), to.join(entry.file_name()));
        let kind = entry.file_type().map_err(io(&source))?;
        if kind.is_dir() {
            copy(&source, &target, &[])?;
        } else if kind.is_symlink() {
            let link = fs::read_link(&source).map_err(io(&source))?;
            std::os::unix::fs::symlink(link, &target).map_err(io(&target))?;
        } else {
            fs::copy(&source, &target).map_err(io(&source))?;
        }
    }
    Ok(())
}

fn quote(path: &Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', "'\\''"))
}

/// The fake `gh` and its answers: each recorded call's arguments, NUL-ended, beside its output. A
/// call is answered by the first recorded call with the same arguments that has not answered yet,
/// else by the first that has. Any other call fails and is logged.
fn fake_gh(scratch: &Path, gh: &[case::GhAnswer]) -> Result<(), String> {
    let answers = scratch.join("gh");
    fs::create_dir_all(&answers).map_err(io(&answers))?;
    for (index, answer) in gh.iter().enumerate() {
        let name = answers.join(format!("{:03}", index + 1));
        let args: String = answer.args.iter().map(|arg| format!("{arg}\0")).collect();
        fs::write(name.with_extension("args"), args).map_err(io(&name))?;
        fs::write(name.with_extension("out"), &answer.output).map_err(io(&name))?;
    }
    let script = format!(
        r#"#!/bin/sh
# The replay's gh: answers a call the case recorded with what it printed then.
dir={answers}
asked=$(mktemp "$dir/asked.XXXXXX") || exit 1
printf '%s\0' "$@" > "$asked"
for args in "$dir"/*.args; do
  if cmp -s "$asked" "$args"; then
    rm -f "$asked"; mv "$args" "$args.used"; cat "${{args%.args}}.out"; exit 0
  fi
done
for args in "$dir"/*.args.used; do
  if cmp -s "$asked" "$args"; then
    rm -f "$asked"; cat "${{args%.args.used}}.out"; exit 0
  fi
done
rm -f "$asked"
printf '%s\n' "$*" >> {log}
echo "gh: not recorded in this case: $*" >&2
exit 1
"#,
        answers = quote(&answers),
        log = quote(&scratch.join("gh-unrecorded.log")),
    );
    let bin = scratch.join("bin");
    fs::create_dir_all(&bin).map_err(io(&bin))?;
    let file = bin.join("gh");
    fs::write(&file, script).map_err(io(&file))?;
    fs::set_permissions(&file, fs::Permissions::from_mode(0o755)).map_err(io(&file))
}

/// A bare origin for `repo` whose `main` is the case's start commit, pushed from Priory's clone of
/// `repo` under `home`.
fn origin(scratch: &Path, home: &Path, repo: &str, start: &str) -> Result<PathBuf, String> {
    let clone = home.join("repos").join(repo);
    if !clone.exists() {
        return Err(format!(
            "no clone of {repo} in {}: run one task in {repo} on this machine first",
            clone.display()
        ));
    }
    let bare = scratch.join("origin").join(format!("{repo}.git"));
    let git = |dir: &Path, args: &[&str]| -> Result<(), String> {
        let output = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_TERMINAL_PROMPT", "0")
            .stdin(Stdio::null())
            .output()
            .map_err(|error| format!("git: {error}"))?;
        if output.status.success() {
            return Ok(());
        }
        Err(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    };
    fs::create_dir_all(&bare).map_err(io(&bare))?;
    git(
        &bare,
        &["init", "--quiet", "--bare", "--initial-branch=main"],
    )?;
    let target = bare.display().to_string();
    git(&clone, &["cat-file", "-e", &format!("{start}^{{commit}}")]).map_err(|_| {
        format!(
            "the clone {} lacks the case's start commit {start}: fetch it there",
            clone.display()
        )
    })?;
    git(
        &clone,
        &[
            "push",
            "--quiet",
            &target,
            &format!("{start}:refs/heads/main"),
        ],
    )?;
    Ok(bare)
}

/// The environment of the replay's daemon: the kept variables of this one, then the scratch
/// folder's own.
fn environment(scratch: &Path, bare: &Path, repo: &str) -> Vec<(OsString, OsString)> {
    let mut env: Vec<(OsString, OsString)> = std::env::vars_os()
        .filter(|(name, _)| KEPT.iter().any(|kept| name == *kept))
        .collect();
    let path = std::env::var_os("PATH").unwrap_or_default();
    let path = std::env::join_paths(
        std::iter::once(scratch.join("bin")).chain(std::env::split_paths(&path)),
    )
    .unwrap_or(path);
    env.retain(|(name, _)| name != "PATH");
    let set: [(&str, OsString); 10] = [
        ("PATH", path),
        ("PRIORY_DB", scratch.join("priory.db").into()),
        ("PRIORY_HOME", scratch.join("home").into()),
        ("HERDR_BUS_DB", scratch.join("bus.db").into()),
        ("TMUX_TMPDIR", scratch.into()),
        ("XDG_RUNTIME_DIR", scratch.into()),
        ("GIT_CONFIG_COUNT", "1".into()),
        (
            "GIT_CONFIG_KEY_0",
            format!("url.file://{}.insteadOf", bare.display()).into(),
        ),
        (
            "GIT_CONFIG_VALUE_0",
            format!("https://github.com/{repo}.git").into(),
        ),
        ("GIT_TERMINAL_PROMPT", "0".into()),
    ];
    env.extend(set.into_iter().map(|(name, value)| (name.into(), value)));
    env
}

/// The message that asks the scratch daemon for the task: the case's request, with its
/// repository when the request names none.
fn request(case: &Case, repo: &str) -> Result<Message, String> {
    let Some(request) = &case.request else {
        return Err("the case holds no request: fill [request] in case.toml".to_owned());
    };
    let mut data = serde_json::to_value(&request.data).map_err(|error| error.to_string())?;
    if let Value::Object(fields) = &mut data {
        fields
            .entry("repo")
            .or_insert_with(|| Value::from(repo.to_owned()));
        fields
            .entry("workflow")
            .or_insert_with(|| Value::from(case.workflow.clone()));
    }
    let message = Message::new(request.kind.clone(), data);
    Ok(match &request.subject {
        Some(subject) => message.subject(subject.clone()),
        None => message,
    })
}

/// Plays the case in `loaded` under the factory `factory`, whose folder is `dir`, within the
/// budget, after `fits` accepted its estimate. `say` prints a line as the replay goes.
pub fn replay(
    dir: &Path,
    factory: &Factory,
    loaded: &Loaded,
    options: &Options,
    say: &mut dyn FnMut(&str),
) -> Result<Replayed, String> {
    let case = &loaded.case;
    let flow = factory.workflows.get(&case.workflow).ok_or_else(|| {
        format!(
            "the configuration has no workflow {}, which the case ran",
            case.workflow
        )
    })?;
    let repo = case
        .repo
        .as_deref()
        .ok_or("the case names no repository: set repo in case.toml")?;
    let began = Instant::now();
    let path = std::env::temp_dir().join(format!("priory-replay-{}", std::process::id()));
    fs::create_dir(&path).map_err(io(&path))?;
    let mut scratch = Scratch {
        dir: path.clone(),
        env: Vec::new(),
        daemon: None,
    };
    say(&format!("scratch: {}", path.display()));
    copy(dir, &path.join("factory"), &[".git", "runners"])?;
    for name in ["factory.toml", "factory.yaml", "factory.yml"] {
        let file = path.join("factory").join(name);
        if file.exists() {
            fs::remove_file(&file).map_err(io(&file))?;
        }
    }
    let settings = path.join("factory/factory.toml");
    fs::write(&settings, case::replay_settings(dir)?).map_err(io(&settings))?;
    fake_gh(&path, &loaded.gh)?;
    let bare = origin(&path, &priory_runtime::runtime::home(), repo, &case.start)?;
    scratch.env = environment(&path, &bare, repo);

    let db = crate::state::open_at(&path.join("priory.db"))?;
    crate::state::mark_replay(&db).map_err(|error| error.to_string())?;
    let bus = Bus::open(&path.join("bus.db")).map_err(|error| error.to_string())?;
    bus.declare(&priory_intake::intake::output())
        .map_err(|error| error.to_string())?;
    bus.publish(priory_intake::intake::OUTPUT, &request(case, repo)?)
        .map_err(|error| error.to_string())?;

    let exe = std::env::current_exe().map_err(|error| error.to_string())?;
    let log = path.join("daemon.log");
    let stderr = fs::File::create(&log).map_err(io(&log))?;
    scratch.daemon = Some(
        scratch
            .command(&exe)
            .arg("daemon")
            .arg(path.join("factory"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(stderr)
            .spawn()
            .map_err(|error| format!("cannot start the replay's daemon: {error}"))?,
    );

    let steps = &flow.value;
    let limit = |step: &str| {
        steps
            .step(step)
            .and_then(|step| step.timeout)
            .map_or(options.turn, |timeout| timeout.min(options.turn))
    };
    let irreversible = |step: &str| {
        steps
            .step(step)
            .is_some_and(|step| step.effect == priory_factory::factory::Effect::Irreversible)
    };
    let mut answered: BTreeMap<String, usize> = BTreeMap::new();
    let ending = loop {
        std::thread::sleep(POLL);
        if let Some(daemon) = scratch.daemon.as_mut()
            && let Ok(Some(status)) = daemon.try_wait()
        {
            let tail = fs::read_to_string(&log).unwrap_or_default();
            let tail: Vec<&str> = tail.lines().rev().take(5).collect();
            let tail: Vec<&str> = tail.into_iter().rev().collect();
            break Ending::AgentError(format!(
                "the replay's daemon exited with {status}: {}",
                tail.join(" | ")
            ));
        }
        let seen = see(&db, &factory.prices).map_err(|error| error.to_string())?;
        match next(
            &seen,
            &loaded.answers,
            &answered,
            options.budget,
            &limit,
            &irreversible,
        ) {
            Next::Wait => {}
            Next::Answer(ask, text) => {
                let step = seen
                    .asks
                    .iter()
                    .find(|(id, ..)| *id == ask)
                    .map(|(.., step)| step.clone())
                    .unwrap_or_default();
                say(&format!("answer: {step} {text}"));
                let revision: i64 = db
                    .query_row("SELECT revision FROM asks WHERE id = ?1", [ask], |row| {
                        row.get(0)
                    })
                    .map_err(|error| error.to_string())?;
                let revision = revision.to_string();
                let ask = ask.to_string();
                if let Err(error) =
                    scratch.priory(&["answer", "--revision", &revision, &ask, &text])
                {
                    break Ending::AgentError(error);
                }
                *answered.entry(step).or_default() += 1;
            }
            Next::End(ending, cancel) => {
                if cancel {
                    let _ = scratch.priory(&["cancel", "1"]);
                }
                break ending;
            }
        }
    };
    let spent = see(&db, &factory.prices)
        .map(|seen| seen.spent)
        .unwrap_or_default();
    let unrecorded = fs::read_to_string(path.join("gh-unrecorded.log"))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect();
    drop(db);
    drop(scratch);
    Ok(Replayed {
        ending,
        spent,
        unrecorded,
        took: began.elapsed(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use priory_factory::case::Reference;

    fn spent(step: &str, minutes: u64, input: u64) -> Spent {
        Spent {
            step: step.to_owned(),
            model: Some("claude-fake-5".to_owned()),
            minutes,
            tokens: Some(Tokens {
                input,
                ..Tokens::default()
            }),
        }
    }

    fn case(spent: Vec<Spent>) -> Case {
        Case {
            workflow: "implement".to_owned(),
            key: "github:owner/repo#42".to_owned(),
            repo: Some("owner/repo".to_owned()),
            start: "abc".to_owned(),
            ends_at: "publish".to_owned(),
            partial: false,
            gaps: vec![],
            request: None,
            reference: Reference {
                path: vec![],
                labels: vec![],
            },
            spent,
        }
    }

    fn priced(models: &[&str]) -> Factory {
        let mut factory = Factory::default();
        for model in models {
            factory.prices.insert(
                (*model).to_owned(),
                Price {
                    input: 1.0,
                    output: 0.0,
                    cache_read: 0.0,
                    cache_write: 0.0,
                },
            );
        }
        factory
    }

    #[test]
    fn the_estimate_prices_each_recorded_step_and_names_the_costliest_minute_times_the_turn() {
        let factory = priced(&["claude-fake-5"]);
        let case = case(vec![
            spent("plan", 2, 1_000_000),
            spent("implement", 1, 3_000_000),
        ]);

        let estimate = estimate(&case, &factory, Duration::from_secs(30 * 60)).unwrap();

        assert_eq!(
            estimate,
            Estimate {
                dollars: 4.0,
                overrun: 90.0
            }
        );
        assert_eq!(
            fits(&estimate, 3.5).unwrap_err(),
            "the estimate, $4.0000, is over the $3.5000 budget: give --budget 4.00 or more"
        );
        assert_eq!(fits(&estimate, 4.0), Ok(()));
    }

    #[test]
    fn the_estimate_refuses_a_model_with_no_price_and_a_step_with_no_tokens() {
        let mut untokened = spent("plan", 1, 1);
        untokened.tokens = None;

        let unpriced = estimate(&case(vec![spent("plan", 1, 1)]), &priced(&[]), TURN);
        let unknown = estimate(&case(vec![untokened]), &priced(&["claude-fake-5"]), TURN);
        let empty = estimate(&case(vec![]), &priced(&["claude-fake-5"]), TURN);

        assert_eq!(
            unpriced.unwrap_err(),
            "prices.toml has no price for claude-fake-5, the model of step plan: a cap in dollars needs every price"
        );
        assert_eq!(
            unknown.unwrap_err(),
            "the case holds no tokens for step plan, so nothing can be estimated: fill them in case.toml"
        );
        assert!(empty.unwrap_err().contains("capture it again"));
    }

    fn at(state: &str, step: &str) -> Seen {
        Seen {
            task: Some((state.to_owned(), step.to_owned(), None)),
            ..Seen::default()
        }
    }

    fn decide(seen: &Seen, answers: &[Answer], answered: &BTreeMap<String, usize>) -> Next {
        next(
            seen,
            answers,
            answered,
            1.0,
            &|_| Duration::from_secs(60),
            &|step| step == "publish",
        )
    }

    #[test]
    fn a_replay_ends_where_the_task_settles() {
        let none = BTreeMap::new();
        let mut errored = at("failed", "implement");
        errored.task.as_mut().unwrap().2 = Some("run 1 exited".to_owned());
        let mut timed = at("failed", "implement");
        timed.task.as_mut().unwrap().2 = Some("run 1 timed out: its turn ran past 60s".to_owned());

        let endings = [
            decide(&at("blocked", "publish"), &[], &none),
            decide(&at("dropped", "implement"), &[], &none),
            decide(&at("done", "implement"), &[], &none),
            decide(&errored, &[], &none),
            decide(&timed, &[], &none),
            decide(&at("running", "implement"), &[], &none),
        ];

        assert_eq!(
            endings,
            [
                Next::End(Ending::Reached("publish".to_owned()), false),
                Next::End(Ending::Stopped, false),
                Next::End(Ending::Stopped, false),
                Next::End(Ending::AgentError("run 1 exited".to_owned()), false),
                Next::End(Ending::TimedOut, false),
                Next::Wait,
            ]
        );
    }

    #[test]
    fn a_human_step_takes_its_recorded_answers_in_order_and_then_ends_as_asked() {
        let answers = [
            Answer {
                step: "approve".to_owned(),
                answer: "rejected".to_owned(),
            },
            Answer {
                step: "approve".to_owned(),
                answer: "approved".to_owned(),
            },
        ];
        let mut seen = at("asleep", "approve");
        seen.asks = vec![(7, "step".to_owned(), "approve".to_owned())];
        let mut agent = at("asleep", "implement");
        agent.asks = vec![(8, "agent".to_owned(), "implement".to_owned())];

        let first = decide(&seen, &answers, &BTreeMap::new());
        let second = decide(
            &seen,
            &answers,
            &BTreeMap::from([("approve".to_owned(), 1)]),
        );
        let none = decide(
            &seen,
            &answers,
            &BTreeMap::from([("approve".to_owned(), 2)]),
        );
        let asked = decide(&agent, &answers, &BTreeMap::new());

        assert_eq!(first, Next::Answer(7, "rejected".to_owned()));
        assert_eq!(second, Next::Answer(7, "approved".to_owned()));
        assert_eq!(none, Next::End(Ending::Asked, true));
        assert_eq!(asked, Next::End(Ending::Asked, true));
    }

    #[test]
    fn a_replay_honors_a_wait_on_a_time_within_the_turn_and_ends_on_any_other_wait() {
        let none = BTreeMap::new();
        let mut timed = at("asleep", "implement");
        timed.waits = vec!["time".to_owned()];
        timed.since = 30;
        let mut long = at("asleep", "implement");
        long.waits = vec!["time".to_owned()];
        long.since = 60;
        let mut checks = at("asleep", "implement");
        checks.waits = vec!["checks".to_owned()];

        assert_eq!(decide(&timed, &[], &none), Next::Wait);
        assert_eq!(decide(&long, &[], &none), Next::End(Ending::Waited, true));
        assert_eq!(decide(&checks, &[], &none), Next::End(Ending::Waited, true));
    }

    #[test]
    fn a_replay_cancels_a_turn_past_its_limit_over_the_budget_or_of_unknown_cost() {
        let none = BTreeMap::new();
        let mut long = at("running", "implement");
        long.turn = Some((60, "implement".to_owned()));
        let mut over = at("running", "implement");
        over.spent = 1.5;
        let mut unknown = at("blocked", "publish");
        unknown.unknown = Some("step implement ended with no tokens".to_owned());

        assert_eq!(decide(&long, &[], &none), Next::End(Ending::TimedOut, true));
        assert_eq!(
            decide(&over, &[], &none),
            Next::End(Ending::OverBudget, true)
        );
        assert_eq!(
            decide(&unknown, &[], &none),
            Next::End(
                Ending::CostUnknown("step implement ended with no tokens".to_owned()),
                true
            )
        );
    }
}
