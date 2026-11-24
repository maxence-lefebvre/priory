//! Cloud runs: a step whose agent has `harness = "claude-cloud"` runs in a Claude cloud session.
//! The session reports by pushing a branch that Priory names, which the daemon polls on `origin`.

use crate::trace::Trace;
use priory_factory::factory::{Agent, Factory, Harness, Step};
use priory_runtime::cloud::{NotStarted, NotTaken, Start, Started};
use priory_runtime::{cloud, runtime};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde_json::json;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::time::{Duration, Instant};

type Error = Box<dyn std::error::Error>;

/// The harness name a cloud run records.
pub(crate) const HARNESS: &str = "claude-cloud";
/// How long a cloud step waits for its branch when it sets no `timeout`.
const DEADLINE: Duration = Duration::from_secs(2 * 60 * 60);
/// How often the daemon asks `origin` for a live cloud run's branch.
const POLL: Duration = Duration::from_secs(60);

/// The run whose session a thread starts. One at a time: a repository has one folder for them.
static STARTING: Mutex<Option<(i64, Starting)>> = Mutex::new(None);

type Starting = Receiver<Result<Started, NotStarted>>;

/// What the daemon knows of each live cloud run past what the state holds.
static WATCHED: Mutex<BTreeMap<i64, Watch>> = Mutex::new(BTreeMap::new());

// shortcut: polls live in memory, so a restart polls every live cloud run at once; fine at a
// handful of runs, move to the state file if a factory runs many.
#[derive(Default)]
struct Watch {
    polled: Option<Instant>,
    polling: Option<Receiver<Result<Option<String>, String>>>,
    /// Its last poll started past the deadline.
    last: bool,
    /// Why the last poll took no push.
    trouble: Option<String>,
    /// What the CLI said when it uploaded the clone.
    upload: Option<String>,
}

/// Whether `agent` starts its steps in a cloud session. A run says itself whether it is one.
pub(crate) fn is_cloud(agent: &Agent) -> bool {
    matches!(agent.harness, Harness::ClaudeCloud)
}

/// Whether run `run` is a cloud run, whatever its agent's harness is now.
pub(crate) fn is_cloud_run(tx: &Transaction, run: i64) -> rusqlite::Result<bool> {
    tx.query_row(
        "SELECT harness IS ?2 FROM runs WHERE id = ?1",
        params![run, HARNESS],
        |row| row.get(0),
    )
}

/// The page of the live cloud session of `task`, if it has one.
pub(crate) fn page(tx: &Transaction, task: i64) -> rusqlite::Result<Option<String>> {
    tx.query_row(
        "SELECT thread FROM runs WHERE task = ?1 AND ended IS NULL AND harness = ?2
         AND thread IS NOT NULL",
        params![task, HARNESS],
        |row| Ok(link(&row.get::<_, String>(0)?)),
    )
    .optional()
}

fn link(cloud: &str) -> String {
    format!("https://claude.ai/code/{cloud}")
}

/// Why a task cannot start a cloud run now.
pub(crate) fn waits(db: &Connection, factory: &Factory) -> rusqlite::Result<Option<String>> {
    let live: u32 = db.query_row(
        "SELECT count(*) FROM runs WHERE ended IS NULL AND harness = ?1",
        [HARNESS],
        |row| row.get(0),
    )?;
    let max = factory.cloud_max();
    if live >= max {
        return Ok(Some(format!(
            "waits for a cloud run to end: {live} of at most {max} run"
        )));
    }
    let starting = STARTING
        .lock()
        .expect("no start panics while it holds the lock");
    Ok(starting
        .is_some()
        .then(|| "waits for another cloud session to start".to_owned()))
}

/// The tmux session that runs `claude --cloud` for `task`, for the few seconds it takes.
pub(crate) fn tmux(task: i64) -> String {
    format!("priory-cloud-{task}")
}

/// The branch the cloud session of `task`'s run with harness session `session` pushes to. The
/// `claude/` prefix is the one cloud sessions use for their own branches.
pub(crate) fn branch(task: i64, session: &str) -> String {
    format!("claude/priory-{task}-{}", &session[..session.len().min(8)])
}

/// The rules that replace the `result` tool's line, since a cloud session cannot reach Priory.
fn rules(branch: &str) -> String {
    format!(
        "When you are done, commit your work and push it to the branch `{branch}`: `git push origin HEAD:{branch}`. \
         Start the last commit message with one line that says what changed. \
         Do not open a pull request: Priory opens it after a person reviews the branch. \
         Nobody answers questions during this session. If you need a decision, write it in the last \
         commit message under `Questions:` and push anyway."
    )
}

/// `text` without `part`.
fn without(text: &str, part: &str) -> String {
    if part.is_empty() {
        return text.trim().to_owned();
    }
    text.replace(&format!("{part}\n"), "")
        .replace(part, "")
        .trim()
        .to_owned()
}

/// The step's entry prompt, without how to end a step with the `result` tool.
fn entry(tx: &Transaction, task: i64, step: &Step) -> rusqlite::Result<String> {
    let prompt = crate::tools::entry_prompt(tx, task, step)?;
    Ok(without(&prompt, &crate::tools::ending(step)))
}

/// Starts the cloud session of the recorded run `run` on a thread, with the agent's prompt, which
/// `start` holds, the step's, the task's event, what is queued for the task and the push rules.
pub(crate) fn dispatch(
    db: &mut Connection,
    step: &Step,
    run: i64,
    task: i64,
    session: &str,
    mut start: Start,
) -> Result<(), Error> {
    let mut queued = Vec::new();
    while let Some(text) = crate::state::claim(db, run)? {
        queued.push(text);
    }
    let tx = db.transaction()?;
    tx.execute(
        "UPDATE runs SET cloud_head = ?2 WHERE id = ?1",
        params![run, start.head],
    )?;
    let entry = entry(&tx, task, step)?;
    let source = crate::state::source(&tx, task)?;
    tx.commit()?;
    let queued = without(&queued.join("\n"), &crate::tools::ending(step));
    let queued = without(&without(&queued, &entry), source.trim());
    start.prompt = [
        start.prompt.trim(),
        &entry,
        source.trim(),
        &queued,
        &rules(&branch(task, session)),
    ]
    .into_iter()
    .filter(|part| !part.is_empty())
    .collect::<Vec<_>>()
    .join("\n\n");
    let (sender, receiver) = channel();
    *STARTING.lock().expect("no start panics") = Some((run, receiver));
    std::thread::spawn(move || {
        let _ = sender.send(cloud::start(&start));
    });
    Ok(())
}

/// Stores what the start under way gave once it ended.
fn collect(db: &mut Connection, trace: &Trace, kit: crate::kit::Current<'_>) -> Result<(), Error> {
    let (run, started) = {
        let mut starting = STARTING.lock().expect("no start panics");
        let Some((run, receiver)) = starting.as_ref() else {
            return Ok(());
        };
        let started = match receiver.try_recv() {
            Ok(started) => started,
            Err(TryRecvError::Empty) => return Ok(()),
            Err(TryRecvError::Disconnected) => {
                Err(NotStarted::Uncertain("its thread stopped".into()))
            }
        };
        let run = *run;
        *starting = None;
        (run, started)
    };
    let live = db
        .query_row(
            "SELECT runs.task, runs.session, tasks.workflow, runs.step FROM runs
             JOIN tasks ON tasks.id = runs.task WHERE runs.id = ?1 AND runs.ended IS NULL",
            [run],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .optional()?;
    let Some((task, session, workflow, step)) = live else {
        if let Ok(started) = started {
            eprintln!(
                "priory daemon: cloud run {run} ended while its session started, which still runs: {}",
                link(&started.id)
            );
        }
        return Ok(());
    };
    match started {
        Ok(started) => {
            db.execute(
                "UPDATE runs SET thread = ?2 WHERE id = ?1",
                params![run, started.id],
            )?;
            db.execute(
                "UPDATE outbox SET state = 'confirmed', confirmed = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                 WHERE run = ?1 AND state = 'attempted'",
                [run],
            )?;
            if let Some(upload) = started.upload {
                eprintln!(
                    "priory daemon: cloud run {run} started from an uploaded copy of the task's branch: {upload}"
                );
                let mut watched = WATCHED.lock().expect("no poll panics");
                watched.entry(run).or_default().upload = Some(upload);
            }
        }
        Err(NotStarted::Uncertain(reason)) => {
            let reason = format!(
                "cloud run {run} may have started a cloud session: {reason}. Look for it at https://claude.ai/code before you resume the task"
            );
            crate::state::block(db, trace, task, "uncertain", &reason)?;
        }
        Err(NotStarted::Refused(reason)) => {
            let step = trace
                .factory
                .workflows
                .get(&workflow)
                .and_then(|workflow| workflow.value.step(&step));
            give_back(db, run, task, step)?;
            let summary = format!("the cloud session did not start: {reason}");
            end(db, trace, kit, task, &session, "failed", &summary, None)?;
        }
    }
    Ok(())
}

/// Queues again what run `run` claimed, which no session read, without the step's own prompt.
fn give_back(
    db: &mut Connection,
    run: i64,
    task: i64,
    step: Option<&Step>,
) -> rusqlite::Result<()> {
    let tx = db.transaction()?;
    let (entry, ending) = match step {
        Some(step) => (entry(&tx, task, step)?, crate::tools::ending(step)),
        None => Default::default(),
    };
    let claimed: Vec<(i64, String)> = tx
        .prepare("SELECT id, text FROM outbox WHERE run = ?1 AND state = 'attempted'")?
        .query_map([run], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    for (id, text) in claimed {
        let text = without(&without(&text, &ending), &entry);
        if text.is_empty() {
            tx.execute("UPDATE inbox SET outbox = NULL WHERE outbox = ?1", [id])?;
            tx.execute(
                "UPDATE subscription_deliveries SET outbox = NULL WHERE outbox = ?1",
                [id],
            )?;
            tx.execute("DELETE FROM outbox WHERE id = ?1", [id])?;
        } else {
            tx.execute(
                "UPDATE outbox SET state = 'queued', run = NULL, attempted = NULL, text = ?2
                 WHERE id = ?1",
                params![id, text],
            )?;
        }
    }
    tx.commit()
}

/// A live cloud run, as the tick reads it.
struct Live {
    id: i64,
    task: i64,
    session: String,
    cloud: Option<String>,
    worktree: String,
    branch: String,
    head: String,
    workflow: String,
    step: String,
    age: i64,
}

/// Ends each live cloud run whose branch is on `origin` as `pushed`, and each past its step's
/// `timeout` as `failed`. A run with no cloud session id and no start under way may or may not have
/// started one, so its task blocks for a person.
pub(crate) fn tick(
    db: &mut Connection,
    trace: &Trace,
    kit: crate::kit::Current<'_>,
) -> Result<(), Error> {
    collect(db, trace, kit)?;
    let starting = STARTING
        .lock()
        .expect("no start panics")
        .as_ref()
        .map(|(run, _)| *run);
    let live: Vec<Live> = db
        .prepare(
            "SELECT runs.id, runs.task, runs.session, runs.thread, coalesce(tasks.worktree, ''),
             coalesce(tasks.branch, ''), coalesce(runs.cloud_head, ''), tasks.workflow, runs.step,
             unixepoch('now') - unixepoch(runs.started)
             FROM runs JOIN tasks ON tasks.id = runs.task
             WHERE runs.ended IS NULL AND runs.harness = 'claude-cloud' ORDER BY runs.id",
        )?
        .query_map([], |row| {
            Ok(Live {
                id: row.get(0)?,
                task: row.get(1)?,
                session: row.get(2)?,
                cloud: row.get(3)?,
                worktree: row.get(4)?,
                branch: row.get(5)?,
                head: row.get(6)?,
                workflow: row.get(7)?,
                step: row.get(8)?,
                age: row.get(9)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    WATCHED
        .lock()
        .expect("no poll panics")
        .retain(|id, _| live.iter().any(|run| run.id == *id));
    for run in &live {
        if starting == Some(run.id) {
            continue;
        }
        let Some(cloud) = &run.cloud else {
            runtime::kill(&tmux(run.task));
            let reason = format!(
                "cloud run {} may or may not have started a cloud session: the daemon stopped while it \
                 started one. Look for it at https://claude.ai/code before you resume the task",
                run.id
            );
            crate::state::block(db, trace, run.task, "uncertain", &reason)?;
            continue;
        };
        let mut watch = WATCHED
            .lock()
            .expect("no poll panics")
            .remove(&run.id)
            .unwrap_or_default();
        if follow(db, trace, kit, run, cloud, &mut watch)? {
            WATCHED
                .lock()
                .expect("no poll panics")
                .insert(run.id, watch);
        }
    }
    Ok(())
}

/// Takes the poll of `run` that ended, starts the next one when it is due, and ends the run on a
/// push or its deadline. Says whether the run goes on.
fn follow(
    db: &mut Connection,
    trace: &Trace,
    kit: crate::kit::Current<'_>,
    run: &Live,
    cloud: &str,
    watch: &mut Watch,
) -> Result<bool, Error> {
    let pushes = branch(run.task, &run.session);
    let worktree = Path::new(&run.worktree);
    if let Some(polling) = &watch.polling {
        let found = match polling.try_recv() {
            Ok(found) => found,
            Err(TryRecvError::Empty) => return Ok(true),
            Err(TryRecvError::Disconnected) => Err("its thread stopped".into()),
        };
        watch.polling = None;
        let taken = match found {
            Ok(Some(commit)) => cloud::adopt(worktree, &run.branch, &run.head, &commit).map(Some),
            Ok(None) => Ok(None),
            Err(reason) => Err(NotTaken::Later(reason)),
        };
        watch.trouble = None;
        match taken {
            Ok(None) => {}
            Ok(Some(pushed)) => {
                end(
                    db,
                    trace,
                    kit,
                    run.task,
                    &run.session,
                    "pushed",
                    &pushed.subject,
                    Some((&pushed.commit, &pushed.body)),
                )?;
                return Ok(false);
            }
            Err(NotTaken::Never(reason)) => {
                let summary = format!(
                    "cloud session {} pushed {pushes}, and Priory did not take it: {reason}",
                    link(cloud)
                );
                end(
                    db,
                    trace,
                    kit,
                    run.task,
                    &run.session,
                    "failed",
                    &summary,
                    None,
                )?;
                return Ok(false);
            }
            Err(NotTaken::Later(reason)) => {
                eprintln!("priory daemon: cloud run {}: {reason}", run.id);
                watch.trouble = Some(reason);
            }
        }
    }
    let timeout = trace
        .factory
        .workflows
        .get(&run.workflow)
        .and_then(|workflow| workflow.value.step(&run.step))
        .and_then(|step| step.timeout)
        .unwrap_or(DEADLINE);
    let late = run.age >= 0 && run.age as u64 >= timeout.as_secs();
    if late && watch.last {
        let seconds = timeout.as_secs();
        let mut summary = match &watch.trouble {
            None => format!("cloud session {cloud} pushed no branch {pushes} within {seconds}s."),
            Some(trouble) => format!(
                "Priory took no push of cloud session {cloud} to {pushes} within {seconds}s: {trouble}."
            ),
        };
        if let Some(upload) = &watch.upload {
            summary.push_str(&format!(
                " It started from an upload, so the Claude GitHub App may not be on the repository: {upload}."
            ));
        }
        summary.push_str(&format!(
            " A cloud run cannot ask, so open the session to see why: {}",
            link(cloud)
        ));
        end(
            db,
            trace,
            kit,
            run.task,
            &run.session,
            "failed",
            &summary,
            None,
        )?;
        return Ok(false);
    }
    // A runner that slept past the deadline still looks once more before it gives up.
    if late || watch.polled.is_none_or(|polled| polled.elapsed() >= POLL) {
        watch.last = late;
        watch.polled = Some(Instant::now());
        let (sender, receiver) = channel();
        watch.polling = Some(receiver);
        let worktree = worktree.to_path_buf();
        std::thread::spawn(move || {
            let _ = sender.send(cloud::poll(&worktree, &pushes));
        });
    }
    Ok(true)
}

/// Stores the result the session's harness session `session` would have sent, then ends its turn as
/// a `Stop` does, so the task moves by `next` like a local run's. A step with no such outcome
/// blocks the task with the summary.
#[allow(clippy::too_many_arguments)]
fn end(
    db: &mut Connection,
    trace: &Trace,
    kit: crate::kit::Current<'_>,
    task: i64,
    session: &str,
    outcome: &str,
    summary: &str,
    pushed: Option<(&str, &str)>,
) -> Result<(), Error> {
    let mut arguments = json!({ "outcome": outcome, "summary": summary });
    if let Some((commit, body)) = pushed {
        arguments["commit"] = json!(commit);
        if !body.is_empty() {
            arguments["evidence"] = json!(body);
        }
    }
    let call = crate::tools::Call {
        tool: "result".into(),
        session: session.into(),
        arguments,
    };
    let (stored, _) = crate::tools::call(db, &call, trace, kit, &|worktree, branch| {
        runtime::head(worktree, branch)
    })?;
    if let Err(refused) = stored {
        let reason = format!("{summary} ({refused})");
        crate::state::block(db, trace, task, outcome, &reason)?;
        return Ok(());
    }
    let tx = db.transaction()?;
    tx.execute(
        "UPDATE runs SET idle = 1, turn = NULL WHERE session = ?1 AND ended IS NULL",
        [session],
    )?;
    crate::tools::stop(&tx, session, trace)?;
    tell(&tx, task)?;
    tx.commit()?;
    Ok(())
}

/// Shows what came during the run on the ask it led to, since no session read it.
fn tell(tx: &Transaction, task: i64) -> rusqlite::Result<()> {
    let kept: Vec<String> = tx
        .prepare(
            "SELECT text FROM outbox WHERE task = ?1 AND state = 'queued' AND compact = 0 ORDER BY id",
        )?
        .query_map([task], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    for text in kept {
        let said: Vec<_> = text
            .lines()
            .filter(|line| !line.starts_with("End this step by calling the result tool"))
            .collect();
        crate::tools::tell_ask(tx, task, &said.join("\n"))?;
    }
    Ok(())
}

/// The most a cloud environment's setup script may hold, as its dialog's file picker takes it.
const SETUP_LIMIT: usize = 64 * 1024;

/// `priory cloud setup-script`: a bash script for a cloud environment's setup script. It writes the
/// instructions and skills a run of `agent` gets, the kit's first, under the session user's
/// `~/.claude`, then runs each repository's `cloud_setup` script, copied from Priory's clone.
pub fn setup_script(
    db: &Connection,
    factory: &Factory,
    home: &Path,
    agent: Option<&str>,
) -> Result<String, String> {
    let agent = match agent {
        Some(name) => Some(
            &factory
                .agents
                .get(name)
                .ok_or_else(|| format!("agent {name} does not exist"))?
                .value,
        ),
        None => None,
    };
    let fetchers =
        crate::kit::Fetchers::new(db, factory, home).map_err(|error| error.to_string())?;
    let current = fetchers.current();
    let sources = crate::kit::sources(factory, current, &[], agent);
    let mut setups = Vec::new();
    let repos = factory
        .settings
        .as_ref()
        .map(|settings| &settings.value.repos);
    for (repo, settings) in repos.into_iter().flatten() {
        if let Some(path) = &settings.cloud_setup {
            setups.push((
                format!("{repo}:{path}"),
                runtime::repo_file(home, repo, path)?,
            ));
        }
    }
    let script = script(
        &factory.name,
        sources.commit,
        &sources.user_files(),
        &setups,
    );
    if script.len() > SETUP_LIMIT {
        return Err(format!(
            "the setup script is {} bytes, over the cloud environment's limit of {SETUP_LIMIT}: name the agent whose skills it needs with --agent, or give that agent fewer skills",
            script.len()
        ));
    }
    Ok(script)
}

/// The script that writes `files` under `~/.claude`, then runs each of `setups`, named by where it
/// came from.
fn script(
    factory: &str,
    commit: Option<&str>,
    files: &[(std::path::PathBuf, &[u8])],
    setups: &[(String, Vec<u8>)],
) -> String {
    let kit = commit.map_or_else(String::new, |commit| format!(", kit commit {commit}"));
    let mut script = format!(
        "#!/bin/bash\n# Written by `priory cloud setup-script` for the factory {factory}{kit}.\n\
         # Paste it into a cloud environment's setup script. It holds no secret.\nset -euo pipefail\n"
    );
    for (path, bytes) in files {
        let target = format!("\"$HOME/.claude/{}\"", path.display());
        script.push_str(&format!(
            "mkdir -p \"$(dirname {target})\"\n{}",
            write(&format!("> {target}"), bytes)
        ));
    }
    for (from, bytes) in setups {
        script.push_str(&format!("# {from}\n{}", write("| bash", bytes)));
    }
    script
}

/// A command that feeds `bytes` to `into`: a quoted heredoc for text, base64 for anything else.
fn write(into: &str, bytes: &[u8]) -> String {
    let Ok(text) = std::str::from_utf8(bytes) else {
        let encoded = <base64ct::Base64 as base64ct::Encoding>::encode_string(bytes);
        return format!("base64 -d <<'PRIORY_BASE64' {into}\n{encoded}\nPRIORY_BASE64\n");
    };
    let mut end = "PRIORY_END".to_owned();
    while text.lines().any(|line| line == end) {
        end.push('_');
    }
    let newline = if text.ends_with('\n') || text.is_empty() {
        ""
    } else {
        "\n"
    };
    // shortcut: a file whose last line has no newline gains one; fine for Markdown and shell.
    format!("cat <<'{end}' {into}\n{text}{newline}{end}\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn the_setup_script_writes_files_through_heredocs_that_their_text_cannot_end() {
        let files = [
            (PathBuf::from("CLAUDE.md"), &b"Be brief.\nPRIORY_END\n"[..]),
            (PathBuf::from("skills/a/icon.bin"), &[0xff, 0x00][..]),
        ];
        let setups = [(
            "owner/repo:scripts/cloud-setup.sh".to_owned(),
            b"apt-get install -y jq".to_vec(),
        )];
        let script = script("demo", Some("abc"), &files, &setups);
        let ran = std::process::Command::new("bash")
            .arg("-c")
            .arg(script.replace("| bash", "> \"$HOME/setup.sh\""))
            .env(
                "HOME",
                std::env::temp_dir().join(format!("cloud-setup-{}", std::process::id())),
            )
            .output()
            .unwrap();
        assert!(ran.status.success(), "{ran:?}");
        let home = std::env::temp_dir().join(format!("cloud-setup-{}", std::process::id()));
        assert_eq!(
            std::fs::read(home.join(".claude/CLAUDE.md")).unwrap(),
            b"Be brief.\nPRIORY_END\n"
        );
        assert_eq!(
            std::fs::read(home.join(".claude/skills/a/icon.bin")).unwrap(),
            [0xff, 0x00]
        );
        assert_eq!(
            std::fs::read(home.join("setup.sh")).unwrap(),
            b"apt-get install -y jq\n"
        );
        std::fs::remove_dir_all(home).unwrap();
    }
}
