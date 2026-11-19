use crate::memory::{self, Gate};
use crate::room::Share;
use crate::route;
use crate::state::{self, Admission, NewRun, Signal};
use crate::sweep;
use crate::tools::{self, Act, Answer, Call, Done};
use crate::trace::Trace;
use herdr_bus::{Bus, Start};
use priory_factory::factory::{Factory, Harness, PermissionMode, Session};
use priory_intake::intake::OUTPUT;
use priory_runtime::runtime::{self, Launch, Line};
use priory_runtime::{room, transcript};
use rusqlite::Connection;
use std::cell::OnceCell;
use std::path::Path;
use std::sync::mpsc::RecvTimeoutError;
use std::time::{Duration, Instant};

const CONSUMER: &str = "priory";
// shortcut: polls the bus file; switch to herdr-bus-client's notify with the systemd service.
const EVERY: Duration = Duration::from_secs(1);
/// How often the daemon removes the worktrees of ended tasks.
const SWEEP: Duration = Duration::from_secs(600);
/// How long a turn may run when its step sets no `timeout`.
const TIMEOUT: Duration = Duration::from_secs(3 * 3600);

type Error = Box<dyn std::error::Error>;

/// Handles every message on `factory.intake` past the daemon's cursor. With `once`, stops there.
/// Otherwise it also runs agents: it starts runs, types their prompts and takes their signals.
pub fn run(factory: &Factory, once: bool) -> Result<(), String> {
    serve(factory, once).map_err(|error| error.to_string())
}

fn serve(factory: &Factory, once: bool) -> Result<(), Error> {
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
    let mut swept: Option<Instant> = None;
    loop {
        drain(&bus, &mut db, factory)?;
        while let Ok(line) = listening.lines.try_recv() {
            apply(&mut db, line, &trace)?;
        }
        tend(&mut db, &trace, &share, &home)?;
        if swept.is_none_or(|at| at.elapsed() >= SWEEP) {
            swept = Some(Instant::now());
            sweep::run(&db, &home)?;
        }
        match listening.lines.recv_timeout(EVERY) {
            Ok(line) => apply(&mut db, line, &trace)?,
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return Err("the signal socket closed".into()),
        }
    }
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
    let workflows = &trace.factory.workflows;
    let replied = |result: Result<String, String>| {
        serde_json::to_string(&result).expect("a result serializes")
    };
    match serde_json::from_str::<Input>(line) {
        Ok(Input::Signal(signal)) => Ok(("ok".to_owned(), state::signal(db, &signal, trace)?)),
        Ok(Input::Call(call)) => Ok((replied(tools::call(db, &call, workflows)?), None)),
        Ok(Input::Answer(answer)) => Ok((
            replied(tools::answer(db, &answer, trace, &runtime::head)?),
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
fn tend(db: &mut Connection, trace: &Trace, share: &Share, home: &Path) -> Result<(), Error> {
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
        } else if let Some((ended_as, reason)) = verdict(&run, factory) {
            // Committed first, so a crash before the kill leaves an ended run, not a lost one.
            state::block(db, trace, run.task, ended_as, &reason)?;
            runtime::kill(&run.tmux);
        }
    }
    state::due(db)?;
    memory::reclaim(db)?;
    perform(db, trace)?;
    loop {
        // Measured again after each start, since its clone and worktree take disk.
        let measured = OnceCell::new();
        let room = || measured.get_or_init(|| room::measure(home));
        let Some(admission) = state::admissible(db, factory, share, room)? else {
            break;
        };
        if let Err(reason) = start(db, factory, home, &admission)? {
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
fn perform(db: &mut Connection, trace: &Trace) -> Result<(), Error> {
    let factory = trace.factory;
    let Some(due) = state::actionable(db, &factory.workflows)? else {
        return Ok(());
    };
    let step = factory.workflows[&due.workflow]
        .value
        .step(&due.step)
        .expect("actionable checked the step");
    let head = head(due.worktree.as_deref(), due.branch.as_deref());
    if !tools::approved(db, due.task, step, head.as_deref())? {
        return Ok(());
    }
    let done = match step.action.as_deref().unwrap_or_default() {
        "open-pr" => open_pr(db, factory, &due, head.as_deref())?,
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

/// The commit of the task's branch, which an approval covers.
fn head(worktree: Option<&str>, branch: Option<&str>) -> Option<String> {
    runtime::head(Path::new(worktree?), branch?)
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
    let repo = factory
        .triggers
        .get(&due.trigger)
        .and_then(|trigger| trigger.value.repo.as_deref());
    let opened = match (repo, due.worktree.as_deref().zip(commit), described) {
        (None, ..) => Err(format!("trigger {} names no repo", due.trigger)),
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

/// Starts a run for `admission`. The outer error stops the daemon; the inner one blocks the task.
fn start(
    db: &mut Connection,
    factory: &Factory,
    home: &Path,
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
    let Some(repo) = factory
        .triggers
        .get(&admission.trigger)
        .and_then(|trigger| trigger.value.repo.as_deref())
    else {
        return Ok(Err(format!("trigger {} names no repo", admission.trigger)));
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
        },
    ))
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
