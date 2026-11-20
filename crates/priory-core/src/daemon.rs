use crate::memory::{self, Gate};
use crate::otlp;
use crate::room::Share;
use crate::route;
use crate::snooze::{self, Snooze};
use crate::state::{self, Admission, NewRun, Signal};
use crate::tools::{self, Act, Answer, Call, Dismiss, Done, Repair};
use crate::trace::Trace;
use crate::{
    deliver, embed, handler, hold, progress, push, search, sections, stall, summary, sweep,
};
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
/// How long the exporter waits after a failed send before it tries again.
const EXPORT_RETRY: Duration = Duration::from_secs(5);

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

/// The OTLP exporter's send in flight, with the last record it carries, and when it may send again.
struct Export {
    endpoint: String,
    sending: Option<(std::process::Child, i64)>,
    after: Instant,
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
    state::irreversible_asks(&mut db, &factory.workflows)?;
    for error in sections::dispatch(&db, factory, None)? {
        eprintln!("priory daemon: {error}");
    }
    let embedder = factory.search().embedder().map(|spec| embed::name(&spec));
    search::configure(&db, embedder.as_deref())?;
    let home = runtime::home();
    let mut counting = Counting {
        embedder: embedder.clone(),
        counter: None,
        tried: None,
    };
    let trace = Trace {
        factory,
        revision: runtime::revision(&factory.dir),
        machine: room::hostname(),
        usage: transcript::usage,
    };
    if once {
        drain(&bus, &mut db, &trace, counting.get(&home))?;
        return Ok(());
    }
    push::keys(&home)?;
    let listening = runtime::listen(&home)?;
    if let Some(config) = factory.serve() {
        crate::serve::start(factory, config.listen)?;
    }
    if let Some(name) = embedder.clone() {
        let home = home.clone();
        std::thread::spawn(move || embed_all(&name, embed::Lazy::new(&home, true)));
    }
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
        let (_, kill) =
            take(&mut db, line, &trace, counting.get(&home)).map_err(|error| error.to_string())?;
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
    // A `run` command neither runs twice nor runs on unbounded: it ends `failed`, and its task moves on.
    for (id, group) in state::orphan_commands(&db)? {
        if let Some((pid, _)) =
            group.filter(|(pid, started)| memory::started(*pid) == Some(*started))
        {
            script::kill(pid);
        }
        let (out, err) = outputs_of(&home, id);
        let (stdout, stderr) = (script::collect(&out), script::collect(&err));
        tools::ran(&mut db, &trace, id, handler::INTERRUPTED, &stdout, &stderr)?;
    }
    // An answer's command never runs twice: one a stopped daemon left running ends `uncertain`.
    for (id, group) in state::orphan_answers(&db)? {
        if let Some((pid, _)) =
            group.filter(|(pid, started)| memory::started(*pid) == Some(*started))
        {
            script::kill(pid);
        }
        let (out, err) = outputs_of(&home, id);
        let (stdout, stderr) = (script::collect(&out), script::collect(&err));
        tools::ran(&mut db, &trace, id, tools::UNCERTAIN, &stdout, &stderr)?;
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
    let summaries = home.join("summaries");
    for (id, group) in summary::orphans(&db)? {
        if let Some((pid, _)) =
            group.filter(|(pid, started)| memory::started(*pid) == Some(*started))
        {
            script::kill(pid);
        }
        let output = runtime::answer(&summaries, id);
        let status = handler::INTERRUPTED;
        let counter = counting.get(&home);
        summary::answered(&mut db, &trace, id, output.as_deref(), status, counter)?;
        runtime::forget(&summaries, id);
    }
    let mut summarizing = Vec::new();
    let mut stalls = Stalls::default();
    let mut export = factory.otlp().map(|endpoint| Export {
        endpoint: endpoint.trim_end_matches('/').to_owned(),
        sending: None,
        after: Instant::now(),
    });
    let mut scripts = Vec::new();
    let mut commands = Vec::new();
    let mut deliveries = deliver::adopt(&db)?;
    let mut reads = Reads::default();
    let every = factory.sweep().every.unwrap_or(SWEEP);
    let mut swept: Option<Instant> = None;
    loop {
        drain(&bus, &mut db, &trace, counting.get(&home))?;
        while let Ok(line) = listening.lines.try_recv() {
            apply(&mut db, line, &trace, counting.get(&home))?;
        }
        start_answers(&mut db, &trace, &home, &mut commands)?;
        commands = run_commands(&mut db, &trace, &home, commands)?;
        tend(&mut db, &trace, &share, &home, &scope, &mut commands)?;
        handle_stalls(&mut db, &trace, &home, &mut stalls)?;
        let counter = counting.get(&home);
        summarize(&mut db, &trace, &summaries, &mut summarizing, counter)?;
        if let Some(export) = &mut export {
            send_spans(&db, &trace, &home, export)?;
        }
        deliver::tick(&db, factory, &home, &mut deliveries)?;
        read_states(&mut db, &mut reads)?;
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
            Ok(line) => apply(&mut db, line, &trace, counting.get(&home))?,
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return Err("the signal socket closed".into()),
        }
    }
}

/// How often the daemon looks again for a tokenizer it could not load.
const COUNTER_RETRY: Duration = Duration::from_secs(10);

/// The token counter of the factory's embedder, loaded from what the embedding thread downloaded.
/// The daemon's own thread never downloads: until the tokenizer is on disk, TL;DRs count words.
struct Counting {
    embedder: Option<String>,
    counter: Option<embed::Counter>,
    tried: Option<Instant>,
}

impl Counting {
    fn get(&mut self, home: &Path) -> Option<&embed::Counter> {
        let due = self.tried.is_none_or(|at| at.elapsed() >= COUNTER_RETRY);
        if let (None, Some(name), true) = (&self.counter, &self.embedder, due) {
            self.tried = Some(Instant::now());
            self.counter = embed::counter(name, home).ok().flatten();
        }
        self.counter.as_ref()
    }
}

/// How long the embedding thread waits between looks for TL;DRs to embed.
const EMBED_EVERY: Duration = Duration::from_secs(1);

/// How long the embedding thread waits after its embedder failed.
const EMBED_RETRY: Duration = Duration::from_secs(60);

/// Embeds, in the background, each task whose vector by the embedder `name` is missing or stale.
/// A failure is logged once, recorded for `priory search`, and retried later.
fn embed_all(name: &str, mut embedder: embed::Lazy) {
    let db = match state::open() {
        Ok(db) => db,
        Err(error) => return eprintln!("priory daemon: embeddings are unavailable: {error}"),
    };
    loop {
        let mut pass = || -> Result<(), String> {
            let stale = search::stale(&db, name).map_err(|error| error.to_string())?;
            for (task, text) in stale {
                let vector = embedder.embed(name, &text)?;
                search::store(&db, task, name, &text, &vector)
                    .map_err(|error| error.to_string())?;
            }
            Ok(())
        };
        let pass = pass();
        let wait = match &pass {
            Ok(()) => EMBED_EVERY,
            Err(_) => EMBED_RETRY,
        };
        match search::failed(&db, name, pass.as_ref().err().map(String::as_str)) {
            Ok(true) => {
                if let Err(error) = &pass {
                    eprintln!("priory daemon: embeddings are unavailable: {error}");
                }
            }
            Ok(false) => {}
            Err(error) => eprintln!("priory daemon: cannot record the embedder's state: {error}"),
        }
        std::thread::sleep(wait);
    }
}

/// Records the summarizer calls that ended, then calls the summarizer on the tasks that have no
/// TL;DR yet, at most `summary::CALLS` at once. A factory with no summarizer marks them skipped.
fn summarize(
    db: &mut Connection,
    trace: &Trace,
    dir: &Path,
    calls: &mut Vec<(i64, Script)>,
    counter: Option<&embed::Counter>,
) -> Result<(), Error> {
    for (id, mut script) in std::mem::take(calls) {
        let Some((status, log)) = script.poll() else {
            calls.push((id, script));
            continue;
        };
        let status = if status == "exit 0" || log.is_empty() {
            status
        } else {
            format!("{status}: {log}")
        };
        let output = runtime::answer(dir, id);
        summary::answered(db, trace, id, output.as_deref(), &status, counter)?;
        runtime::forget(dir, id);
    }
    let Some(model) = summary::model(trace.factory) else {
        for (task, ..) in summary::due(db, usize::MAX >> 1)? {
            summary::skipped(db, task, "no summarizer")?;
        }
        return Ok(());
    };
    for (task, title, request) in summary::due(db, summary::CALLS.saturating_sub(calls.len()))? {
        let id = summary::called(db, task)?;
        let prompt = summary::prompt(&title, &request);
        match runtime::ask_claude(dir, id, model, &prompt, HANDLER_TIMEOUT) {
            Ok(script) => {
                summary::spawned(db, id, script.pid(), memory::started(script.pid()))?;
                calls.push((id, script));
            }
            Err(error) => {
                let status = format!("cannot start: {error}");
                summary::answered(db, trace, id, None, &status, counter)?;
                runtime::forget(dir, id);
            }
        }
    }
    Ok(())
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
        match runtime::ask_claude(&dir, id, model, &prompt, HANDLER_TIMEOUT) {
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

/// Sends the trace records past the endpoint's cursor as OTLP spans, one request at a time. The
/// cursor moves only once `curl` reports success, so a failed send is retried after `EXPORT_RETRY`.
fn send_spans(
    db: &Connection,
    trace: &Trace,
    home: &Path,
    export: &mut Export,
) -> Result<(), Error> {
    if let Some((mut child, seq)) = export.sending.take() {
        let status = match child.try_wait() {
            Ok(None) => {
                export.sending = Some((child, seq));
                return Ok(());
            }
            Ok(Some(status)) => status,
            Err(error) => {
                eprintln!("priory daemon: sending spans: {error}");
                export.after = Instant::now() + EXPORT_RETRY;
                return Ok(());
            }
        };
        let read = |pipe: Option<&mut dyn std::io::Read>| {
            let mut text = String::new();
            pipe.map(|pipe| pipe.read_to_string(&mut text));
            text.trim().to_owned()
        };
        let code = read(child.stdout.as_mut().map(|pipe| pipe as _));
        let stderr = read(child.stderr.as_mut().map(|pipe| pipe as _));
        let endpoint = &export.endpoint;
        // OTLP/HTTP: a 4xx other than 408 and 429 means the request will never be accepted.
        let rejected = code.starts_with('4') && code != "408" && code != "429";
        if status.success() && code.starts_with('2') {
            otlp::sent(db, endpoint, seq)?;
        } else if status.success() && rejected {
            eprintln!(
                "priory daemon: {endpoint} rejected spans with HTTP {code}, so they are skipped"
            );
            otlp::sent(db, endpoint, seq)?;
        } else {
            let detail = if status.success() {
                format!("HTTP {code}")
            } else {
                status.to_string()
            };
            eprintln!("priory daemon: sending spans to {endpoint} failed ({detail}): {stderr}");
            export.after = Instant::now() + EXPORT_RETRY;
        }
    }
    if Instant::now() < export.after {
        return Ok(());
    }
    let Some((seq, request)) = otlp::pending(db, &export.endpoint, trace)? else {
        return Ok(());
    };
    let Some(request) = request else {
        otlp::sent(db, &export.endpoint, seq)?;
        return Ok(());
    };
    let body = home.join("otlp.json");
    let url = format!("{}/v1/traces", export.endpoint);
    let started =
        std::fs::write(&body, request.to_string()).and_then(|()| runtime::post_json(&url, &body));
    match started {
        Ok(child) => export.sending = Some((child, seq)),
        Err(error) => {
            eprintln!("priory daemon: cannot send spans to {url}: {error}");
            export.after = Instant::now() + EXPORT_RETRY;
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
fn apply(
    db: &mut Connection,
    line: Line,
    trace: &Trace,
    counter: Option<&embed::Counter>,
) -> Result<(), Error> {
    let (reply, kill) = take(db, &line.text, trace, counter)?;
    let _ = line.applied.send(reply);
    kill.inspect(|tmux| runtime::kill(tmux));
    Ok(())
}

/// What arrives on the socket: a hook's signal, a run's tool call, a person's answer, desk action
/// or snooze, a pick on a report, a task's progress, a memory gate message, a quick request, or a
/// browser that turns notifications on or off.
#[derive(serde::Deserialize)]
#[serde(untagged)]
enum Input {
    Add(AddLine),
    Signal(Signal),
    Call(Call),
    Answer(Answer),
    Dismiss(Dismiss),
    Act(Act),
    Snooze(Snooze),
    Choice(sections::Choice),
    Progress(progress::Put),
    Gate(Gate),
    Push(push::Line),
}

/// `POST /api/request`'s line: `{"add": {...}}`.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct AddLine {
    add: priory_intake::request::Add,
}

/// Publishes `add` as `priory add` does, then takes it in at once, so the reply names the task:
/// `{"key": ..., "id": ...}`, with no id when no active task has the key. A repeated text adds
/// `error`, which says nothing was added.
fn add(
    db: &mut Connection,
    add: &priory_intake::request::Add,
    trace: &Trace,
    counter: Option<&embed::Counter>,
) -> Result<Result<String, String>, Error> {
    use priory_intake::request::Refused;
    let (key, error) = match priory_intake::request::add(trace.factory, add) {
        Ok(key) => (key, None),
        Err(Refused::Invalid(refused)) => return Ok(Err(refused)),
        Err(Refused::Repeated(key)) => {
            let error = Refused::Repeated(key.clone()).to_string();
            (key, Some(error))
        }
    };
    drain(&Bus::open_default()?, db, trace, counter)?;
    let tx = db.transaction()?;
    let id = state::owner(&tx, std::slice::from_ref(&key))?.map(|task| task.id);
    drop(tx);
    let mut reply = serde_json::json!({ "key": key, "id": id });
    if let Some(error) = error {
        reply["error"] = serde_json::json!(error);
    }
    Ok(Ok(reply.to_string()))
}

/// The reads of wait conditions' current states that run on threads of their own, so a slow
/// Linear or GitHub read never holds the daemon's loop or a lock on the state file.
#[derive(Default)]
struct Reads {
    started: std::collections::HashMap<(Reader, i64), Instant>,
    done: Vec<std::thread::JoinHandle<(Reader, i64, Option<Read>)>>,
}

/// How often the daemon reads the state of a hold's unmet Linear issue again, as the intake polls.
const HOLD_READ_EVERY: Duration = Duration::from_secs(60);

/// Whose condition a read is for: a wait's or a hold's.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Reader {
    Wait,
    Hold,
}

/// A wait condition's current state, or why it could not be read.
type Read = Result<String, String>;

/// Starts a read for each open wait the daemon has not read yet, and for each unmet issue
/// condition of a hold once a minute, since the Linear stream misses a close before its position.
/// Fires the waits and meets the conditions whose finished read shows they already hold.
fn read_states(db: &mut Connection, reads: &mut Reads) -> Result<(), Error> {
    let holds = hold::readable(db)?.into_iter();
    let readable = tools::readable(db)?
        .into_iter()
        .map(|(wait, kind, resource)| (Reader::Wait, wait, kind, resource))
        .chain(holds.map(|(id, resource)| (Reader::Hold, id, "linear_state".to_owned(), resource)));
    for (reader, id, kind, resource) in readable {
        let due = match reads.started.get(&(reader, id)) {
            None => true,
            Some(at) => reader == Reader::Hold && at.elapsed() >= HOLD_READ_EVERY,
        };
        if !due {
            continue;
        }
        reads.started.insert((reader, id), Instant::now());
        reads.done.push(std::thread::spawn(move || {
            (reader, id, current_state(&kind, &resource))
        }));
    }
    let (finished, running) = std::mem::take(&mut reads.done)
        .into_iter()
        .partition::<Vec<_>, _>(|read| read.is_finished());
    reads.done = running;
    for read in finished {
        match read.join().expect("a state read does not panic") {
            (Reader::Wait, wait, Some(Ok(state))) => tools::read(db, wait, &state)?,
            (Reader::Hold, id, Some(Ok(state))) => hold::read(db, id, &state)?,
            (Reader::Wait, id, Some(Err(error))) => {
                eprintln!("priory daemon: cannot read the current state of wait {id}: {error}")
            }
            (Reader::Hold, id, Some(Err(error))) => eprintln!(
                "priory daemon: cannot read the current state of hold condition {id}: {error}"
            ),
            (_, _, None) => {}
        }
    }
    Ok(())
}

/// The current state of a wait's resource outside Priory: a Linear issue's state, when the daemon
/// has a Linear key, or a pull request's state on GitHub.
fn current_state(kind: &str, resource: &str) -> Option<Read> {
    match kind {
        "linear_state" => priory_intake::intake::linear_state(resource.strip_prefix("linear:")?),
        _ => {
            let (repo, number) = resource.strip_prefix("github:")?.rsplit_once('#')?;
            Some(runtime::pull_request_state(repo, number.parse().ok()?))
        }
    }
}

/// Applies one line and returns its reply, with a tmux session to kill. A signal's reply is `ok`;
/// a call's or an answer's is the JSON of its `Result`. A line that is none of them is reported
/// and dropped.
fn take(
    db: &mut Connection,
    line: &str,
    trace: &Trace,
    counter: Option<&embed::Counter>,
) -> Result<(String, Option<String>), Error> {
    let replied = |result: Result<String, String>| {
        serde_json::to_string(&result).expect("a result serializes")
    };
    match serde_json::from_str::<Input>(line) {
        Ok(Input::Add(line)) => Ok((replied(add(db, &line.add, trace, counter)?), None)),
        Ok(Input::Signal(signal)) => Ok(("ok".to_owned(), state::signal(db, &signal, trace)?)),
        Ok(Input::Call(call)) if call.tool == "progress" => {
            Ok((replied(progress::tool(db, &call, trace)?), None))
        }
        Ok(Input::Call(call)) => Ok((replied(tools::call(db, &call, trace.factory)?), None)),
        Ok(Input::Answer(answer)) => Ok((
            replied(tools::answer(db, &answer, trace, &|worktree, branch| {
                runtime::kept_head(&runtime::home(), worktree, branch)
            })?),
            None,
        )),
        Ok(Input::Dismiss(dismiss)) => {
            Ok((replied(tools::dismiss(db, &dismiss, trace.factory)?), None))
        }
        Ok(Input::Act(act)) => {
            let (reply, kill) = tools::act(db, &act, trace.factory)?;
            Ok((replied(reply), kill))
        }
        Ok(Input::Snooze(snooze)) => Ok((replied(snooze::snooze(db, &snooze, trace)?), None)),
        Ok(Input::Choice(choice)) => {
            Ok((replied(sections::pick(db, &choice, trace.factory)?), None))
        }
        Ok(Input::Progress(put)) => Ok((replied(progress::put(db, &put, trace)?), None)),
        Ok(Input::Push(line)) => Ok((replied(push::change(db, &line)?), None)),
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
    commands: &mut Vec<(i64, Script)>,
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
    perform(db, trace, home, commands)?;
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

/// Runs the action of the oldest task at a code step that does not wait for disk, once an
/// irreversible step's approval holds. An action that is not built yet blocks the task.
fn perform(
    db: &mut Connection,
    trace: &Trace,
    home: &Path,
    commands: &mut Vec<(i64, Script)>,
) -> Result<(), Error> {
    for due in state::actionable(db, &trace.factory.workflows)? {
        if !act(db, trace, home, commands, &due)? {
            break;
        }
    }
    Ok(())
}

/// Runs `due`'s action, or returns true when it waits for disk to add its worktree back.
fn act(
    db: &mut Connection,
    trace: &Trace,
    home: &Path,
    commands: &mut Vec<(i64, Script)>,
    due: &state::Due,
) -> Result<bool, Error> {
    let factory = trace.factory;
    let step = factory.workflows[&due.workflow]
        .value
        .step(&due.step)
        .expect("actionable checked the step");
    // The sweep or a retirement removed the worktree; its branch still holds the work. Only
    // `open-pr` pushes from it and `run` works in it, so no other action waits for disk to add it back.
    if let Some(repo) = repo_of(factory, &due.trigger, due.repo.as_deref())
        && matches!(step.action.as_deref(), Some("open-pr" | "run"))
        && due
            .worktree
            .as_deref()
            .is_some_and(|worktree| !Path::new(worktree).exists())
    {
        match restore_worktree(db, home, repo, due.task)? {
            Restored::Ready => {}
            Restored::Waits => return Ok(true),
            Restored::Failed(reason) => {
                state::block(db, trace, due.task, "blocked", &reason)?;
                return Ok(false);
            }
        }
    }
    let head = due
        .worktree
        .as_deref()
        .zip(due.branch.as_deref())
        .and_then(|(worktree, branch)| runtime::kept_head(home, Path::new(worktree), branch));
    if !tools::approved(db, due.task, step, head.as_deref())? {
        return Ok(false);
    }
    let done = match step.action.as_deref().unwrap_or_default() {
        "open-pr" => open_pr(db, factory, due, head.as_deref())?,
        "open-issue" => open_issue(db, factory, due)?,
        "run" => {
            start_command(db, trace, home, due, step, commands)?;
            return Ok(false);
        }
        other => {
            let reason = format!("action {other} is not built yet");
            state::block(db, trace, due.task, "blocked", &reason)?;
            return Ok(false);
        }
    };
    tools::performed(db, due.task, &due.workflow, step, done, trace)?;
    Ok(false)
}

/// Where a removed worktree stands once `restore_worktree` looked at it.
enum Restored {
    Ready,
    Waits,
    Failed(String),
}

/// Adds back the worktree of `task`, which the sweep or a retirement removed, from its branch once
/// there is disk room; a task waiting for room says so in its reason.
fn restore_worktree(
    db: &Connection,
    home: &Path,
    repo: &str,
    task: i64,
) -> Result<Restored, Error> {
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
            rusqlite::params![task, reason],
        )?;
        return Ok(Restored::Waits);
    }
    if let Err(reason) = runtime::worktree(home, repo, task) {
        return Ok(Restored::Failed(reason));
    }
    db.execute("UPDATE tasks SET retired = NULL WHERE id = ?1", [task])?;
    Ok(Restored::Ready)
}

/// Stores the task's `run` action, then starts its command with `sh -c` in the task's worktree, or
/// the factory folder when it has none, bounded by the step's `timeout` or `command_timeout`.
fn start_command(
    db: &mut Connection,
    trace: &Trace,
    home: &Path,
    due: &state::Due,
    step: &factory::Step,
    commands: &mut Vec<(i64, Script)>,
) -> Result<(), Error> {
    let factory = trace.factory;
    let id = tools::launched(db, due.task, step)?;
    let folder = std::path::absolute(&factory.dir)?;
    let dir = due.worktree.as_deref().map(Path::new).unwrap_or(&folder);
    let env = command_env(home, factory, due.task)?;
    let env: Vec<_> = env
        .iter()
        .map(|(name, value)| (*name, value.as_os_str()))
        .collect();
    let run = step.run.as_deref().unwrap_or_default();
    let (out, err) = outputs_of(home, id);
    let timeout = step.timeout.unwrap_or(factory.command_timeout());
    match script::spawn_held(&["sh", "-c", run, "sh"], &env, dir, &out, &err, timeout) {
        Ok(mut child) => {
            state::command_spawned(db, id, child.pid(), memory::started(child.pid()))?;
            child.release();
            commands.push((id, child));
        }
        Err(error) => tools::ran(db, trace, id, &format!("cannot start: {error}"), "", "")?,
    }
    Ok(())
}

/// The environment a `run` action's or an answer's command gets: the task, Priory home, the state
/// file and the factory folder, each absolute.
fn command_env(
    home: &Path,
    factory: &Factory,
    task: i64,
) -> Result<[(&'static str, std::ffi::OsString); 4], Error> {
    let state_file = std::path::absolute(std::env::var_os("PRIORY_DB").unwrap_or_default())?;
    Ok([
        ("PRIORY_TASK", task.to_string().into()),
        ("PRIORY_HOME", std::path::absolute(home)?.into()),
        ("PRIORY_DB", state_file.into()),
        ("PRIORY_FACTORY", std::path::absolute(&factory.dir)?.into()),
    ])
}

/// Starts the command of each answer stored to run it, in the folder it was asked with, bounded by
/// `command_timeout`. Its argv is exec'd, never parsed by a shell.
fn start_answers(
    db: &mut Connection,
    trace: &Trace,
    home: &Path,
    commands: &mut Vec<(i64, Script)>,
) -> Result<(), Error> {
    for (id, task, stored) in state::unstarted_answers(db)? {
        let ran: crate::command::Command =
            serde_json::from_str(&stored).expect("a stored command parses");
        // A cancel or a revive between the answer and this tick leaves it unwanted: it never starts.
        if !state::command_wanted(db, id)? {
            tools::ran(db, trace, id, "not started: the task moved on", "", "")?;
            continue;
        }
        if ran.target == crate::command::WORKTREE && !ran.dir.exists() {
            let (trigger, repo): (String, Option<String>) = db.query_row(
                "SELECT trigger, repo FROM tasks WHERE id = ?1",
                [task],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            let restored = match repo_of(trace.factory, &trigger, repo.as_deref()) {
                Some(repo) => restore_worktree(db, home, repo, task)?,
                None => Restored::Failed(format!("task {task} names no repo")),
            };
            match restored {
                Restored::Ready => {}
                Restored::Waits => continue,
                Restored::Failed(reason) => {
                    tools::ran(db, trace, id, &format!("cannot start: {reason}"), "", "")?;
                    continue;
                }
            }
        }
        let env = command_env(home, trace.factory, task)?;
        let env: Vec<_> = env
            .iter()
            .map(|(name, value)| (*name, value.as_os_str()))
            .collect();
        let program = ran.program();
        let argv: Vec<&str> = std::iter::once(program.as_str())
            .chain(ran.argv[1..].iter().map(String::as_str))
            .collect();
        let (out, err) = outputs_of(home, id);
        let timeout = trace.factory.command_timeout();
        match script::spawn_held(&argv, &env, &ran.dir, &out, &err, timeout) {
            Ok(mut child) => {
                state::command_spawned(db, id, child.pid(), memory::started(child.pid()))?;
                child.release();
                commands.push((id, child));
            }
            Err(error) => tools::ran(db, trace, id, &format!("cannot start: {error}"), "", "")?,
        }
    }
    Ok(())
}

/// Where `run` action `id` writes its stdout and its stderr.
fn outputs_of(home: &Path, id: i64) -> (std::path::PathBuf, std::path::PathBuf) {
    let dir = home.join("actions");
    (dir.join(format!("{id}.out")), dir.join(format!("{id}.err")))
}

/// Records each `run` command that ended and moves its task. A command whose task left its step, as
/// a cancel does, is killed. Returns the commands still running.
fn run_commands(
    db: &mut Connection,
    trace: &Trace,
    home: &Path,
    commands: Vec<(i64, Script)>,
) -> Result<Vec<(i64, Script)>, Error> {
    let mut still = Vec::new();
    for (id, mut command) in commands {
        if !state::command_wanted(db, id)? {
            script::kill(command.pid());
        }
        match command.poll() {
            Some((status, stdout)) => {
                let stderr = script::collect(&outputs_of(home, id).1);
                tools::ran(db, trace, id, &status, &stdout, &stderr)?;
            }
            None => still.push((id, command)),
        }
    }
    Ok(still)
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
    let env: Vec<String> = factory
        .otlp()
        .map(|_| {
            let ids = format!("priory.task.id={},priory.run.id={run}", admission.task);
            match std::env::var("OTEL_RESOURCE_ATTRIBUTES") {
                Ok(own) if !own.is_empty() => format!("OTEL_RESOURCE_ATTRIBUTES={own},{ids}"),
                _ => format!("OTEL_RESOURCE_ATTRIBUTES={ids}"),
            }
        })
        .into_iter()
        .collect();
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
            env: &env,
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

fn drain(
    bus: &Bus,
    db: &mut Connection,
    trace: &Trace,
    counter: Option<&embed::Counter>,
) -> Result<(), Error> {
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
            let ended = state::handle(db, &delivery.message, trace, counter)?;
            bus.ack(delivery.connection, delivery.seq)?;
            ended.inspect(|tmux| runtime::kill(tmux));
        }
    }
}
