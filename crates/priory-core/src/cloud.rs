//! Cloud runs: a step whose agent has `harness = "claude-cloud"` runs in a Claude cloud session.
//! The session reports by pushing a branch that Priory names, which the daemon polls on `origin`.

use crate::trace::Trace;
use priory_factory::factory::{Agent, Harness, Step};
use priory_runtime::runtime;
use rusqlite::{Connection, params};
use serde_json::json;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

type Error = Box<dyn std::error::Error>;

/// How long a cloud step waits for its branch when it sets no `timeout`.
const DEADLINE: Duration = Duration::from_secs(2 * 60 * 60);
/// How often the daemon asks `origin` for a live cloud run's branch.
const POLL: Duration = Duration::from_secs(60);

// shortcut: poll times live in memory, so a restart polls every live cloud run at once; fine at a
// handful of runs, move to the state file if a factory runs many.
static POLLED: Mutex<Option<HashMap<i64, Instant>>> = Mutex::new(None);

/// Whether `agent` runs its steps in a cloud session.
pub(crate) fn is_cloud(agent: &Agent) -> bool {
    matches!(agent.harness, Harness::ClaudeCloud)
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

/// Starts the cloud session of the recorded run `run` with the agent's `body`, the run's first
/// prompt and the push rules, and stores the cloud session's id. A failed start ends the step
/// `failed`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn dispatch(
    db: &mut Connection,
    trace: &Trace,
    kit: crate::kit::Current<'_>,
    home: &Path,
    step: &Step,
    run: i64,
    task: i64,
    session: &str,
    worktree: &Path,
    body: &str,
) -> Result<(), Error> {
    let first = crate::state::claim(db, run)?.unwrap_or_default();
    let first = first.replace(&crate::tools::ending(step), "");
    let prompt = [body.trim(), first.trim(), &rules(&branch(task, session))]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    match runtime::cloud_start(&tmux(task), worktree, home, &prompt) {
        Ok(id) => {
            db.execute(
                "UPDATE runs SET thread = ?2 WHERE id = ?1",
                params![run, id],
            )?;
            db.execute(
                "UPDATE outbox SET state = 'confirmed', confirmed = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                 WHERE run = ?1 AND state = 'attempted'",
                [run],
            )?;
            Ok(())
        }
        Err(reason) => end(
            db,
            trace,
            kit,
            task,
            session,
            "failed",
            &format!("the cloud session did not start: {reason}"),
            None,
        ),
    }
}

/// A live cloud run, as the tick reads it.
struct Live {
    id: i64,
    task: i64,
    session: String,
    cloud: Option<String>,
    worktree: String,
    workflow: String,
    step: String,
    age: i64,
}

/// Ends each live cloud run whose branch is on `origin` as `pushed`, and each past its step's
/// `timeout` as `failed`. A run with no cloud session id may or may not have started one, so its
/// task blocks for a person.
pub(crate) fn tick(
    db: &mut Connection,
    trace: &Trace,
    kit: crate::kit::Current<'_>,
) -> Result<(), Error> {
    let live: Vec<Live> = db
        .prepare(
            "SELECT runs.id, runs.task, runs.session, runs.thread, coalesce(tasks.worktree, ''),
             tasks.workflow, runs.step, unixepoch('now') - unixepoch(runs.started)
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
                workflow: row.get(5)?,
                step: row.get(6)?,
                age: row.get(7)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    for run in live {
        let Some(cloud) = run.cloud else {
            let reason = format!(
                "cloud run {} may or may not have started a cloud session: the daemon stopped while it \
                 ran claude --cloud. Look for it at https://claude.ai/code before you resume the task",
                run.id
            );
            crate::state::block(db, trace, run.task, "uncertain", &reason)?;
            continue;
        };
        let branch = branch(run.task, &run.session);
        let worktree = Path::new(&run.worktree);
        if due(run.id) {
            match runtime::remote_head(worktree, &branch) {
                Ok(Some(_)) => {
                    let ended = match runtime::adopt(worktree, &branch) {
                        Ok(pushed) => end(
                            db,
                            trace,
                            kit,
                            run.task,
                            &run.session,
                            "pushed",
                            &pushed.subject,
                            Some((&pushed.commit, &pushed.body)),
                        ),
                        Err(reason) => end(
                            db,
                            trace,
                            kit,
                            run.task,
                            &run.session,
                            "failed",
                            &format!(
                                "cloud session {cloud} pushed {branch}, and Priory could not fetch it: {reason}"
                            ),
                            None,
                        ),
                    };
                    forget(run.id);
                    ended?;
                    continue;
                }
                Ok(None) => {}
                Err(reason) => eprintln!("priory daemon: cloud run {}: {reason}", run.id),
            }
        }
        let timeout = trace
            .factory
            .workflows
            .get(&run.workflow)
            .and_then(|workflow| workflow.value.step(&run.step))
            .and_then(|step| step.timeout)
            .unwrap_or(DEADLINE);
        if run.age >= 0 && run.age as u64 >= timeout.as_secs() {
            let reason = format!(
                "cloud session {cloud} pushed no branch {branch} within {}s. A cloud run cannot ask, so \
                 open the session to see why: https://claude.ai/code/{cloud}",
                timeout.as_secs()
            );
            forget(run.id);
            end(
                db,
                trace,
                kit,
                run.task,
                &run.session,
                "failed",
                &reason,
                None,
            )?;
        }
    }
    Ok(())
}

/// Whether the run's branch is due for another look, and marks it looked at.
fn due(run: i64) -> bool {
    let mut polled = POLLED
        .lock()
        .expect("no poll panics while it holds the lock");
    let polled = polled.get_or_insert_with(HashMap::new);
    let now = Instant::now();
    if polled
        .get(&run)
        .is_some_and(|last| now.duration_since(*last) < POLL)
    {
        return false;
    }
    polled.insert(run, now);
    true
}

fn forget(run: i64) {
    if let Some(polled) = POLLED.lock().expect("no poll panics").as_mut() {
        polled.remove(&run);
    }
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
    tx.commit()?;
    Ok(())
}

/// The most a cloud environment's setup script may hold, as its dialog's file picker takes it.
const SETUP_LIMIT: usize = 64 * 1024;

/// `priory cloud setup-script`: a bash script for a cloud environment's setup script. It writes the
/// instructions and skills a run of `agent` gets, the kit's first, under the session user's
/// `~/.claude`, then runs each repository's `cloud_setup` script, copied from Priory's clone.
pub fn setup_script(
    db: &Connection,
    factory: &priory_factory::factory::Factory,
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
