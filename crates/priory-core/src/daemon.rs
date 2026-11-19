use crate::memory::{self, Gate};
use crate::room::Share;
use crate::route;
use crate::state::{self, Admission, NewRun, Signal};
use crate::tools::{self, Act, Answer, Call, Done, Repair};
use crate::trace::Trace;
use crate::{handler, stall, sweep};
use herdr_bus::{Bus, Start};
use priory_factory::factory::{self, Factory, Harness, PermissionMode, Session};
use priory_intake::intake::OUTPUT;
use priory_runtime::runtime::{self, Launch, Line};
use priory_runtime::script::{self, Script};
use priory_runtime::{room, slice, transcript};
use rusqlite::Connection;
use std::cell::OnceCell;
use std::collections::HashMap;
use std::path::Path;
use std::sync::mpsc::RecvTimeoutError;
use std::time::{Duration, Instant};

const CONSUMER: &str = "priory";
// shortcut: polls the bus file; switch to herdr-bus-client's notify with the systemd service.
const EVERY: Duration = Duration::from_secs(1);
/// How often the daemon removes the worktrees of ended tasks.
const SWEEP: Duration = Duration::from_secs(600);
/// How long the queue stays stalled after its scripts before the sweeper starts, unless `after` says.
const AFTER: Duration = Duration::from_secs(1800);
/// How long a sweep script may run when it sets no `timeout`.
const SCRIPT_TIMEOUT: Duration = Duration::from_secs(300);
/// How long a turn may run when its step sets no `timeout`.
const TIMEOUT: Duration = Duration::from_secs(3 * 3600);
/// How long a run goes without a hook event or a pane change before it counts as stalled.
const STALLED: Duration = Duration::from_secs(600);
/// How long a handler call may take.
const HANDLER_TIMEOUT: Duration = Duration::from_secs(120);

/// What the daemon watches for stalls: each live run's last screen and when it changed, and the
/// handler calls in flight.
#[derive(Default)]
struct Stalls {
    screens: HashMap<i64, (Vec<String>, Instant)>,
    calls: Vec<Handling>,
}

/// A handler call in flight on `run`.
struct Handling {
    id: i64,
    run: i64,
    script: Script,
    started: Instant,
}

type Error = Box<dyn std::error::Error>;

/// Handles every message on `factory.intake` past the daemon's cursor. With `once`, stops there.
/// Otherwise it also runs agents: it starts runs, types their prompts and takes their signals.
pub fn run(factory: &Factory, once: bool) -> Result<(), String> {
    serve(factory, once).map_err(|error| error.to_string())
}

fn serve(factory: &Factory, once: bool) -> Result<(), Error> {
    runtime::bound_commands(factory.command_timeout());
    let mut db = state::open()?;
    tools::backfill_endings(&mut db, &factory.workflows)?;
    let bus = Bus::open_default()?;
    state::bands(&mut db, &factory.bands())?;
    if once {
        drain(&bus, &mut db, factory)?;
        return Ok(());
    }
    let home = runtime::home();
    let listening = runtime::listen(&home)?;
    let trace = Trace {
        factory,
        revision: runtime::revision(&factory.dir),
        machine: room::hostname(),
        usage: transcript::usage,
    };
    let share = Share::of(factory, trace.machine.as_deref());
    let slice = slice::name();
    let scope = slice::scope(&slice, share.cpu, share.memory).unwrap_or_else(|error| {
        eprintln!(
            "priory daemon: no systemd user manager, so runs start outside {slice} and only the room check caps them: {error}"
        );
        Vec::new()
    });
    state::uncertain(&db)?;
    // A crash between ending a run and killing its session leaves its agent running.
    let ended = state::ended_sessions(&db)?;
    for (tmux, _) in runtime::tagged()
        .into_iter()
        .filter(|pair| ended.contains(pair))
    {
        runtime::kill(&tmux);
    }
    runtime::replay(&home, |line| {
        let (_, kill) = take(&mut db, line, &trace).map_err(|error| error.to_string())?;
        kill.inspect(|tmux| runtime::kill(tmux));
        Ok(())
    })?;
    // A reboot or a killed tmux server takes the agents with it; that is the host's doing, not theirs.
    for run in state::live(&db)? {
        if runtime::pane(&run.tmux, &run.session).is_some() {
            continue;
        }
        runtime::kill(&run.tmux);
        if !tools::gone(&mut db, &run.session, &trace)? {
            tools::lost(&mut db, &trace, &run.session, &run.tmux)?;
        }
    }
    // No one bounds a script a stopped daemon started; one whose pid another process reused is gone.
    for (id, group) in stall::orphans(&db)? {
        if let Some((pid, _)) =
            group.filter(|(pid, started)| memory::started(*pid) == Some(*started))
        {
            script::kill(pid);
        }
        let output = script::collect(&log_of(&home, id));
        stall::ended(&db, id, "interrupted by a daemon restart", &output)?;
    }
    // A crash between ending its last script and marking it swept leaves a stall unswept.
    stall::swept(&db)?;
    // A handler call a stopped daemon started still counts, and its answer is recorded but not applied.
    let handlers = home.join("handlers");
    for (id, group) in handler::orphans(&db)? {
        if let Some((pid, _)) =
            group.filter(|(pid, started)| memory::started(*pid) == Some(*started))
        {
            script::kill(pid);
        }
        let output = runtime::answer(&handlers, id);
        let status = handler::INTERRUPTED;
        handler::answered(&mut db, &trace, id, output.as_deref(), status, true)?;
        runtime::forget(&handlers, id);
    }
    let mut stalls = Stalls::default();
    let mut scripts = Vec::new();
    let every = factory.sweep().every.unwrap_or(SWEEP);
    let mut swept: Option<Instant> = None;
    loop {
        drain(&bus, &mut db, factory)?;
        while let Ok(line) = listening.lines.try_recv() {
            apply(&mut db, line, &trace)?;
        }
        tend(&mut db, &trace, &share, &home, &scope)?;
        handle_stalls(&mut db, &trace, &home, &mut stalls)?;
        if sweep_scripts(&db, factory, &home, &mut scripts)? {
            swept = None;
        }
        let sweep = factory.sweep();
        if let (Some(_), Some(workflow)) = (&sweep.agent, factory.workflows.get(factory::SWEEP)) {
            let after = sweep.after.unwrap_or(AFTER);
            let band = factory.bands().first().copied();
            stall::call_sweeper(&mut db, after, band, &workflow.value.start, &home)?;
        }
        if swept.is_none_or(|at| at.elapsed() >= every) {
            swept = Some(Instant::now());
            sweep::run(&db, &home)?;
            let short = || room::measure(&home).is_ok_and(|room| crate::room::short_of_disk(&room));
            sweep::retire(&db, &home, short)?;
        }
        match listening.lines.recv_timeout(EVERY) {
            Ok(line) => apply(&mut db, line, &trace)?,
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return Err("the signal socket closed".into()),
        }
    }
}

/// Applies the answers of the handler calls that ended, then calls the handler on each run stalled
/// past the factory's `stalled`, once per stall and at most `handler::CALLS` times per run.
fn handle_stalls(
    db: &mut Connection,
    trace: &Trace,
    home: &Path,
    stalls: &mut Stalls,
) -> Result<(), Error> {
    let factory = trace.factory;
    let Some(model) = handler::model(factory) else {
        return Ok(());
    };
    let dir = home.join("handlers");
    // Screens first, so an answer is checked against what the run shows now.
    let quiet = handler::quiet(db)?;
    stalls
        .screens
        .retain(|run, _| quiet.iter().any(|quiet| quiet.run == *run));
    for run in &quiet {
        let screen = runtime::agent_screen(&run.tmux, &run.session);
        let seen = stalls
            .screens
            .entry(run.run)
            .or_insert_with(|| (screen.clone(), Instant::now()));
        let joined = screen.join("\n");
        if seen.0 != screen {
            *seen = (screen, Instant::now());
        }
        if run
            .left
            .as_deref()
            .is_some_and(|left| !left.is_empty() && left != joined)
        {
            handler::redrawn(db, run.run)?;
        }
    }
    for mut call in std::mem::take(&mut stalls.calls) {
        let Some((status, log)) = call.script.poll() else {
            stalls.calls.push(call);
            continue;
        };
        let output = runtime::answer(&dir, call.id);
        let moved = stalls
            .screens
            .get(&call.run)
            .is_some_and(|(_, changed)| *changed > call.started);
        let status = if status == "exit 0" || log.is_empty() {
            status
        } else {
            format!("{status}: {log}")
        };
        let kill = handler::answered(db, trace, call.id, output.as_deref(), &status, moved)?;
        runtime::forget(&dir, call.id);
        kill.inspect(|tmux| runtime::kill(tmux));
    }
    let stalled = factory
        .settings
        .as_ref()
        .and_then(|settings| settings.value.stalled)
        .unwrap_or(STALLED)
        .as_secs() as i64;
    for run in handler::quiet(db)? {
        let Some((screen, changed)) = stalls.screens.get(&run.run) else {
            continue;
        };
        let stalled_for = run.silent.min(changed.elapsed().as_secs() as i64);
        // One call per stall, also across a daemon restart: `left` is the screen the last call read.
        let due = stalled_for >= stalled
            && run.calls < handler::CALLS
            && run.handled.is_none_or(|handled| handled > stalled_for)
            && run.left.as_deref() != Some(screen.join("\n").as_str())
            && !stalls.calls.iter().any(|call| call.run == run.run);
        if !due {
            continue;
        }
        let screen = screen.clone();
        let id = handler::called(db, run.run, &screen)?;
        let prompt = handler::prompt(db, factory, run.run, stalled_for, &screen)?;
        match runtime::ask_handler(&dir, id, model, &prompt, HANDLER_TIMEOUT) {
            Ok(script) => {
                handler::spawned(db, id, script.pid(), memory::started(script.pid()))?;
                stalls.calls.push(Handling {
                    id,
                    run: run.run,
                    script,
                    started: Instant::now(),
                });
            }
            Err(error) => {
                let status = format!("cannot start: {error}");
                handler::answered(db, trace, id, None, &status, false)?;
                runtime::forget(&dir, id);
            }
        }
    }
    Ok(())
}

/// Records the sweep scripts that ended, then starts the scripts of a stall that just opened, for
/// its resource. Returns whether that stall is short of disk, which the built-in sweep frees too.
fn sweep_scripts(
    db: &Connection,
    factory: &Factory,
    home: &Path,
    running: &mut Vec<(i64, Script)>,
) -> Result<bool, Error> {
    let mut still = Vec::new();
    for (id, mut script) in running.drain(..) {
        match script.poll() {
            Some((status, output)) => stall::ended(db, id, &status, &output)?,
            None => still.push((id, script)),
        }
    }
    *running = still;
    let Some((stall, resource)) = stall::unswept(db)? else {
        return Ok(false);
    };
    for script in factory
        .sweep()
        .scripts
        .iter()
        .filter(|script| script.when.name() == resource)
    {
        let id = stall::started(db, stall, &script.run)?;
        let log = log_of(home, id);
        let timeout = script.timeout.unwrap_or(SCRIPT_TIMEOUT);
        match script::spawn(&script.run, home, &log, timeout) {
            Ok(child) => {
                stall::spawned(db, id, child.pid(), memory::started(child.pid()))?;
                running.push((id, child));
            }
            Err(error) => stall::ended(db, id, &format!("cannot start: {error}"), "")?,
        }
    }
    stall::swept(db)?;
    Ok(resource == "disk")
}

/// Where sweep script `id` writes its output.
fn log_of(home: &Path, id: i64) -> std::path::PathBuf {
    home.join("sweeps").join(format!("{id}.log"))
}

/// Applies a line from the socket and replies to its sender. A run its line ended is killed after
/// the reply, so the hook that sent it is not killed first.
fn apply(db: &mut Connection, line: Line, trace: &Trace) -> Result<(), Error> {
    let (reply, kill) = take(db, &line.text, trace)?;
    let _ = line.applied.send(reply);
    kill.inspect(|tmux| runtime::kill(tmux));
    Ok(())
}

/// What arrives on the socket: a hook's signal, a run's tool call, a person's answer or desk action,
/// or a memory gate message.
#[derive(serde::Deserialize)]
#[serde(untagged)]
enum Input {
    Signal(Signal),
    Call(Call),
    Answer(Answer),
    Act(Act),
    Gate(Gate),
}

/// Applies one line and returns its reply, with a tmux session to kill. A signal's reply is `ok`;
/// a call's or an answer's is the JSON of its `Result`. A line that is none of them is reported
/// and dropped.
fn take(db: &mut Connection, line: &str, trace: &Trace) -> Result<(String, Option<String>), Error> {
    let replied = |result: Result<String, String>| {
        serde_json::to_string(&result).expect("a result serializes")
    };
    match serde_json::from_str::<Input>(line) {
        Ok(Input::Signal(signal)) => Ok(("ok".to_owned(), state::signal(db, &signal, trace)?)),
        Ok(Input::Call(call)) => Ok((replied(tools::call(db, &call, trace.factory)?), None)),
        Ok(Input::Answer(answer)) => Ok((
            replied(tools::answer(db, &answer, trace, &|worktree, branch| {
                runtime::kept_head(&runtime::home(), worktree, branch)
            })?),
            None,
        )),
        Ok(Input::Act(act)) => {
            let (reply, kill) = tools::act(db, &act, trace.factory)?;
            Ok((replied(reply), kill))
        }
        Ok(Input::Gate(gate)) => {
            let share = Share::of(trace.factory, trace.machine.as_deref());
            let room = || room::measure(&runtime::home());
            let reply = memory::gate(db, &gate, trace.factory, &share, room)?;
            Ok((replied(reply), None))
        }
        Err(error) => {
            eprintln!("priory daemon: dropped line {line:?}: {error}");
            Ok((replied(Err(format!("unreadable line: {error}"))), None))
        }
    }
}

/// Ends runs that exited, timed out or wait on a prompt, fires due waits, starts every run that
/// fits in the room, and types prompts.
fn tend(
    db: &mut Connection,
    trace: &Trace,
    share: &Share,
    home: &Path,
    scope: &[String],
) -> Result<(), Error> {
    let factory = trace.factory;
    for run in state::live(db)? {
        if runtime::pane(&run.tmux, &run.session).is_none() {
            // A dead pane that remain-on-exit keeps holds the session open.
            runtime::kill(&run.tmux);
            if tools::gone(db, &run.session, trace)? {
                continue;
            }
            let reason = format!(
                "run {} exited: {} no longer runs in tmux session {}",
                run.id,
                harness_of(factory, &run.agent).program(),
                run.tmux
            );
            state::block(db, trace, run.task, "exited", &reason)?;
            // A harness that died before SessionStart would die the same way again: a person looks.
            if !run.ready {
                continue;
            }
            let repair = if tools::spent(db, run.id)? {
                None
            } else {
                Some(diagnose(db, factory, share, home, &run.agent)?)
            };
            tools::fail(db, trace, run.task, run.id, &reason, repair.as_ref())?;
        } else if let Some((ended_as, reason)) = verdict(&run, factory) {
            // Committed first, so a crash before the kill leaves an ended run, not a lost one.
            state::block(db, trace, run.task, ended_as, &reason)?;
            runtime::kill(&run.tmux);
        }
    }
    state::due(db)?;
    tools::enter_queued(db, &factory.workflows)?;
    state::disk_due(db, || room::measure(home).ok().map(|room| room.disk_free))?;
    memory::reclaim(db)?;
    perform(db, trace, home)?;
    loop {
        // Measured again after each start, since its clone and worktree take disk.
        let measured = OnceCell::new();
        let room = || measured.get_or_init(|| room::measure(home));
        let Some(admission) = state::admissible(db, factory, share, room)? else {
            break;
        };
        if let Err(reason) = start(db, factory, home, scope, &admission)? {
            state::block(db, trace, admission.task, "failed", &reason)?;
        }
    }
    while let Some((tmux, session, text)) = state::attempt(db)? {
        let typed = runtime::pane(&tmux, &session)
            .ok_or_else(|| "its pane is gone".to_owned())
            .and_then(|pane| runtime::type_text(&pane, &text));
        if let Err(error) = typed {
            eprintln!("priory daemon: typing into {tmux}: {error}");
        }
    }
    Ok(())
}

/// Classifies why a run's agent exited; the admission's room check holds a memory restart.
fn diagnose(
    db: &Connection,
    factory: &Factory,
    share: &Share,
    home: &Path,
    agent: &str,
) -> Result<Repair, Error> {
    let need = crate::room::need(factory.agents.get(agent).map(|agent| &agent.value));
    let short = |db: &Connection| -> Result<Result<Option<crate::room::Short>, String>, Error> {
        let promised = state::promised(db, factory, None)?;
        Ok(room::measure(home).map(|room| crate::room::wait(&room, share, promised, need)))
    };
    let missing = |short: &crate::room::Short| {
        short
            .reason
            .trim_start_matches("waits for room: ")
            .to_owned()
    };
    match short(db)? {
        Ok(Some(found)) if found.resource == "memory" => Ok(Repair {
            cause: format!("out of memory ({})", missing(&found)),
            action: "wait for room",
            gone: true,
        }),
        Ok(Some(found)) if found.resource == "disk" => {
            sweep::run(db, home)?;
            // An unreadable room confirms nothing.
            let gone =
                short(db)?.is_ok_and(|after| after.is_none_or(|after| after.resource != "disk"));
            Ok(Repair {
                cause: format!("disk full ({})", missing(&found)),
                action: "sweep",
                gone,
            })
        }
        _ => Ok(Repair {
            cause: "the harness crashed".to_owned(),
            action: "restart",
            gone: true,
        }),
    }
}

/// Why a live run must end: no `SessionStart`, a permission prompt, or a turn past its step's timeout.
fn verdict(run: &state::LiveRun, factory: &Factory) -> Option<(&'static str, String)> {
    if run.late {
        // Claude and Codex ask whether to trust a folder, and Codex its hooks, before any hook; Priory
        // never answers.
        let clone = run
            .worktree
            .as_deref()
            .and_then(|worktree| runtime::clone_of(Path::new(worktree)))
            .map_or_else(|| "<clone>".to_owned(), |clone| clone.display().to_string());
        let harness = harness_of(factory, &run.agent);
        let exe = runtime::exe().unwrap_or_else(|_| "priory".into());
        let mut reason = format!(
            "{} showed no SessionStart within {}; {}",
            harness.program(),
            state::READY_WITHIN.replace(" seconds", "s"),
            harness.trust(&exe, &clone)
        );
        let screen = runtime::screen(&run.tmux);
        eprintln!(
            "priory daemon: run {} never started; its pane ended with:\n{}",
            run.id,
            screen.join("\n")
        );
        let tail: Vec<_> = screen[screen.len().saturating_sub(5)..]
            .iter()
            .map(|line| line.trim())
            .collect();
        if !tail.is_empty() {
            reason.push_str(&format!(". Its pane ended with: {}", tail.join(" | ")));
        }
        return Some(("exited", reason));
    }
    if let Some(message) = &run.prompted {
        let reason = format!("run {} stopped on a permission prompt: {message}", run.id);
        return Some(("permission", reason));
    }
    let limit = factory
        .workflows
        .get(&run.workflow)
        .and_then(|workflow| workflow.value.steps.iter().find(|step| step.id == run.step))
        .and_then(|step| step.timeout)
        .unwrap_or(TIMEOUT)
        .as_secs();
    run.turn.filter(|turn| *turn >= limit as i64).map(|_| {
        let reason = format!("run {} timed out: its turn ran past {limit}s", run.id);
        ("timeout", reason)
    })
}

/// Runs the action of the oldest task at a code step, once an irreversible step's approval holds.
/// An action that is not built yet blocks the task.
fn perform(db: &mut Connection, trace: &Trace, home: &Path) -> Result<(), Error> {
    let factory = trace.factory;
    let Some(due) = state::actionable(db, &factory.workflows)? else {
        return Ok(());
    };
    let step = factory.workflows[&due.workflow]
        .value
        .step(&due.step)
        .expect("actionable checked the step");
    // The sweep or a retirement removed the worktree; its branch still holds the work. Only
    // `open-pr` pushes from it, so no other action waits for disk to add it back.
    if let Some(repo) = repo_of(factory, &due.trigger, due.repo.as_deref())
        && step.action.as_deref() == Some("open-pr")
        && due
            .worktree
            .as_deref()
            .is_some_and(|worktree| !Path::new(worktree).exists())
    {
        // A resurrected task goes through the room check, which a code step otherwise skips.
        let waits = match room::measure(home) {
            Err(error) => Some(format!("waits for room: {error}")),
            Ok(room) if crate::room::short_of_disk(&room) => {
                Some("waits for room to add back its worktree".to_owned())
            }
            Ok(_) => None,
        };
        if let Some(reason) = waits {
            db.execute(
                "UPDATE tasks SET reason = ?2 WHERE id = ?1 AND reason IS NOT ?2",
                rusqlite::params![due.task, reason],
            )?;
            return Ok(());
        }
        if let Err(reason) = runtime::worktree(home, repo, due.task) {
            return Ok(state::block(db, trace, due.task, "blocked", &reason)?);
        }
        db.execute("UPDATE tasks SET retired = NULL WHERE id = ?1", [due.task])?;
    }
    let head = due
        .worktree
        .as_deref()
        .zip(due.branch.as_deref())
        .and_then(|(worktree, branch)| runtime::kept_head(home, Path::new(worktree), branch));
    if !tools::approved(db, due.task, step, head.as_deref())? {
        return Ok(());
    }
    let done = match step.action.as_deref().unwrap_or_default() {
        "open-pr" => open_pr(db, factory, &due, head.as_deref())?,
        "open-issue" => open_issue(db, factory, &due)?,
        other => {
            let reason = format!("action {other} is not built yet");
            return Ok(state::block(db, trace, due.task, "blocked", &reason)?);
        }
    };
    Ok(tools::performed(
        db,
        due.task,
        &due.workflow,
        step,
        done,
        trace,
    )?)
}

/// Pushes `commit` to the task's branch and opens its pull request, titled and described by its
/// latest result.
fn open_pr(
    db: &Connection,
    factory: &Factory,
    due: &state::Due,
    commit: Option<&str>,
) -> Result<Done, Error> {
    let described = state::last_result(db, due.task)?;
    let repo = repo_of(factory, &due.trigger, due.repo.as_deref());
    let opened = match (repo, due.worktree.as_deref().zip(commit), described) {
        (None, ..) => Err(format!("task {} names no repo", due.task)),
        (_, None, _) => Err("the task's branch has no commit yet".to_owned()),
        (.., None) => Err("no result describes the pull request".to_owned()),
        (Some(repo), Some((worktree, commit)), Some((summary, evidence))) => {
            let branch = due
                .branch
                .as_deref()
                .expect("a commit comes from the branch");
            let title = summary.lines().next().unwrap_or_default();
            let body = match evidence {
                Some(evidence) => format!("{summary}\n\n{evidence}"),
                None => summary.clone(),
            };
            runtime::open_pull_request(Path::new(worktree), repo, branch, commit, title, &body)
                .map(|number| route::pull_request(repo, number))
        }
    };
    Ok(match opened {
        Ok(pull_request) => Done {
            status: "opened",
            pull_request: Some(pull_request),
            commit: commit.map(str::to_owned),
            error: None,
        },
        Err(error) => Done {
            status: "failed",
            pull_request: None,
            commit: None,
            error: Some(error),
        },
    })
}

/// Opens an issue on the task's repository, titled and described by its latest result.
fn open_issue(db: &Connection, factory: &Factory, due: &state::Due) -> Result<Done, Error> {
    let opened = match (
        repo_of(factory, &due.trigger, due.repo.as_deref()),
        state::last_result(db, due.task)?,
    ) {
        (None, _) => Err(format!("task {} names no repo", due.task)),
        (_, None) => Err("no result describes the issue".to_owned()),
        (Some(repo), Some((summary, evidence))) => {
            let title = summary.lines().next().unwrap_or_default();
            let body = match evidence {
                Some(evidence) => format!("{summary}\n\n{evidence}"),
                None => summary.clone(),
            };
            let source: String = db.query_row(
                "SELECT source FROM tasks WHERE id = ?1",
                [due.task],
                |row| row.get(0),
            )?;
            let mark = format!("Filed by Priory for event {source}.");
            runtime::open_issue(repo, title, &body, &mark)
        }
    };
    Ok(match opened {
        Ok(_) => Done {
            status: "opened",
            pull_request: None,
            commit: None,
            error: None,
        },
        Err(error) => Done {
            status: "failed",
            pull_request: None,
            commit: None,
            error: Some(error),
        },
    })
}

/// Starts a run for `admission`. The outer error stops the daemon; the inner one blocks the task.
fn start(
    db: &mut Connection,
    factory: &Factory,
    home: &Path,
    scope: &[String],
    admission: &Admission,
) -> Result<Result<(), String>, Error> {
    let step = factory.workflows[&admission.workflow]
        .value
        .step(&admission.step)
        .expect("admissible checked the step");
    let Some((name, agent)) = step
        .agent
        .as_ref()
        .and_then(|name| Some((name, factory.agents.get(name)?)))
    else {
        return Ok(Err(format!("step {} names no known agent", step.id)));
    };
    let Some(repo) = repo_of(factory, &admission.trigger, admission.repo.as_deref()) else {
        return Ok(Err(format!("task {} names no repo", admission.task)));
    };
    let (worktree, branch) = match runtime::worktree(home, repo, admission.task) {
        Ok(found) => found,
        Err(reason) => return Ok(Err(reason)),
    };
    let head = runtime::head(&worktree, &branch);
    if !tools::approved(db, admission.task, step, head.as_deref())? {
        return Ok(Ok(()));
    }
    let shared = matches!(agent.value.session, Session::Shared);
    let resumed = if shared {
        state::shared(db, name)?
    } else {
        state::resumable(db, admission.task, name)?
    };
    let harness = harness(&agent.value.harness);
    let resume = match (&resumed, &agent.value.harness) {
        (Some(session), Harness::Codex) => state::thread(db, session)?,
        (Some(session), Harness::ClaudeCode) => {
            (state::thread(db, session)?.is_none()).then(|| session.clone())
        }
        (None, _) => None,
    };
    // A session the harness cannot resume, such as one of another harness, starts again.
    let session = resumed
        .filter(|_| resume.is_some())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let tmux = format!("priory-{}", admission.task);
    let prompt = tools::step_prompt(step);
    let run = state::start(
        db,
        &NewRun {
            task: admission.task,
            step: &step.id,
            agent: name,
            tmux: &tmux,
            session: &session,
            repo,
            branch: &branch,
            worktree: &worktree.to_string_lossy(),
            prompt: admission.fresh.then_some(prompt.as_str()),
            ending: &tools::ending(step),
            shared,
            lanes: factory.lanes(),
        },
    )?;
    let first = match agent.value.harness {
        Harness::Codex => state::claim(db, run)?,
        Harness::ClaudeCode => None,
    };
    let agent = &agent.value;
    let permissions = serde_json::to_value(&agent.permissions).expect("permissions serialize");
    Ok(runtime::launch(
        harness,
        &Launch {
            tmux: &tmux,
            dir: &worktree,
            home,
            session: &session,
            resume: resume.as_deref(),
            model: agent.model.as_deref(),
            effort: agent.effort.as_deref(),
            body: &agent.body,
            prompt: first.as_deref(),
            permissions,
            permission_mode: agent
                .permissions
                .default_mode
                .map_or("manual", PermissionMode::as_str),
            scope,
        },
    ))
}

/// The repository a task works in: its own, else its trigger's for a task stored before tasks
/// had one; a sweeper's is the `[sweep]` repository.
pub(crate) fn repo_of<'f>(
    factory: &'f Factory,
    trigger: &str,
    repo: Option<&'f str>,
) -> Option<&'f str> {
    if trigger == stall::TRIGGER {
        return Some(factory.sweep().repo());
    }
    repo.or_else(|| factory.triggers.get(trigger)?.value.repo.as_deref())
}

/// The harness that runs `agent`; Claude Code for an agent the factory lost.
fn harness_of(factory: &Factory, agent: &str) -> &'static dyn runtime::Harness {
    factory
        .agents
        .get(agent)
        .map_or(&runtime::ClaudeCode, |agent| harness(&agent.value.harness))
}

fn harness(harness: &Harness) -> &'static dyn runtime::Harness {
    match harness {
        Harness::ClaudeCode => &runtime::ClaudeCode,
        Harness::Codex => &runtime::Codex,
    }
}

fn drain(bus: &Bus, db: &mut Connection, factory: &Factory) -> Result<(), Error> {
    match bus.connect(CONSUMER, OUTPUT, Start::Beginning) {
        // The intake has not declared its output yet: nothing is pending.
        Err(herdr_bus::Error::UnknownOutput(_)) => return Ok(()),
        connected => connected?,
    };
    loop {
        let deliveries = bus.pull(CONSUMER, 100)?;
        if deliveries.is_empty() {
            return Ok(());
        }
        for delivery in deliveries {
            state::handle(db, &delivery.message, factory)?;
            bus.ack(delivery.connection, delivery.seq)?;
        }
    }
}
