use crate::enter::Entering;
use crate::idle;
use crate::link::Link;
use crate::memory::{self, Gate};
use crate::metrics;
use crate::otlp;
use crate::reload::{self, Polled};
use crate::room::Share;
use crate::route;
use crate::runners;
use crate::snooze::{self, Snooze};
use crate::state::{self, Admission, NewRun, Signal};
use crate::tools::{self, Act, Answer, Call, Dismiss, Done, Repair};
use crate::trace::Trace;
use crate::upgrade::Upgrader;
use crate::{
    deliver, embed, handler, hold, notify, overlap, progress, push, search, sections, stall,
    subscribe, summary, sweep,
};
use priory_factory::factory::{self, Factory, Harness, OnChange, PermissionMode, Session};
use priory_intake::intake::{self, OUTPUT};
use priory_intake::log::{self, Log};
use priory_runtime::runtime::{self, Launch, Line};
use priory_runtime::script::{self, Script};
use priory_runtime::{room, slice, transcript, upgrade};
use rusqlite::{Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use std::cell::OnceCell;
use std::collections::HashMap;
use std::path::Path;
use std::sync::mpsc::{RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

// shortcut: polls the bus file; switch to herdr-bus-client's notify with the systemd service.
const EVERY: Duration = Duration::from_secs(1);
/// How often the daemon removes the worktrees of ended tasks.
const SWEEP: Duration = Duration::from_secs(600);
/// How long the queue stays stalled after its scripts before the sweeper starts, unless `after` says.
const AFTER: Duration = Duration::from_secs(1800);
/// How often the daemon looks for merged tasks a `held` scorer is due to grade.
const HELD: Duration = Duration::from_secs(60);
/// How long a sweep script may run when it sets no `timeout`.
const SCRIPT_TIMEOUT: Duration = Duration::from_secs(300);
/// How long the daemon waits to publish a pick's pending request again after a publish failed.
const REDISPATCH: Duration = Duration::from_secs(10);
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
    /// The file holding the header lines each request carries.
    headers: Option<std::path::PathBuf>,
    sending: Option<(std::process::Child, otlp::Cursor)>,
    after: Instant,
}

type Error = Box<dyn std::error::Error>;

/// Handles every message on `factory.intake` past the daemon's cursor. With `once`, stops there.
/// Otherwise it also runs agents: it starts runs, types their prompts and takes their signals.
pub fn run(factory: &Factory, once: bool) -> Result<(), String> {
    serve(factory, once).map_err(|error| error.to_string())
}

/// Why a view-only machine refuses a write on its socket.
pub const VIEW_ONLY: &str = "this machine is view-only: it runs no task and changes none here. \
     Answer or dismiss an ask as <runner>:<ask>, or act on a task as <runner>:<key>, which goes to \
     its runner through the log";

/// Reads the whole log into mirrors of every runner's asks and tasks, and serves them when the
/// factory has `[serve]`. It claims, runs and publishes nothing: a person's answer, dismiss or task
/// action goes to its runner through the log, from the server or the command line.
pub fn view_only(factory: &Factory) -> Result<(), String> {
    view(factory).map_err(|error| error.to_string())
}

fn view(factory: &Factory) -> Result<(), Error> {
    let watch = reload::Watch::new(factory);
    let machine = room::hostname();
    let viewer = runners::viewer(factory, machine.as_deref())?;
    let mut db = state::open()?;
    let bus = log::open(factory.log())?;
    let trace = Trace {
        factory,
        revision: runtime::revision(&factory.dir),
        runner: None,
        machine,
        usage: transcript::usage,
    };
    // The listings add every runner's asks and tasks once this state file names its machine.
    runners::name(&db, Some(&viewer))?;
    let consumer = runners::view_consumer(&viewer);
    // The socket's lock keeps a second daemon off this state file.
    let listening = runtime::listen(&runtime::home())?;
    let server = match factory.serve() {
        Some(config) => Some(crate::serve::start(factory, config.listen)?),
        None => None,
    };
    let mut live = Live::new(factory, watch, trace.revision.clone(), server);
    let mut link = Link::default();
    loop {
        live.poll(&mut db, &trace, false)?;
        let trace = live.trace(&trace);
        if link.due() {
            let drained = view_drain(&*bus, &consumer, &mut db, &viewer, &trace);
            metered(&*bus, drained.as_ref());
            link.tried(&db, drained)?;
        }
        match listening.lines.recv_timeout(EVERY) {
            Ok(line) => {
                let reply = match serde_json::from_str::<Input>(&line.text) {
                    // Since you looked is this machine's own.
                    Ok(Input::Seen(line)) => state::seen(&db, &line.seen.person, line.seen.at)
                        .map_err(|error| error.to_string()),
                    _ => Err(VIEW_ONLY.to_owned()),
                };
                let reply = serde_json::to_string(&reply).expect("a result serializes");
                let _ = line.applied.send(reply);
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return Err("the signal socket closed".into()),
        }
    }
}

/// Reads every message past the view's cursor into its mirrors.
fn view_drain(
    bus: &dyn Log,
    consumer: &str,
    db: &mut Connection,
    viewer: &str,
    trace: &Trace,
) -> Result<(), Error> {
    let mut probe = Some(Instant::now());
    loop {
        let deliveries = read(bus, consumer, &mut probe)?;
        if deliveries.is_empty() {
            return Ok(());
        }
        for delivery in deliveries {
            let message = runners::clocked(delivery.message.clone(), delivery.logged);
            runners::view(db, &message, viewer, trace)?;
            bus.ack(consumer, &delivery)?;
        }
    }
}

fn serve(factory: &Factory, once: bool) -> Result<(), Error> {
    // Taken first, so an edit made while the daemon starts still reloads.
    let watch = reload::Watch::new(factory);
    runtime::bound_commands(factory.command_timeout());
    if !once {
        upgrade::admit(&runtime::home(), true)?;
    }
    let mut db = state::open()?;
    let bus = log::open(factory.log())?;
    tools::backfill_endings(&mut db, &factory.workflows)?;
    state::bands(&mut db, &factory.bands(), factory.age())?;
    state::fairness(&db, factory.fair())?;
    state::irreversible_asks(&mut db, &factory.workflows)?;
    let embedder = factory.search().embedder().map(|spec| embed::name(&spec));
    search::configure(&db, embedder.as_deref())?;
    let home = runtime::home();
    let mut counting = Counting {
        embedder: embedder.clone(),
        counter: None,
        tried: None,
    };
    let machine = room::hostname();
    let trace = Trace {
        factory,
        revision: runtime::revision(&factory.dir),
        runner: runners::runner(factory, machine.as_deref())?,
        machine,
        usage: transcript::usage,
    };
    let consumer = runners::consumer(trace.runner.as_deref());
    runners::name(&db, trace.runner.as_deref())?;
    // What came before writes the state alone, which a rollback puts back. A dispatch reaches further.
    let held = match !once && upgrade::held(&home) {
        true => Some(hold(factory, &home)?),
        false => None,
    };
    for error in sections::dispatch(&db, factory, None)? {
        eprintln!("priory daemon: {error}");
    }
    if once {
        drain(&*bus, &consumer, &mut db, &trace, counting.get(&home))?;
        runners::flush(&db, &*bus)?;
        crate::held::tick(&mut db, &trace, intake::now()?)?;
        match state::waiting(&db)? {
            0 => {}
            1 => println!("1 task waits: --once only routes; a running daemon starts it"),
            waiting => {
                println!("{waiting} tasks wait: --once only routes; a running daemon starts them")
            }
        }
        return Ok(());
    }
    // The kit's and the memories repository's fetches come before any run, so the first runs get
    // their commits.
    let mut kit = crate::kit::Fetchers::new(&db, factory, &home)?;
    kit.start(&db)?;
    // Without its key the daemon sends no notification, but still runs tasks.
    if let Err(error) = push::keys(&home) {
        eprintln!("priory daemon: notifications are off: {error}");
    }
    let searching = search::answering(state::read_only()?, home.clone());
    let listening = match held {
        Some(listening) => listening,
        None => runtime::listen(&home)?,
    };
    let headers = otlp::header_file(factory, &home, "otlp-headers")?;
    let server = match factory.serve() {
        Some(config) => Some(crate::serve::start(factory, config.listen)?),
        None => None,
    };
    if let Some(name) = embedder.clone() {
        let home = home.clone();
        std::thread::spawn(move || embed_all(&name, embed::Lazy::new(&home, true)));
    }
    let share = Share::of(factory, trace.machine.as_deref());
    let slice = slice::name();
    let claimed = slice::claim(&slice, &home);
    let scope = slice::control_group(&slice)
        .and_then(|_| match &claimed {
            Ok(()) => slice::cap(&slice, share.cpu, share.memory),
            Err(_) => Ok(()),
        })
        .and_then(|()| slice::scope(&slice))
        .unwrap_or_else(|error| {
            eprintln!(
                "priory daemon: no systemd user manager, so runs start outside {slice} and only the room check caps them: {error}"
            );
            Vec::new()
        });
    sweep::clear_refusal(&home);
    let mut limits = (!scope.is_empty()).then(|| sweep::Limits::new(&slice, &home, claimed.err()));
    let mut starting = HashMap::new();
    // Checked once, as the slice is: a jailed task never starts unjailed where this fails.
    let probe = || {
        priory_runtime::jail::probe().inspect_err(|error| {
            eprintln!(
                "priory daemon: this runner cannot start bubblewrap, so jailed tasks block: {error}"
            );
        })
    };
    let mut probed = factory.sandboxed();
    let mut jailing = if probed { probe() } else { Ok(()) };
    state::uncertain(&db)?;
    crate::keyboard::adopt(&mut db, &trace)?;
    // No one watches a compaction a stopped daemon started, so it never types its `/compact`.
    for tmux in idle::tmux_sessions(&db)? {
        runtime::kill(&tmux);
    }
    for live in idle::live(&db)? {
        idle::end(&db, live.id, "interrupted")?;
    }
    // A crash between ending a run and killing its session leaves its agent running.
    let ended = state::ended_sessions(&db)?;
    for (tmux, _) in runtime::tagged()
        .into_iter()
        .filter(|pair| ended.contains(pair))
    {
        runtime::kill(&tmux);
    }
    runtime::replay(&home, |line| {
        let (_, kill) = take(&mut db, line, &trace, counting.get(&home), kit.current())
            .map_err(|error| error.to_string())?;
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
    // A recipe never runs on unbounded: one a stopped daemon left running ends, and a setup or
    // resume it ends blocks its task.
    crate::recipe::orphans(&mut db, &trace, &home)?;
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
        headers,
        sending: None,
        after: Instant::now(),
    });
    let mut scripts = Vec::new();
    let mut commands = Vec::new();
    let mut recipes = crate::recipe::Running::default();
    let mut probes = crate::connectors::Probes::default();
    let mut records = crate::evidence::Records::default();
    let mut deliveries = deliver::adopt(&db)?;
    let mut attaching = Vec::new();
    let mut pushing = push::Sending::default();
    let mut desktop = factory
        .desktop_notifications()
        .then(notify::Desktop::default);
    let mut reads = Reads::default();
    let mut peeked = tools::Peeked::default();
    let mut grading = crate::gates::Grading::default();
    let every = factory.sweep().every.unwrap_or(SWEEP);
    let mut swept: Option<Instant> = None;
    let mut reclaiming = sweep::Reclaim::default();
    let mut retires = false;
    let mut held: Option<Instant> = None;
    let mut alive = None;
    let mut aged = None;
    let mut link = Link::default();
    let mut redispatch: Option<Instant> = None;
    let mut entering = Entering::default();
    let mut live = Live::new(factory, watch, trace.revision.clone(), server);
    let mut upgrader = Upgrader::default();
    loop {
        let tick = Instant::now();
        if live.poll(&mut db, &trace, true)? {
            let factory = live.factory();
            state::bands(&mut db, &factory.bands(), factory.age())?;
            state::fairness(&db, factory.fair())?;
            state::irreversible_asks(&mut db, &factory.workflows)?;
            // A reload that jails a workflow probes as a start would.
            if !probed && factory.sandboxed() {
                probed = true;
                jailing = probe();
            }
        }
        let trace = live.trace(&trace);
        let factory = trace.factory;
        // A runner back from away reads the log, and so drops the tasks that moved, before any run
        // and before it publishes what it queued meanwhile.
        let drained = link.due().then(|| {
            let counter = counting.get(&home);
            drain_probed(&*bus, &consumer, &mut db, &trace, counter, true)
        });
        if let Some(me) = &trace.runner {
            runners::tick(&mut db, me, intake::now()?, &mut alive)?;
        }
        if let Some(drained) = drained {
            let tried = drained.and_then(|()| runners::flush(&db, &*bus));
            metered(&*bus, tried.as_ref());
            let reached = tried.is_ok();
            link.tried(&db, tried)?;
            // A pick's request that a lost log refused goes out once the log answers again.
            if reached && redispatch.is_none_or(|after| Instant::now() >= after) {
                let failed = sections::dispatch(&db, factory, None)?;
                for error in &failed {
                    eprintln!("priory daemon: {error}");
                }
                redispatch = (!failed.is_empty()).then(|| Instant::now() + REDISPATCH);
            }
        }
        while let Ok(line) = listening.lines.try_recv() {
            apply(
                &mut db,
                line,
                &trace,
                counting.get(&home),
                kit.current(),
                &searching,
            )?;
        }
        start_answers(&mut db, &trace, &home, &mut commands)?;
        commands = run_commands(&mut db, &trace, &home, commands)?;
        // A teardown that ended lets the sweep remove its worktree now, not a sweep later.
        if crate::recipe::tick(&mut db, &trace, &home, &mut recipes)? {
            swept = None;
        }
        crate::evidence::tick(&mut db, factory, &home, &mut records)?;
        kit.tick(&db)?;
        let fetched = kit.current();
        tend(
            &mut db,
            &trace,
            &share,
            &home,
            &scope,
            &jailing,
            &mut commands,
            &mut recipes,
            &mut probes,
            fetched,
            &mut entering,
            &mut attaching,
            &mut starting,
        )?;
        // Committed first, so a crash before the kill leaves an ended run, not a lost one.
        for tmux in tools::over_budget_in_turn(&mut db, &trace, &mut peeked, &watched)? {
            runtime::kill(&tmux);
        }
        compact_idle(&db, &trace, &home, &scope, fetched)?;
        handle_stalls(&mut db, &trace, &home, &mut stalls)?;
        let counter = counting.get(&home);
        summarize(&mut db, &trace, &summaries, &mut summarizing, counter)?;
        if let Some(export) = &mut export {
            send_spans(&db, &trace, &home, export)?;
        }
        deliver::tick(&db, factory, &home, &mut deliveries)?;
        crate::attach::tick(&mut db, &trace, &mut attaching)?;
        push::tick(&db, &home, &mut pushing)?;
        if let Some(desktop) = &mut desktop {
            notify::tick(&db, desktop)?;
        }
        snooze::tick(&mut db, &trace)?;
        state::age_queue(&db, &mut aged)?;
        read_states(&mut db, &mut reads)?;
        let head = |worktree: &Path, branch: &str| runtime::kept_head(&home, worktree, branch);
        for tmux in crate::gates::tick(&mut db, &trace, &home, &mut grading, &head)? {
            runtime::kill(&tmux);
        }
        if sweep_scripts(&db, factory, &home, &mut scripts)? {
            swept = None;
        }
        let sweep = factory.sweep();
        if let (Some(_), Some(workflow)) = (&sweep.agent, factory.workflows.get(factory::SWEEP)) {
            let after = sweep.after.unwrap_or(AFTER);
            let band = factory.bands().first().copied();
            stall::call_sweeper(&mut db, after, band, &workflow.value.start, &home)?;
        }
        if held.is_none_or(|at| at.elapsed() >= HELD) {
            held = Some(Instant::now());
            crate::held::tick(&mut db, &trace, intake::now()?)?;
        }
        let due = swept.is_none_or(|at| at.elapsed() >= every);
        if due {
            swept = Some(Instant::now());
            sweep::run(&db, factory, &home)?;
            if let Some(limits) = &mut limits {
                limits.check(&slice, &share, &home);
            }
            sweep::measure(&home);
        }
        let under = || {
            room::measure(&home)
                .is_ok_and(|room| crate::room::under_reclaim(&room, factory.sweep().reclaim))
        };
        reclaiming.tick(&mut db, factory, &home, &trace, due, under)?;
        // A retirement costs a worktree, so it waits for what the reclaim frees.
        retires |= due;
        if retires && reclaiming.settled() {
            retires = false;
            let short = || room::measure(&home).is_ok_and(|room| crate::room::short_of_disk(&room));
            sweep::retire(&db, factory, &home, short)?;
        }
        let working = !(commands.is_empty()
            && recipes.is_empty()
            && scripts.is_empty()
            && summarizing.is_empty()
            && stalls.calls.is_empty()
            && deliveries.is_empty());
        upgrader.tick(&mut db, &trace, &home, working);
        priory_runtime::ticks::record(tick.elapsed());
        // A pasted prompt's Enter is due before the next whole tick.
        let wait = entering.due().map_or(EVERY, |due| due.min(EVERY));
        match listening.lines.recv_timeout(wait) {
            Ok(line) => apply(
                &mut db,
                line,
                &trace,
                counting.get(&home),
                kit.current(),
                &searching,
            )?,
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return Err("the signal socket closed".into()),
        }
    }
}

/// The factory the daemon's loop reads: the one it started with, until an edit reloads it.
struct Live<'a> {
    start: &'a Factory,
    reloaded: Option<Factory>,
    /// A valid edit the guard refused, and the digest of its files: it applies once no task is at
    /// a workflow or step it removes.
    held: Option<(Box<Factory>, reload::Digest)>,
    watch: reload::Watch,
    revision: Option<String>,
    server: Option<crate::serve::Server>,
}

impl<'a> Live<'a> {
    fn new(
        start: &'a Factory,
        watch: reload::Watch,
        revision: Option<String>,
        server: Option<crate::serve::Server>,
    ) -> Self {
        Live {
            start,
            reloaded: None,
            held: None,
            watch,
            revision,
            server,
        }
    }

    fn factory(&self) -> &Factory {
        self.reloaded.as_ref().unwrap_or(self.start)
    }

    /// `base` with the live factory and its revision.
    fn trace(&self, base: &Trace) -> Trace<'_> {
        Trace {
            factory: self.factory(),
            revision: self.revision.clone(),
            runner: base.runner.clone(),
            machine: base.machine.clone(),
            usage: base.usage,
        }
    }

    /// Loads an edit of the factory's files once it passes every rule, and with `guard`, once it
    /// strands no task. An edit the guard refused stays held and applies once its tasks leave,
    /// unless a newer edit is settling.
    /// Returns whether a new factory is live; a refused edit keeps the last good one. Each
    /// refusal is reported once, to stderr and to the trace, and so is each reload.
    fn poll(&mut self, db: &mut Connection, base: &Trace, guard: bool) -> Result<bool, Error> {
        let polled = self.watch.poll();
        if self.watch.committed() {
            self.revision = runtime::revision(&self.factory().dir);
        }
        let (new, files, fresh) = match polled {
            // A newer edit is settling: it replaces or drops the held one once it holds.
            Polled::Same if self.watch.settling() => return Ok(false),
            Polled::Same => match self.held.take() {
                Some((new, files)) => (new, files, false),
                None => return Ok(false),
            },
            Polled::Current => {
                self.held = None;
                return Ok(false);
            }
            Polled::Refused(problems) => {
                self.held = None;
                reload::report("daemon", Err(&problems));
                reload::record(db, &self.trace(base), Err(&problems))?;
                return Ok(false);
            }
            // The newer edit replaces the held one, and holds itself if it strands a task.
            Polled::Loaded(new, files) => {
                self.held = None;
                (new, files, true)
            }
        };
        let stranded = match guard {
            true => reload::stranded(db, &new)?,
            false => Vec::new(),
        };
        if !stranded.is_empty() {
            if fresh {
                reload::report("daemon", Err(&stranded));
                reload::record(db, &self.trace(base), Err(&stranded))?;
            }
            self.held = Some((new, files));
            return Ok(false);
        }
        let restart = reload::restart_only(self.factory(), &new);
        if let Some(server) = &self.server {
            server.update(&new);
        }
        self.watch.applied(&new, files);
        self.revision = runtime::revision(&new.dir);
        self.reloaded = Some(*new);
        reload::report("daemon", Ok(&restart));
        reload::record(db, &self.trace(base), Ok(&restart))?;
        Ok(true)
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
    if let Some((mut child, cursor)) = export.sending.take() {
        let status = match child.try_wait() {
            Ok(None) => {
                export.sending = Some((child, cursor));
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
            otlp::sent(db, endpoint, cursor)?;
        } else if status.success() && rejected {
            eprintln!(
                "priory daemon: {endpoint} rejected spans with HTTP {code}, so they are skipped"
            );
            otlp::sent(db, endpoint, cursor)?;
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
    let Some((cursor, request)) = otlp::pending(db, &export.endpoint, trace)? else {
        return Ok(());
    };
    let Some(request) = request else {
        otlp::sent(db, &export.endpoint, cursor)?;
        return Ok(());
    };
    let body = home.join("otlp.json");
    let url = format!("{}/v1/traces", export.endpoint);
    let started = std::fs::write(&body, request.to_string())
        .and_then(|()| runtime::post_json(&url, export.headers.as_deref(), &body));
    match started {
        Ok(child) => export.sending = Some((child, cursor)),
        Err(error) => {
            eprintln!("priory daemon: cannot send spans to {url}: {error}");
            export.after = Instant::now() + EXPORT_RETRY;
        }
    }
    Ok(())
}

/// Records the sweep scripts that ended, then starts the scripts of a stall that just opened, for
/// its resource, and the scripts due on their `every`. Returns whether that stall is short of disk,
/// which the built-in sweep frees too.
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
    let scripts = &factory.sweep().scripts;
    let scheduled = scripts.iter().filter(|script| script.every.is_some());
    stall::prune(db, scheduled.map(|script| script.run.as_str()))?;
    let swept = match stall::unswept(db)? {
        Some((stall, resource)) => {
            for script in scripts
                .iter()
                .filter(|script| script.when.is_some_and(|when| when.name() == resource))
            {
                if !stall::adopt(db, stall, &script.run)? {
                    start_script(db, home, Some(stall), script, running)?;
                }
            }
            stall::swept(db)?;
            resource == "disk"
        }
        None => false,
    };
    for script in scripts {
        if let Some(every) = script.every
            && stall::due(db, &script.run, every)?
        {
            start_script(db, home, None, script, running)?;
        }
    }
    Ok(swept)
}

/// Starts a sweep script for `stall`, or on its schedule without one.
fn start_script(
    db: &Connection,
    home: &Path,
    stall: Option<i64>,
    script: &factory::Script,
    running: &mut Vec<(i64, Script)>,
) -> Result<(), Error> {
    let id = stall::started(db, stall, &script.run)?;
    let timeout = script.timeout.unwrap_or(SCRIPT_TIMEOUT);
    match script::spawn(&script.run, home, &log_of(home, id), timeout) {
        Ok(child) => {
            stall::spawned(db, id, child.pid(), memory::started(child.pid()))?;
            running.push((id, child));
        }
        Err(error) => stall::ended(db, id, &format!("cannot start: {error}"), "")?,
    }
    Ok(())
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
    kit: crate::kit::Current<'_>,
    searching: &Sender<search::Asked>,
) -> Result<(), Error> {
    if let Ok(call) = serde_json::from_str::<Call>(&line.text)
        && call.tool == "search"
    {
        let _ = searching.send((call, line.applied));
        return Ok(());
    }
    let (reply, kill) = take(db, &line.text, trace, counter, kit)?;
    let _ = line.applied.send(reply);
    kill.inspect(|tmux| runtime::kill(tmux));
    Ok(())
}

/// What arrives on the socket: a hook's signal, a run's tool call, a person's answer, desk action
/// or snooze, a pick or a comment on a report, a task's progress, a memory gate message, a quick
/// request, a browser that turns notifications on or off, a visit to the desk, or a report read.
#[derive(serde::Deserialize)]
#[serde(untagged)]
enum Input {
    Version(VersionLine),
    Guard(crate::guard::Line),
    Private(crate::private::Line),
    Evidence(crate::evidence::AddLine),
    Connector(crate::todo::ConnectorLine),
    Scoped(crate::scoped::Line),
    Todo(crate::todo::Line),
    Subscribe(subscribe::Line),
    Unsubscribe(subscribe::EndLine),
    Label(crate::label::Line),
    Deliver(deliver::Line),
    Add(AddLine),
    Outside(crate::outside::Line),
    Signal(Signal),
    Call(Call),
    Answer(Answer),
    Dismiss(Dismiss),
    Act(Act),
    Snooze(Snooze),
    Choice(sections::Choice),
    Comment(sections::Comment),
    Progress(progress::Put),
    Gate(Gate),
    Proof(crate::proof::Line),
    Push(push::Line),
    Seen(state::SeenLine),
    Read(crate::post::ReadLine),
    BatchEdit(crate::batch::EditLine),
    Move(state::MoveLine),
    Keyboard(crate::keyboard::TakeLine),
    KeyboardReturn(crate::keyboard::ReturnLine),
    Keys(crate::keyboard::KeysLine),
    Attach(crate::keyboard::AttachLine),
}

/// `{"version": true}`, which the helper of an upgrade sends.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct VersionLine {
    #[serde(rename = "version")]
    _version: bool,
}

/// This daemon's version, as its reply to a `VersionLine`.
fn version(_: &VersionLine) -> Result<String, String> {
    Ok(env!("CARGO_PKG_VERSION").to_owned())
}

/// Waits until a migration's upgrade is committed. The daemon then answers its version alone, so a
/// rollback to the backups loses nothing.
fn hold(factory: &Factory, home: &Path) -> Result<runtime::Listening, Error> {
    let listening = runtime::listen(home)?;
    if let Some(config) = factory.serve() {
        drop(std::net::TcpListener::bind(config.listen)?);
    }
    while upgrade::uncommitted(home) {
        match listening.lines.recv_timeout(EVERY) {
            Ok(line) => {
                let reply = match serde_json::from_str::<Input>(&line.text) {
                    Ok(Input::Version(line)) => version(&line),
                    _ => Err(upgrade::IN_PROGRESS.to_owned()),
                };
                let reply = serde_json::to_string(&reply).expect("a result serializes");
                let _ = line.applied.send(reply);
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return Err("the signal socket closed".into()),
        }
    }
    Ok(listening)
}

/// `POST /api/request`'s line: `{"add": {...}}`.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct AddLine {
    add: priory_intake::request::Add,
}

/// Publishes `add` as `priory add` does, then takes it in at once, so the reply names the task and
/// what the request did: `{"key": ..., "id": ..., "outcome": ...}`, with no id when no active task
/// has the key. A repeated text adds `error`, which says nothing was added, in place of `outcome`.
fn add(
    db: &mut Connection,
    add: &priory_intake::request::Add,
    trace: &Trace,
    counter: Option<&embed::Counter>,
) -> Result<Result<String, String>, Error> {
    use priory_intake::request::Refused;
    if let Some(missing) = crate::relate::missing(db, &add.relates)? {
        return Ok(Err(missing));
    }
    let (key, error) = match priory_intake::request::add(trace.factory, add) {
        Ok(key) => (key, None),
        Err(Refused::Invalid(refused)) => return Ok(Err(refused)),
        Err(Refused::Repeated(key)) => {
            let error = Refused::Repeated(key.clone()).to_string();
            (key, Some(error))
        }
    };
    let log = log::open(trace.factory.log())?;
    let consumer = runners::consumer(trace.runner.as_deref());
    // A request the log took but did not hand back yet: the loop takes it in once the log answers.
    match drain(&*log, &consumer, db, trace, counter).and_then(|()| runners::flush(db, &*log)) {
        Err(error) if !log::unreachable(&*error) => return Err(error),
        _ => {}
    }
    let tx = db.transaction()?;
    let id = state::owner(&tx, std::slice::from_ref(&key))?.map(|task| task.id);
    drop(tx);
    let mut reply = serde_json::json!({ "key": key, "id": id });
    match error {
        Some(error) => reply["error"] = serde_json::json!(error),
        None => reply["outcome"] = serde_json::json!(added(db, &key)?),
    }
    Ok(Ok(reply.to_string()))
}

/// What the newest request for `key` did: `created` its task, `said` its text to the task's agent,
/// or `kept` it for the task's next run. None while the log has not handed it back.
fn added(db: &Connection, key: &str) -> rusqlite::Result<Option<String>> {
    db.query_row(
        "SELECT CASE WHEN outcome IN ('said', 'kept') THEN outcome
           WHEN (SELECT outcome FROM inbox WHERE type = 'factory.task.requested' AND subject = ?1
             ORDER BY seq DESC LIMIT 1) = 'created' THEN 'created' END
         FROM inbox WHERE type = ?2 AND subject = ?1 ORDER BY seq DESC LIMIT 1",
        rusqlite::params![key, priory_intake::request::REQUESTED],
        |row| row.get(0),
    )
    .optional()
    .map(Option::flatten)
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
    kit: crate::kit::Current<'_>,
) -> Result<(String, Option<String>), Error> {
    let replied = |result: Result<String, String>| {
        serde_json::to_string(&result).expect("a result serializes")
    };
    match serde_json::from_str::<Input>(line) {
        Ok(Input::Version(line)) => Ok((replied(version(&line)), None)),
        Ok(Input::Guard(line)) => {
            Ok((replied(crate::guard::decide(db, &line.guard, trace)?), None))
        }
        Ok(Input::Private(_)) => {
            let private = trace.factory.private().cloned().unwrap_or_default();
            let rules = serde_json::to_string(&private).expect("the rules serialize");
            Ok((replied(Ok(rules)), None))
        }
        Ok(Input::Evidence(line)) => Ok((
            replied(crate::evidence::add(
                db,
                &line.evidence,
                trace.factory,
                &runtime::home(),
                &|worktree, branch| runtime::kept_head(&runtime::home(), worktree, branch),
                &runtime::checked_out,
            )?),
            None,
        )),
        Ok(Input::Connector(line)) => {
            crate::todo::connector_used(db, &line.connector_used)?;
            Ok((replied(Ok(String::new())), None))
        }
        Ok(Input::Scoped(line)) => Ok((replied(crate::scoped::sent(db, &line.scoped)?), None)),
        Ok(Input::Todo(line)) => Ok((
            replied(crate::todo::change(db, &line, trace.factory)?),
            None,
        )),
        Ok(Input::Subscribe(line)) => Ok((replied(subscribe::apply(db, &line.subscribe)?), None)),
        Ok(Input::Unsubscribe(line)) => {
            let end = &line.unsubscribe;
            Ok((
                replied(subscribe::unsubscribe(db, end.task, &end.resource)?),
                None,
            ))
        }
        Ok(Input::Label(line)) => {
            let labeled = intake::now()
                .and_then(|now| crate::label::apply(db, trace.factory, &line.label, now));
            Ok((replied(labeled), None))
        }
        Ok(Input::BatchEdit(line)) => {
            Ok((replied(crate::batch::apply(db, &line.batch_edit)), None))
        }
        Ok(Input::Move(line)) => Ok((replied(state::move_line(db, line.r#move)), None)),
        Ok(Input::Keyboard(line)) => Ok((
            replied(crate::keyboard::take(db, &line.keyboard, trace)?),
            None,
        )),
        Ok(Input::KeyboardReturn(line)) => Ok((
            replied(crate::keyboard::give_back(
                db,
                &line.keyboard_return,
                trace,
            )?),
            None,
        )),
        Ok(Input::Keys(line)) => Ok((replied(crate::keyboard::keys(db, &line.keys, trace)?), None)),
        Ok(Input::Attach(line)) => Ok((
            replied(crate::keyboard::attach_issued(
                db,
                &line.attach_issued,
                trace,
            )?),
            None,
        )),
        Ok(Input::Deliver(line)) => Ok((replied(deliver::act(db, &line)?), None)),
        Ok(Input::Add(line)) => Ok((replied(add(db, &line.add, trace, counter)?), None)),
        Ok(Input::Outside(line)) => {
            let call = &line.outside;
            let reply = match call.tool.as_str() {
                "add" => match crate::outside::request(call) {
                    Ok(request) => add(db, &request, trace, counter)?,
                    Err(refused) => Err(refused),
                },
                "answer" | "dismiss" => {
                    crate::outside::decide(db, call, trace, &|worktree, branch| {
                        runtime::kept_head(&runtime::home(), worktree, branch)
                    })?
                }
                _ => crate::outside::apply(db, call, trace.factory, &runtime::home())?,
            };
            Ok((replied(reply), None))
        }
        Ok(Input::Signal(signal)) => Ok((
            "ok".to_owned(),
            state::signal(db, &signal, trace, &watched)?,
        )),
        Ok(Input::Call(call))
            if let Some(refused) =
                crate::consult::unserved(db, &call.session, &call.tool, trace.factory)? =>
        {
            Ok((replied(Err(refused)), None))
        }
        Ok(Input::Call(call)) if call.tool == "report" => Ok((
            replied(sections::tool(db, &call.session, &call.arguments)),
            None,
        )),
        Ok(Input::Call(call)) if call.tool == "relate" => Ok((
            replied(crate::relate::tool(db, &call.session, &call.arguments)),
            None,
        )),
        Ok(Input::Call(call)) if call.tool == "tldr" => Ok((
            replied(search::tldr(db, &call.session, &call.arguments, counter)),
            None,
        )),
        Ok(Input::Call(call)) if call.tool == "progress" => {
            Ok((replied(progress::tool(db, &call, trace)?), None))
        }
        Ok(Input::Call(call)) if matches!(call.tool.as_str(), "remember" | "forget") => Ok((
            replied(crate::memories::write(db, &call, trace, kit)?),
            None,
        )),
        Ok(Input::Call(call)) if call.tool == "memory" => Ok((
            replied(crate::memories::recall(db, &call, trace.factory, kit)?),
            None,
        )),
        Ok(Input::Call(call)) if call.tool == "finding" => Ok((
            replied(crate::findings::tool(db, &call, trace.factory)?),
            None,
        )),
        Ok(Input::Call(call)) if matches!(call.tool.as_str(), "subscribe" | "unsubscribe") => {
            Ok((replied(subscribe::tool(db, &call)?), None))
        }
        Ok(Input::Call(call)) => {
            let (reply, kill) = tools::call(db, &call, trace, kit, &|worktree, branch| {
                runtime::kept_head(&runtime::home(), worktree, branch)
            })?;
            Ok((replied(reply), kill))
        }
        Ok(Input::Proof(line)) => Ok((
            replied(crate::proof::gate(
                db,
                &line,
                trace,
                &|worktree, branch| runtime::kept_head(&runtime::home(), worktree, branch),
            )?),
            None,
        )),
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
            let (reply, kill) = tools::act(db, &act, trace)?;
            Ok((replied(reply), kill))
        }
        Ok(Input::Snooze(snooze)) => Ok((replied(snooze::snooze(db, &snooze, trace)?), None)),
        Ok(Input::Choice(choice)) => Ok((replied(sections::pick(db, &choice, trace)?), None)),
        Ok(Input::Comment(comment)) => Ok((
            replied(sections::comment(db, &comment, trace.factory)?),
            None,
        )),
        Ok(Input::Progress(put)) => Ok((replied(progress::put(db, &put, trace)?), None)),
        Ok(Input::Push(line)) => Ok((replied(push::change(db, &line)?), None)),
        Ok(Input::Read(line)) => Ok((
            replied(crate::post::read(db, line.read).map(|()| format!("{} read", line.read))),
            None,
        )),
        Ok(Input::Seen(line)) => Ok((
            replied(Ok(state::seen(db, &line.seen.person, line.seen.at)?)),
            None,
        )),
        Ok(Input::Gate(gate)) => {
            let share = Share::of(trace.factory, trace.machine.as_deref());
            let room = || room::measure(&runtime::home());
            let reply = memory::gate(db, &gate, trace.factory, &share, room)?;
            Ok((replied(reply), None))
        }
        Err(error) => {
            // Never the line: a keys line holds what a person typed, maybe a password.
            eprintln!(
                "priory daemon: dropped a line of {} bytes: {error}",
                line.len()
            );
            Ok((replied(Err(format!("unreadable line: {error}"))), None))
        }
    }
}

/// Ends runs that exited, timed out or wait on a prompt, fires due waits, starts every run that
/// fits in the room, and types prompts.
#[allow(clippy::too_many_arguments)]
fn tend(
    db: &mut Connection,
    trace: &Trace,
    share: &Share,
    home: &Path,
    scope: &[String],
    jailing: &Result<(), String>,
    commands: &mut Vec<(i64, Script)>,
    recipes: &mut crate::recipe::Running,
    probes: &mut crate::connectors::Probes,
    kit: crate::kit::Current<'_>,
    entering: &mut Entering,
    attaching: &mut Vec<crate::attach::Attaching>,
    starting: &mut HashMap<i64, Option<slice::Cpu>>,
) -> Result<(), Error> {
    let factory = trace.factory;
    let live = state::live(db)?;
    starting.retain(|id, _| live.iter().any(|run| run.id == *id && !run.ready));
    for mut run in live {
        if !run.ready && !scope.is_empty() {
            starting
                .entry(run.id)
                .or_insert_with(|| slice::cpu(&slice::name()));
        }
        if run.direnv == 1
            && let Some(error) = runtime::direnv_failed(home, &run.session)
        {
            direnv_failed(db, trace, &run, &error)?;
            run.direnv = 2;
        }
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
        } else if let Some((ended_as, reason, missed)) = verdict(
            &run,
            factory,
            home,
            &|| crate::trace::granted(db, run.id),
            &|| slice::throttled(&slice::name(), starting.get(&run.id).copied()??),
        ) {
            let worktree = run.worktree.as_deref().unwrap_or_default();
            let harness = harness_of(factory, &run.agent);
            // The harness may have rewritten its config from an older copy, dropping the grant.
            let held = missed.then(|| priory_runtime::trust::holds(harness, Path::new(worktree)));
            let overwritten = matches!(held, Some(Ok(false)));
            let reason = if overwritten {
                format!(
                    "{} asked whether to trust {worktree}: it overwrote the trust Priory recorded \
                     before it read it",
                    harness.program()
                )
            } else {
                reason
            };
            if missed && let Some(agent) = factory.agents.get(&run.agent) {
                let grant = priory_runtime::trust::Grant {
                    path: worktree.into(),
                    file: std::path::PathBuf::new(),
                    outcome: priory_runtime::trust::Outcome::Missed,
                    why: match &held {
                        Some(Ok(false)) => Some("the grant was gone from the config".to_owned()),
                        Some(Err(why)) => {
                            Some(format!("Priory could not read the grant back: {why}"))
                        }
                        _ => None,
                    },
                };
                let harness = agent.value.harness.name();
                let version = runtime::version(harness_of(factory, &run.agent).program());
                let tx = db.transaction()?;
                crate::trace::trusted(&tx, trace, run.id, harness, &grant, version.as_deref())?;
                tx.commit()?;
            }
            // A debate turn past its timeout goes to the person, as a debate that did not converge.
            if ended_as == "timeout"
                && let Some(tmux) = crate::debate::timed_out(db, trace, &run.session, &reason)?
            {
                runtime::kill(&tmux);
                continue;
            }
            // Committed first, so a crash before the kill leaves an ended run, not a lost one.
            state::block(db, trace, run.task, ended_as, &reason)?;
            runtime::kill(&run.tmux);
            if overwritten {
                // The next start writes the grant again; a second loss fails the task.
                let repair = (!tools::spent(db, run.id)?).then(|| Repair {
                    cause: format!("{} overwrote Priory's trust grant", harness.program()),
                    action: "restart",
                    gone: true,
                });
                tools::fail(db, trace, run.task, run.id, &reason, repair.as_ref())?;
            }
        }
    }
    state::due(db, factory, &|worktree, branch| {
        runtime::kept_head(home, Path::new(worktree), branch)
    })?;
    overlap::tick(db, factory, home)?;
    crate::stacked::pushed(db, trace, home)?;
    tools::enter_queued(db, factory)?;
    state::disk_due(db, || room::measure(home).ok().map(|room| room.disk_free))?;
    memory::reclaim(db)?;
    perform(db, trace, home, commands)?;
    crate::keyboard::sweep(db, trace)?;
    let now: String = db.query_row("SELECT strftime('%Y-%m-%dT%H:%M:%SZ', 'now')", [], |row| {
        row.get(0)
    })?;
    // Before admission, so an investigator past its deadline never starts.
    for tmux in crate::consult::tick(db, trace)? {
        runtime::kill(&tmux);
    }
    // Before the outbox types, so a run between turns is held before its next prompt.
    for tmux in crate::spend::tick(db, trace, &now, &watched)? {
        runtime::kill(&tmux);
    }
    // Before admission, so a run's slow start never delays another run's Enter.
    entering.tick(db)?;
    // The last measurement of this tick, for the metrics.
    let mut last = None;
    let (promised, waiting) = loop {
        // Measured again after each start, since its clone and worktree take disk.
        let measured = OnceCell::new();
        let room = || measured.get_or_init(|| room::measure(home));
        let checking = |task| probes.checking(task);
        let pass = state::admissible(
            db,
            factory,
            trace.runner.as_deref(),
            share,
            room,
            &watched,
            &checking,
        )?;
        last = measured.into_inner().or(last);
        let Some(admission) = pass.admission else {
            break (pass.promised, Some(pass.waiting));
        };
        if let Some(me) = &trace.runner {
            match runners::cleared(db, me, admission.task)? {
                Some(true) => {}
                Some(false) => continue,
                // Its claim is not on the log yet: the next tick tries again. The pass stopped
                // short of the tasks behind it, so it counts no waiting task.
                None => break (pass.promised, None),
            }
        }
        if let Err(reason) = start(
            db,
            trace,
            trace.runner.as_deref(),
            home,
            scope,
            jailing,
            &admission,
            recipes,
            probes,
            kit,
            attaching,
        )? {
            state::block(db, trace, admission.task, "failed", &reason)?;
        }
    };
    if metrics::wants_room(last.is_some()) {
        let room = last.unwrap_or_else(|| room::measure(home));
        metrics::sample_room(db, share, &room, promised.memory, waiting);
    }
    // A marked run idle between turns has no turn end to wait for.
    for session in crate::hold::idle(db)? {
        if let Some(tmux) = tools::hold_idle(db, trace, &session)? {
            runtime::kill(&tmux);
        }
    }
    for session in crate::preempt::idle(db)? {
        if let Some(tmux) = tools::preempt_idle(db, trace, &session, &watched)? {
            runtime::kill(&tmux);
        }
    }
    // Enter goes on a later tick, so the agent never reads it as part of the paste.
    while let Some(attempt) = state::attempt(db)? {
        let pasted = runtime::pane(&attempt.tmux, &attempt.session)
            .ok_or_else(|| "its pane is gone".to_owned())
            .and_then(|pane| runtime::paste(&pane, &attempt.text).map(|()| pane));
        match pasted {
            Ok(pane) => entering.pasted(attempt.outbox, pane, attempt.compact),
            Err(error) => eprintln!("priory daemon: typing into {}: {error}", attempt.tmux),
        }
    }
    Ok(())
}

/// Whether a person may be typing in the tmux session `name`: a client that can type is attached,
/// or tmux cannot tell. A read-only view, as Herdr opens by itself, protects nothing.
fn watched(name: &str) -> bool {
    runtime::clients(name).is_none_or(|clients| clients > 0)
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
            sweep::run(db, factory, home)?;
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

/// Records that direnv failed with `error` before run `run`'s harness started, which then started
/// without the `.envrc`: in the trace and as its task's reason.
fn direnv_failed(
    db: &mut Connection,
    trace: &Trace,
    run: &state::LiveRun,
    error: &str,
) -> Result<(), Error> {
    let error = crate::screen::plain(error);
    let secrets = crate::screen::secrets(&trace.factory.secret_envs());
    let error = crate::screen::redact(&error, &secrets, None)
        .map_or_else(|| "[redacted]".to_owned(), |(shown, _)| shown);
    let why = if error.is_empty() {
        "direnv exited before the harness started".to_owned()
    } else {
        error
    };
    let grant = priory_runtime::trust::Grant {
        path: run.worktree.as_deref().unwrap_or_default().into(),
        file: std::path::PathBuf::new(),
        outcome: priory_runtime::trust::Outcome::Failed,
        why: Some(why.clone()),
    };
    let tx = db.transaction()?;
    crate::trace::trusted(&tx, trace, run.id, "direnv", &grant, None)?;
    state::direnv(&tx, run.id, 2)?;
    tx.execute(
        "UPDATE tasks SET reason = ?2 WHERE id = ?1",
        rusqlite::params![run.task, format!("started without its .envrc: {why}")],
    )?;
    tx.commit()?;
    Ok(())
}

/// Why a live run must end: its harness's trust dialog, no `SessionStart`, a permission prompt, or
/// a turn past its step's timeout. The flag says the dialog asked for trust that `granted`, the
/// run's own grant, had recorded. `throttled` says what the slice's CPU quota kept back.
fn verdict(
    run: &state::LiveRun,
    factory: &Factory,
    home: &Path,
    granted: &dyn Fn() -> bool,
    throttled: &dyn Fn() -> Option<String>,
) -> Option<(&'static str, String, bool)> {
    // Claude and Codex ask whether to trust a folder, and Codex its hooks, before any hook; Priory
    // never answers.
    let harness = harness_of(factory, &run.agent);
    let trust = || {
        let clone = run
            .worktree
            .as_deref()
            .and_then(|worktree| runtime::clone_of(Path::new(worktree)))
            .map_or_else(|| "<clone>".to_owned(), |clone| clone.display().to_string());
        let exe = runtime::exe().unwrap_or_else(|_| "priory".into());
        let worktrees = home.join("worktrees").display().to_string();
        harness.trust(&exe, &clone, &worktrees)
    };
    let rows = (!run.ready).then(|| runtime::screen(&run.tmux).join("\n"));
    if let Some(rows) = &rows
        && harness.asks_trust(rows)
    {
        if granted() {
            let reason = format!(
                "{} asked whether to trust {} although Priory recorded its trust; this version of \
                 the harness keys trust differently. Trust the folder by hand, or turn \
                 trust_worktrees off",
                harness.program(),
                run.worktree.as_deref().unwrap_or("its folder"),
            );
            return Some(("exited", reason, true));
        }
        let trust = trust();
        let reason = format!(
            "{} asked whether to trust {}; Priory never answers it. {}{}",
            harness.program(),
            run.worktree.as_deref().unwrap_or("its folder"),
            trust[..1].to_uppercase(),
            &trust[1..]
        );
        return Some(("exited", reason, false));
    }
    let late = run.waited.and_then(|waited| {
        // Read only past the shortest window, since each read is a query.
        let trusted = waited >= state::ready_within(0, false) && granted();
        let within = state::ready_within(run.direnv, trusted);
        (waited >= within).then_some((within, trusted))
    });
    if let Some((within, trusted)) = late {
        let program = harness.program();
        let mut reason = if run.direnv == 1 && runtime::direnv_loading(home, &run.session) {
            format!(
                "direnv did not finish loading {}/.envrc within {within}s, so {program} never \
                 started; warm it with `direnv exec <worktree> true`, or fix the .envrc",
                run.worktree.as_deref().unwrap_or("its folder"),
            )
        } else {
            let ready = rows.as_deref().is_some_and(|rows| harness.at_prompt(rows));
            let known = trusted && !harness.reviews_hooks();
            let recorded = if known {
                " although Priory recorded its trust"
            } else {
                ""
            };
            let advice = match (ready || known, throttled()) {
                (true, Some(throttled)) => throttled,
                (true, None) if ready => "its prompt is ready and asks for no trust, so its \
                                          SessionStart hook is late or never reached the daemon"
                    .to_owned(),
                (true, None) => "it started too slowly".to_owned(),
                (false, Some(throttled)) => format!("{}; {throttled}", trust()),
                (false, None) => trust(),
            };
            format!("{program} showed no SessionStart within {within}s{recorded}; {advice}")
        };
        reason.push_str(". Resume the task to try again");
        let rows = rows.unwrap_or_default();
        let secrets = crate::screen::secrets(&factory.secret_envs());
        // When a secret still shows after redaction, no row of the pane is kept.
        let shown = crate::screen::redact(&rows, &secrets, None)
            .map_or_else(|| "[redacted]".to_owned(), |(shown, _)| shown);
        eprintln!(
            "priory daemon: run {} never started; its pane ended with:\n{shown}",
            run.id,
        );
        let screen: Vec<_> = shown.lines().collect();
        let tail: Vec<_> = screen[screen.len().saturating_sub(5)..]
            .iter()
            .map(|line| line.trim())
            .collect();
        if !tail.is_empty() {
            reason.push_str(&format!(". Its pane ended with: {}", tail.join(" | ")));
        }
        return Some(("exited", reason, false));
    }
    if let Some(message) = &run.prompted {
        let reason = format!("run {} stopped on a permission prompt: {message}", run.id);
        return Some(("permission", reason, false));
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
        ("timeout", reason, false)
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

/// Runs `due`'s action, or returns true when it waits, for disk to add its worktree back or for
/// the log to say this runner keeps it, so the next task's action may run.
fn act(
    db: &mut Connection,
    trace: &Trace,
    home: &Path,
    commands: &mut Vec<(i64, Script)>,
    due: &state::Due,
) -> Result<bool, Error> {
    // A code step's action is a start like a run's: it waits until the log says this runner keeps it.
    if let Some(me) = &trace.runner
        && runners::cleared(db, me, due.task)? != Some(true)
    {
        return Ok(true);
    }
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
        match restore_worktree(
            db,
            trace.factory,
            home,
            repo,
            trace.runner.as_deref(),
            due.task,
        )? {
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
    if step.effect == factory::Effect::Irreversible && factory.staged(&due.trigger) {
        let preview = preview(db, factory, due, step, head.as_deref())?;
        tools::staged(db, due.task, step, &preview)?;
        return Ok(false);
    }
    let done = match step.action.as_deref().unwrap_or_default() {
        "open-pr" => open_pr(db, factory, due, head.as_deref())?,
        "open-issue" => open_issue(db, factory, due)?,
        factory::OPEN_MEMORIES_PR => crate::memories::open_pull_request(
            db,
            factory,
            home,
            repo_of(factory, &due.trigger, due.repo.as_deref()),
        )?,
        "start-batch" => {
            start_batch(db, trace, due, step)?;
            return Ok(false);
        }
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

/// The task's worktree folder and branch name, `priory/<name>`: the branch it has, else
/// `<runner>-<task>` with runners, whose task ids overlap, else `<task>`.
fn worktree_name(db: &Connection, runner: Option<&str>, task: i64) -> rusqlite::Result<String> {
    let branch: Option<String> =
        db.query_row("SELECT branch FROM tasks WHERE id = ?1", [task], |row| {
            row.get(0)
        })?;
    Ok(
        match (
            branch
                .as_deref()
                .and_then(|branch| branch.strip_prefix("priory/")),
            runner,
        ) {
            (Some(name), _) => name.to_owned(),
            (None, Some(runner)) => format!("{runner}-{task}"),
            (None, None) => task.to_string(),
        },
    )
}

/// The branch of the task that `task` stacks on, while that task's stack has a pull request open
/// from it; once it merged, `task` starts from and lands on the default branch.
fn base_branch(db: &Connection, task: i64) -> rusqlite::Result<Option<String>> {
    db.query_row(
        "SELECT base.branch FROM tasks JOIN tasks AS base ON base.id = tasks.base_task
         WHERE tasks.id = ?1 AND EXISTS (SELECT 1 FROM pull_requests
           WHERE pull_requests.task = base.id AND pull_requests.branch = base.branch
           AND pull_requests.state = 'open')",
        [task],
        |row| row.get(0),
    )
    .optional()
}

/// Whether the branch of the task that `task` stacks on moved since the `waiting` moment recorded
/// it, before `task`'s first run: then `task` is held until a person answers its ask.
fn parent_moved(db: &mut Connection, home: &Path, task: i64) -> rusqlite::Result<bool> {
    let recorded: Option<(i64, String, String, Option<String>)> = db
        .query_row(
            "SELECT tasks.base_task, tasks.parent_branch, tasks.parent_head, parent.worktree
             FROM tasks JOIN tasks AS parent ON parent.id = tasks.base_task
             WHERE tasks.id = ?1 AND tasks.parent_head IS NOT NULL
             AND NOT EXISTS (SELECT 1 FROM runs WHERE runs.task = tasks.id)",
            [task],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()?;
    let Some((parent, branch, was, worktree)) = recorded else {
        return Ok(false);
    };
    // A branch that is gone refuses the start when the worktree is added.
    let Some(now) = worktree
        .and_then(|worktree| runtime::kept_head(home, Path::new(&worktree), &branch))
        .filter(|now| *now != was)
    else {
        return Ok(false);
    };
    let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    crate::hold::parent_moved(&tx, task, parent, &branch, &was, &now)?;
    tx.commit()?;
    Ok(true)
}

/// Starts `due`'s batch and records the action in one transaction, so a crash between them never
/// creates its tasks twice.
fn start_batch(
    db: &mut Connection,
    trace: &Trace,
    due: &state::Due,
    step: &factory::Step,
) -> Result<(), Error> {
    let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let done = match crate::batch::start(&tx, due.task, trace.factory)? {
        Ok(()) => Done {
            status: "started",
            pull_request: None,
            commit: None,
            error: None,
            stdout: None,
            stack: Vec::new(),
        },
        Err(error) => failed(error),
    };
    tools::performed_in(&tx, due.task, &due.workflow, step, done, trace)?;
    tx.commit()?;
    Ok(())
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
    factory: &Factory,
    home: &Path,
    repo: &str,
    runner: Option<&str>,
    task: i64,
) -> Result<Restored, Error> {
    // A resurrected task goes through the room check, which a code step otherwise skips.
    let waits = match room::measure(home) {
        Err(error) => Some(format!(
            "{}: cannot measure it: {error}",
            crate::room::RESTORES
        )),
        Ok(room) if crate::room::short_of_disk(&room) => Some(crate::room::RESTORES.to_owned()),
        Ok(_) => None,
    };
    if let Some(reason) = waits {
        db.execute(
            "UPDATE tasks SET reason = ?2 WHERE id = ?1 AND reason IS NOT ?2",
            rusqlite::params![task, reason],
        )?;
        return Ok(Restored::Waits);
    }
    match runtime::worktree(home, repo, &worktree_name(db, runner, task)?, None) {
        Ok((worktree, _, added)) => {
            state::set_base(db, task, added.base())?;
            db.execute(
                "UPDATE tasks SET reason = NULL WHERE id = ?1 AND substr(reason, 1, length(?2)) = ?2",
                rusqlite::params![task, crate::room::RESTORES],
            )?;
            // The code step needs no resume; the task's next run starts it first.
            let resume = factory.recipe(repo, factory::Moment::Resume);
            if let (runtime::Added::Restored, Some(recipe)) = (&added, resume) {
                crate::recipe::owe(
                    db,
                    factory,
                    task,
                    factory::Moment::Resume,
                    &worktree,
                    recipe,
                )?;
            }
        }
        Err(reason) => return Ok(Restored::Failed(reason)),
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
pub(crate) fn command_env(
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
/// its step's `timeout` or `command_timeout`. Its argv is exec'd, never parsed by a shell.
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
                Some(repo) => {
                    restore_worktree(db, trace.factory, home, repo, trace.runner.as_deref(), task)?
                }
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
        let timeout = ran
            .timeout
            .map_or_else(|| trace.factory.command_timeout(), Duration::from_secs);
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

/// Pushes each branch of the task's stack, bottom first, and opens or reuses its pull request on
/// the one below, titled and described by its latest result. The top branch is the task's own, at
/// `commit`, which its approval covers; each lower one must be below it.
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
        (
            Some(repo),
            Some((worktree, commit)),
            Some(state::Described {
                summary,
                evidence,
                branches,
            }),
        ) => {
            let own = due
                .branch
                .as_deref()
                .expect("a commit comes from the branch");
            let branches = branches.unwrap_or_else(|| vec![own.to_owned()]);
            let cap = factory.workflows[&due.workflow].value.max_prs;
            if let Some(cap) = cap.filter(|cap| branches.len() > cap.get()) {
                return Ok(failed(format!(
                    "the stack has {} branches, past max_prs = {cap} of workflow {}",
                    branches.len(),
                    due.workflow
                )));
            }
            let (title, body) = titled(&summary, evidence.as_deref());
            let body = with_evidence(db, factory, due.task, body)?;
            let base = base_branch(db, due.task)?;
            let stack = Stack {
                branches: &branches,
                base: base.as_deref(),
            };
            let worktree = Path::new(worktree);
            factory
                .private()
                .map_or(Ok(()), |private| {
                    crate::private::screen_stack(
                        private, worktree, repo, commit, stack.base, title, &body,
                    )
                })
                .and_then(|()| open_stack(worktree, repo, &stack, commit, title, &body))
        }
    };
    Ok(match opened {
        Ok(stack) => Done {
            status: "opened",
            pull_request: stack.last().map(|pull| pull.resource.clone()),
            commit: commit.map(str::to_owned),
            error: None,
            stdout: None,
            stack,
        },
        Err(error) => failed(error),
    })
}

/// The title and body of the pull request or issue a result describes: the first line of its
/// summary, then the summary and its evidence.
fn titled<'s>(summary: &'s str, evidence: Option<&str>) -> (&'s str, String) {
    let title = summary.lines().next().unwrap_or_default();
    let body = match evidence {
        Some(evidence) => format!("{summary}\n\n{evidence}"),
        None => summary.to_owned(),
    };
    (title, body)
}

/// `body` with the task's Evidence section after it, when the task has items to list.
fn with_evidence(
    db: &Connection,
    factory: &Factory,
    task: i64,
    body: String,
) -> Result<String, Error> {
    let url = factory.serve().and_then(|serve| serve.url.as_deref());
    Ok(match crate::evidence::section(db, task, url)? {
        Some(section) => format!("{body}\n\n{section}"),
        None => body,
    })
}

/// What the irreversible code step `step` of a staged task would do, without doing it: the pull
/// requests or issue it would open with their title and body, or the command it would run.
fn preview(
    db: &Connection,
    factory: &Factory,
    due: &state::Due,
    step: &factory::Step,
    commit: Option<&str>,
) -> Result<String, Error> {
    let action = step.action.as_deref().unwrap_or_default();
    let repo = repo_of(factory, &due.trigger, due.repo.as_deref());
    let described = state::last_result(db, due.task)?;
    let text = match (action, repo, described) {
        ("run", ..) => format!("would run: {}", step.run.as_deref().unwrap_or_default()),
        ("open-pr" | "open-issue", None, _) => {
            format!("would fail: task {} names no repo", due.task)
        }
        ("open-pr" | "open-issue", _, None) => {
            "would fail: no result describes what it opens".to_owned()
        }
        ("open-pr" | "open-issue", Some(repo), Some(described)) => {
            let (title, body) = titled(&described.summary, described.evidence.as_deref());
            let body = match action {
                "open-pr" => with_evidence(db, factory, due.task, body)?,
                _ => body,
            };
            let mut lines = Vec::new();
            if action == "open-issue" {
                lines.push(format!("would open an issue on {repo}"));
            } else {
                let own = due.branch.clone().unwrap_or_default();
                let branches = described.branches.unwrap_or_else(|| vec![own]);
                let mut below = match base_branch(db, due.task)? {
                    Some(base) => base,
                    None => due
                        .worktree
                        .as_deref()
                        .and_then(|worktree| runtime::default_branch(Path::new(worktree)).ok())
                        .unwrap_or_else(|| "the default branch".to_owned()),
                };
                for branch in branches {
                    lines.push(format!(
                        "would open a pull request on {repo} from {branch} into {below}"
                    ));
                    below = branch;
                }
                lines.push(format!("at: {}", commit.unwrap_or("no commit yet")));
            }
            lines.push(format!("title: {title}"));
            lines.push(format!("body:\n{body}"));
            lines.join("\n")
        }
        (other, ..) => format!("would run the {other} action"),
    };
    Ok(text)
}

pub(crate) fn failed(error: String) -> Done {
    Done {
        status: "failed",
        pull_request: None,
        commit: None,
        error: Some(error),
        stdout: None,
        stack: Vec::new(),
    }
}

/// The branches of a task's stack, bottom first, and the branch its bottom one is based on, when
/// not the default one.
struct Stack<'a> {
    branches: &'a [String],
    base: Option<&'a str>,
}

/// Opens the pull request of each of the stack's branches, bottom first, each on the one below.
fn open_stack(
    worktree: &Path,
    repo: &str,
    stack: &Stack,
    commit: &str,
    title: &str,
    body: &str,
) -> Result<Vec<tools::Stacked>, String> {
    let branches = stack.branches;
    let mut opened: Vec<tools::Stacked> = Vec::new();
    for (at, branch) in branches.iter().enumerate() {
        let head = match at + 1 == branches.len() {
            true => commit.to_owned(),
            false => runtime::head(worktree, branch)
                .filter(|head| runtime::is_ancestor(worktree, head, commit))
                .ok_or_else(|| {
                    format!("branch {branch} is not below the approved commit {commit}")
                })?,
        };
        let title = match branches.len() {
            1 => title.to_owned(),
            count => format!("{title} ({} of {count})", at + 1),
        };
        let below = opened
            .last()
            .map(|pull| pull.branch.as_str())
            .or(stack.base);
        let (number, base) =
            runtime::open_pull_request(worktree, repo, branch, &head, below, &title, body)?;
        opened.push(tools::Stacked {
            resource: route::pull_request(repo, number),
            branch: branch.clone(),
            base,
        });
    }
    Ok(opened)
}

/// Opens an issue on the task's repository, titled and described by its latest result.
fn open_issue(db: &Connection, factory: &Factory, due: &state::Due) -> Result<Done, Error> {
    let opened = match (
        repo_of(factory, &due.trigger, due.repo.as_deref()),
        state::last_result(db, due.task)?,
    ) {
        (None, _) => Err(format!("task {} names no repo", due.task)),
        (_, None) => Err("no result describes the issue".to_owned()),
        (
            Some(repo),
            Some(state::Described {
                summary, evidence, ..
            }),
        ) => {
            let (title, body) = titled(&summary, evidence.as_deref());
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
            stdout: None,
            stack: Vec::new(),
        },
        Err(error) => failed(error),
    })
}

/// The workflow whose sandbox a run of `task` starts in: its own, or, for a lead or an investigator
/// task, that of the task it serves, since its prompt holds that task's agent's words. The inner
/// error blocks the task: a helper whose served task is unknown never starts unjailed.
fn jailing_workflow(db: &Connection, helper: i64) -> rusqlite::Result<Result<String, String>> {
    let mut task = helper;
    loop {
        let served: Option<Option<i64>> = db
            .query_row(
                "SELECT task FROM consults WHERE investigation = ?1
                 UNION ALL
                 SELECT CASE json_type(inbox.data, '$.task')
                   WHEN 'integer' THEN json_extract(inbox.data, '$.task') END
                 FROM tasks JOIN inbox ON inbox.id = tasks.source
                 WHERE tasks.id = ?1 AND tasks.trigger = ?2 AND inbox.type = 'priory.ask.routed'",
                rusqlite::params![task, tools::LEAD_TRIGGER],
                |row| row.get(0),
            )
            .optional()?;
        match served {
            None => break,
            // A served task is inserted before its helper, so each step goes to a lower id and ends.
            Some(Some(served)) if served < task => task = served,
            Some(_) => {
                return Ok(Err(format!(
                    "task {helper} serves a task its routed ask does not name, so its sandbox is unknown"
                )));
            }
        }
    }
    let workflow = db
        .query_row("SELECT workflow FROM tasks WHERE id = ?1", [task], |row| {
            row.get(0)
        })
        .optional()?;
    Ok(workflow.ok_or(format!(
        "task {helper} serves task {task}, which is gone, so its sandbox is unknown"
    )))
}

/// Starts a run for `admission`. The outer error stops the daemon; the inner one blocks the task.
#[allow(clippy::too_many_arguments)]
fn start(
    db: &mut Connection,
    trace: &crate::trace::Trace,
    runner: Option<&str>,
    home: &Path,
    scope: &[String],
    jailing: &Result<(), String>,
    admission: &Admission,
    recipes: &mut crate::recipe::Running,
    probes: &mut crate::connectors::Probes,
    kit: crate::kit::Current<'_>,
    attaching: &mut Vec<crate::attach::Attaching>,
) -> Result<Result<(), String>, Error> {
    let factory = trace.factory;
    let step = factory.workflows[&admission.workflow]
        .value
        .step(&admission.step)
        .expect("admissible checked the step");
    let jailed_by = match jailing_workflow(db, admission.task)? {
        Ok(workflow) => workflow,
        Err(reason) => return Ok(Err(reason)),
    };
    // A helper whose served workflow is gone cannot tell whether that workflow was jailed.
    if !factory.workflows.contains_key(&jailed_by) {
        return Ok(Err(format!(
            "task {} serves a task of workflow {jailed_by}, which the factory no longer has",
            admission.task
        )));
    }
    let sandbox = factory.sandbox_of(&jailed_by);
    if let (Some((name, _)), Err(why)) = (sandbox, jailing) {
        return Ok(Err(format!(
            "workflow {jailed_by} needs sandbox {name}, and this runner cannot start bubblewrap: {why}"
        )));
    }
    let jail = sandbox.map(|(_, sandbox)| priory_runtime::jail::Jail {
        read: sandbox.read.clone(),
        write: sandbox.write.clone(),
        env: sandbox.env.clone(),
    });
    let speaker = match &step.debate {
        Some(debate) => Some(crate::debate::speaker_of(db, admission.task, step, debate)?),
        None => None,
    };
    let Some((name, agent)) = speaker
        .as_ref()
        .or(step.agent.as_ref())
        .and_then(|name| Some((name, factory.agents.get(name)?)))
    else {
        return Ok(Err(format!("step {} names no known agent", step.id)));
    };
    let candidates = crate::memories::layer(db, Some(admission.task))?;
    let sources = crate::kit::sources(factory, kit, &candidates, Some(&agent.value));
    for skill in &sources.missing {
        let held = match sources.commit {
            Some(commit) => format!(
                "which neither the factory nor kit commit {} holds",
                &commit[..commit.len().min(12)]
            ),
            None => "which the factory does not hold, and this runner has no kit commit yet".into(),
        };
        eprintln!("priory daemon: agent {name} names skill {skill}, {held}");
    }
    let Some(repo) = repo_of(factory, &admission.trigger, admission.repo.as_deref()) else {
        return Ok(Err(format!("task {} names no repo", admission.task)));
    };
    let folder = worktree_name(db, runner, admission.task)?;
    let base = base_branch(db, admission.task)?;
    if base.is_some() && parent_moved(db, home, admission.task)? {
        return Ok(Ok(()));
    }
    let (worktree, branch, added) = match runtime::worktree(home, repo, &folder, base.as_deref()) {
        Ok((worktree, branch, added)) => {
            if let runtime::Added::Kept(Some(recovery)) = &added {
                let tx = db.transaction()?;
                crate::trace::worktree_recovered(&tx, trace, admission.task, &worktree, recovery)?;
                tx.commit()?;
            }
            let start = added.base();
            state::set_base(db, admission.task, start)?;
            if base.is_some() && start.is_some() {
                db.execute(
                    "UPDATE tasks SET parent_head = ?2 WHERE id = ?1",
                    rusqlite::params![admission.task, start],
                )?;
            }
            (worktree, branch, added)
        }
        Err(reason) => return Ok(Err(reason)),
    };
    let task = admission.task;
    match crate::recipe::before_run(db, factory, home, task, repo, &worktree, &added, recipes)? {
        crate::recipe::Before::Go => {}
        crate::recipe::Before::Waits => return Ok(Ok(())),
        crate::recipe::Before::Blocked(reason) => return Ok(Err(reason)),
    }
    let head = runtime::head(&worktree, &branch);
    if !tools::approved(db, admission.task, step, head.as_deref())? {
        return Ok(Ok(()));
    }
    if step.effect == factory::Effect::Irreversible && factory.staged(&admission.trigger) {
        return Ok(Err(format!(
            "staged: step {} is irreversible, so its run did not start",
            step.id
        )));
    }
    let turn = match &step.debate {
        Some(debate) => Some(crate::debate::turn(
            db,
            admission.task,
            step,
            debate,
            head.as_deref(),
        )?),
        None => None,
    };
    let servers = match servers(name, &agent.value, &worktree) {
        Ok(servers) => servers,
        Err(reason) => return Ok(Err(reason)),
    };
    if !servers.connectors.is_empty() {
        let connectors = &agent.value.connectors;
        match probes.before_run(admission.task, name, connectors, &worktree) {
            crate::connectors::Before::Go => {}
            crate::connectors::Before::Waits => {
                db.execute(
                    "UPDATE tasks SET reason = ?2 WHERE id = ?1 AND reason IS NOT ?2",
                    rusqlite::params![admission.task, crate::connectors::CHECKING],
                )?;
                return Ok(Ok(()));
            }
            crate::connectors::Before::Blocked(reason) => return Ok(Err(reason)),
        }
    }
    let shared = matches!(agent.value.session, Session::Shared);
    let resumed = match &turn {
        Some(turn) => turn.resume.clone(),
        None if shared => state::shared(db, name)?,
        None => state::resumable(db, admission.task, name)?,
    };
    let definition = definition(factory, &agent.value, &sources);
    let changed = match &resumed {
        Some(session) if shared => {
            state::definition(db, session)?.and_then(|stored| changed(&stored, &definition))
        }
        _ => None,
    };
    let fresh = changed.is_some() && agent.value.on_change == OnChange::Fresh;
    let resumed = resumed.filter(|_| !fresh);
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
    let prompt = match &turn {
        Some(turn) => turn.prompt.clone(),
        None => tools::step_prompt(step),
    };
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
            prompt: (admission.fresh || turn.is_some()).then_some(prompt.as_str()),
            ending: &tools::ending(step),
            shared,
            definition: &definition.to_string(),
            changed: changed.filter(|_| resume.is_some()).as_deref(),
            lanes: factory.lanes(),
            kit_hash: sources.hash().as_deref(),
            kit_commit: sources.commit,
            debate: turn.as_ref().map(|turn| turn.debate),
            memories_commit: sources.memories_commit,
            harness: agent.value.harness.name(),
            resumes: resume.is_some(),
            sandbox: sandbox.map(|(name, _)| name),
        },
    )?;
    // The new run is the task's only live one, so a session of this name is an ended run's agent.
    runtime::kill(&tmux);
    let first = match agent.value.harness {
        Harness::Codex => state::claim(db, run)?,
        Harness::ClaudeCode => None,
    };
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
        .chain(guard_env(factory))
        .chain(crate::todo::connectors_env(db)?)
        .chain(servers_env(&agent.value, &servers))
        .collect();
    let mut trust = false;
    let mut direnv = None;
    if factory.trust_worktrees() {
        let grants = if factory.names_repo(repo) {
            priory_runtime::trust::grant(harness(&agent.value.harness), home, repo, &worktree)
        } else {
            vec![priory_runtime::trust::Grant {
                path: worktree.clone(),
                file: std::path::PathBuf::new(),
                outcome: priory_runtime::trust::Outcome::Refused,
                why: Some(format!("the factory does not name {repo}")),
            }]
        };
        let tx = db.transaction()?;
        for grant in &grants {
            crate::trace::trusted(&tx, trace, run, agent.value.harness.name(), grant, None)?;
        }
        tx.commit()?;
        trust = grants.iter().any(|grant| {
            use priory_runtime::trust::Outcome;
            matches!(grant.outcome, Outcome::Granted | Outcome::Present)
        });
        if factory.names_repo(repo) {
            let folder = priory_runtime::trust::direnv_folder(home, &session);
            let grants = priory_runtime::trust::direnv(home, repo, &worktree, &folder);
            let tx = db.transaction()?;
            for grant in &grants {
                crate::trace::trusted(&tx, trace, run, "direnv", grant, None)?;
            }
            tx.commit()?;
            if grants
                .iter()
                .any(|grant| grant.outcome == priory_runtime::trust::Outcome::Granted)
            {
                state::direnv(db, run, 1)?;
                direnv = Some(folder);
            }
        }
    }
    let started = Started {
        name,
        tmux: &tmux,
        dir: &worktree,
        session: &session,
        resume: resume.as_deref(),
        prompt: first.as_deref(),
        debate: turn.is_some(),
        servers: &servers,
        jail: jail.as_ref(),
        trust,
        direnv: direnv.as_deref(),
    };
    let launched = launch(factory, &agent.value, &sources, home, scope, &env, &started);
    if launched.is_ok() && step.attach {
        crate::attach::start(db, trace, home, admission.task, run, &tmux, attaching)?;
    }
    Ok(launched)
}

/// What a shared session starts with: digests of its agent's prompt, tools, model and flags.
fn definition(
    factory: &Factory,
    agent: &factory::Agent,
    sources: &crate::kit::Sources,
) -> serde_json::Value {
    let digest = |part: serde_json::Value| {
        Sha256::digest(part.to_string().as_bytes())[..8]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    };
    let permissions = &agent.permissions;
    serde_json::json!({
        // Without the factory's skills or instructions, the digest is the one sessions stored before them.
        "prompt": match sources.hash() {
            None => digest(serde_json::json!(factory.prompt(agent))),
            Some(kit) => digest(serde_json::json!([factory.prompt(agent), kit])),
        },
        // Without connectors, the digest is the one sessions stored before them.
        "tools": if agent.connectors.is_empty() {
            digest(serde_json::json!([
                agent.mcp_servers,
                permissions.allow,
                permissions.deny,
                agent.connector_sign_in
            ]))
        } else {
            digest(serde_json::json!([
                agent.mcp_servers,
                permissions.allow,
                permissions.deny,
                agent.connector_sign_in,
                agent.connectors
            ]))
        },
        "model": digest(serde_json::json!([agent.model, agent.effort])),
        "flags": digest(serde_json::json!(permissions.default_mode.map(PermissionMode::as_str))),
    })
}

/// The line that tells a resumed session which parts of its agent changed since `stored`, if any.
fn changed(stored: &str, definition: &serde_json::Value) -> Option<String> {
    let stored: serde_json::Value = serde_json::from_str(stored).ok()?;
    let parts: Vec<&str> = ["prompt", "tools", "model", "flags"]
        .into_iter()
        .filter(|part| stored[part] != definition[part])
        .collect();
    (!parts.is_empty()).then(|| {
        format!(
            "Your agent definition changed since this conversation started: its {}.",
            parts.join(", ")
        )
    })
}

/// Where and as what a harness starts.
struct Started<'a> {
    /// The agent's name.
    name: &'a str,
    tmux: &'a str,
    dir: &'a Path,
    session: &'a str,
    resume: Option<&'a str>,
    prompt: Option<&'a str>,
    debate: bool,
    servers: &'a priory_runtime::servers::Servers,
    /// The sandbox its harness starts in, if its workflow turns one on.
    jail: Option<&'a priory_runtime::jail::Jail>,
    /// Priory recorded the harness's trust in the run's folders, so a Codex run takes its profile.
    trust: bool,
    /// The direnv config folder that trusts the run's folders, so the harness starts in `.envrc`.
    direnv: Option<&'a Path>,
}

/// The MCP servers `agent` names, found in the person's own config of its harness when its run
/// starts in `dir`, so their tokens never enter the factory or the trace.
fn servers(
    name: &str,
    agent: &factory::Agent,
    dir: &Path,
) -> Result<priory_runtime::servers::Servers, String> {
    match agent.harness {
        Harness::ClaudeCode => {
            priory_runtime::servers::claude(&agent.mcp_servers, &agent.connectors, dir)
        }
        Harness::Codex => priory_runtime::servers::codex(&agent.mcp_servers, dir),
    }
    .map_err(|reason| format!("agent {name} {reason}"))
}

/// `PRIORY_MCP_SERVERS` for a run with claude.ai connectors: the servers its tool guard lets
/// through, as their tools name them. Such a run loads every server and connector of the person's,
/// and the guard denies a tool of any other.
fn servers_env(
    agent: &factory::Agent,
    servers: &priory_runtime::servers::Servers,
) -> Option<String> {
    use priory_runtime::servers::tool_server;
    (!servers.connectors.is_empty()).then(|| {
        let names: Vec<String> = std::iter::once("priory")
            .chain(agent.mcp_servers.iter().map(String::as_str))
            .chain(servers.connectors.iter().map(String::as_str))
            .map(tool_server)
            .collect();
        format!("PRIORY_MCP_SERVERS={}", names.join(","))
    })
}

/// Starts `agent`'s harness with the flags every run of it gets, so a resumed session keeps them.
fn launch(
    factory: &Factory,
    agent: &factory::Agent,
    sources: &crate::kit::Sources,
    home: &Path,
    scope: &[String],
    env: &[String],
    started: &Started,
) -> Result<(), String> {
    let permissions = serde_json::to_value(&agent.permissions).expect("permissions serialize");
    let plugin = sources.plugin(home)?;
    let memories = sources.memories(home)?;
    let mut instructions = sources.run_instructions(
        &memories
            .as_deref()
            .map(|folder| folder.display().to_string())
            .unwrap_or_default(),
    );
    if let (Harness::Codex, Some(plugin)) = (&agent.harness, &plugin) {
        let index = sources.skill_index(&plugin.join("skills").display().to_string());
        instructions = factory::join_instructions(&instructions, &index);
    }
    instructions = factory::join_instructions(&instructions, &factory.consult_index(started.name));
    let tools = if agent.investigator {
        priory_runtime::mcp::Tools::Investigator
    } else if factory.consultable(started.name).is_empty() {
        priory_runtime::mcp::Tools::Worker
    } else {
        priory_runtime::mcp::Tools::Asker
    };
    let mut env = env.to_vec();
    if let Some(scoped) = sources.scoped(home, plugin.as_deref(), memories.as_deref())? {
        env.push(format!("PRIORY_PATHS={}", scoped.display()));
        env.push(format!("PRIORY_WORKTREE={}", started.dir.display()));
    }
    runtime::launch(
        harness(&agent.harness),
        &Launch {
            tmux: started.tmux,
            dir: started.dir,
            home,
            session: started.session,
            resume: started.resume,
            model: agent.model.as_deref(),
            effort: agent.effort.as_deref(),
            body: &factory.prompt(agent),
            instructions: &instructions,
            plugin: plugin.as_deref(),
            memories: memories.as_deref(),
            prompt: started.prompt,
            permissions,
            permission_mode: agent
                .permissions
                .default_mode
                .map_or("manual", PermissionMode::as_str),
            scope,
            env: &env,
            debate: started.debate,
            servers: started.servers,
            tools,
            jail: started.jail,
            trust: started.trust,
            direnv: started.direnv,
        },
    )
    .map_err(|failed| {
        // The full command runs to kilobytes, so it goes to the daemon's log, not the reason.
        if let Some(command) = &failed.command {
            eprintln!(
                "priory daemon: starting {}: {}\n  command: {command}",
                started.tmux, failed.reason
            );
        }
        failed.reason
    })
}

/// Compacts each shared session left idle past its agent's `idleCompactAt` before its prompt cache
/// expires: it resumes the session in tmux, types `/compact` once Claude started, and kills the
/// session once Claude compacted it.
fn compact_idle(
    db: &Connection,
    trace: &Trace,
    home: &Path,
    scope: &[String],
    kit: crate::kit::Current<'_>,
) -> Result<(), Error> {
    let candidates = crate::memories::layer(db, None)?;
    for live in idle::live(db)? {
        let ended_as = match (live.overdue, live.late) {
            (true, _) => Some("timeout"),
            (false, true) => Some("late"),
            _ => None,
        };
        if let Some(ended_as) = ended_as {
            runtime::kill(&live.tmux);
            idle::end(db, live.id, ended_as)?;
            continue;
        }
        if !live.ready {
            continue;
        }
        let typed = runtime::pane(&live.tmux, &live.session)
            .ok_or_else(|| "its pane is gone".to_owned())
            .and_then(|pane| runtime::type_text(&pane, "/compact"));
        if let Err(error) = typed {
            eprintln!("priory daemon: compacting {}: {error}", live.tmux);
            runtime::kill(&live.tmux);
            idle::end(db, live.id, "failed")?;
            continue;
        }
        idle::typed(db, live.id)?;
    }
    for (name, agent) in &trace.factory.agents {
        let Some(limit) = agent.value.idle_compact_at else {
            continue;
        };
        let Some(due) = idle::due(db, name)? else {
            continue;
        };
        let sources = crate::kit::sources(trace.factory, kit, &candidates, Some(&agent.value));
        let tmux = format!("priory-idle-{name}");
        let id = idle::begin(db, name, &due.session, &tmux)?;
        let dir = Path::new(&due.worktree);
        let context = (trace.usage)(Path::new(&due.transcript), 0).and_then(|usage| usage.context);
        // A resume with other flags than the cached conversation's would read it at full price.
        let stored = state::definition(db, &due.session)?;
        let skipped = if !dir.is_dir() {
            Some("no worktree")
        } else if context.is_none_or(|context| context < limit.get()) {
            Some("small")
        } else if stored.is_some_and(|stored| {
            changed(&stored, &definition(trace.factory, &agent.value, &sources)).is_some()
        }) {
            Some("changed")
        } else {
            None
        };
        if let Some(skipped) = skipped {
            idle::end(db, id, skipped)?;
            continue;
        }
        let servers = match servers(name, &agent.value, dir) {
            Ok(servers) => servers,
            Err(error) => {
                eprintln!("priory daemon: compacting {tmux}: {error}");
                idle::end(db, id, "failed")?;
                continue;
            }
        };
        let started = Started {
            name,
            tmux: &tmux,
            dir,
            session: &due.session,
            resume: Some(&due.session),
            prompt: None,
            debate: false,
            servers: &servers,
            jail: None,
            trust: false,
            direnv: None,
        };
        let env: Vec<String> = guard_env(trace.factory).collect();
        if let Err(error) = launch(
            trace.factory,
            &agent.value,
            &sources,
            home,
            scope,
            &env,
            &started,
        ) {
            eprintln!("priory daemon: compacting {tmux}: {error}");
            idle::end(db, id, "failed")?;
        }
    }
    Ok(())
}

/// `PRIORY_GUARD`: the factory's own guards, which the run's tool guard hook adds to the built-in
/// ones. `PRIORY_PRIVATE`: the factory has private terms, which the hook asks the daemon for.
fn guard_env(factory: &Factory) -> impl Iterator<Item = String> {
    let guards = factory.guards();
    let guards = (guards != Default::default()).then(|| {
        let json = serde_json::to_string(&guards).expect("guards serialize");
        format!("PRIORY_GUARD={json}")
    });
    let private = factory.private().map(|_| "PRIORY_PRIVATE=1".to_owned());
    guards.into_iter().chain(private)
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
    bus: &dyn Log,
    consumer: &str,
    db: &mut Connection,
    trace: &Trace,
    counter: Option<&embed::Counter>,
) -> Result<(), Error> {
    drain_probed(bus, consumer, db, trace, counter, false)
}

/// Handles every message past `consumer`'s cursor. With `probe`, times the first read as the
/// tick's probe of the log.
fn drain_probed(
    bus: &dyn Log,
    consumer: &str,
    db: &mut Connection,
    trace: &Trace,
    counter: Option<&embed::Counter>,
    probe: bool,
) -> Result<(), Error> {
    let mut probe = probe.then(Instant::now);
    loop {
        let deliveries = read(bus, consumer, &mut probe)?;
        if deliveries.is_empty() {
            return Ok(());
        }
        for delivery in deliveries {
            let mut message = runners::clocked(delivery.message.clone(), delivery.logged);
            crate::decision::triage(db, trace, &mut message)?;
            let ended = state::handle(db, &message, trace, counter)?;
            // Before the ack, which can fail on a lost link: a redelivered message kills nothing.
            ended.iter().for_each(|tmux| runtime::kill(tmux));
            bus.ack(consumer, &delivery)?;
        }
    }
}

/// Reads up to 100 messages past `consumer`'s cursor, timing the read as the log's probe when
/// `probe` holds when it started, and then clearing it.
fn read(
    bus: &dyn Log,
    consumer: &str,
    probe: &mut Option<Instant>,
) -> Result<Vec<log::Delivery>, Error> {
    let read = bus.read(consumer, OUTPUT, 100);
    if let Some(started) = probe.take() {
        metrics::probed(
            started.elapsed(),
            read.as_ref().map(|_| ()).map_err(|error| &**error),
        );
    }
    read
}

/// Samples the log's backlog for the metrics once a try on the log reached it, and forgets the
/// last one when it did not.
fn metered(bus: &dyn Log, tried: Result<&(), &Error>) {
    match tried {
        Ok(()) => metrics::sample_backlog(bus),
        Err(_) => metrics::unreached(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// A workflow of human steps, each one going to the next.
    fn workflow(steps: &[&str]) -> String {
        let mut text = format!("start = \"{}\"\n", steps[0]);
        for (at, step) in steps.iter().enumerate() {
            let next = steps.get(at + 1).unwrap_or(&"done");
            text.push_str(&format!(
                "\n[[steps]]\nid = \"{step}\"\ndecides = \"human\"\neffect = \"none\"\n\
                 options = [\"go\"]\nnext = {{ go = \"{next}\" }}\n"
            ));
        }
        text
    }

    #[test]
    fn a_late_run_whose_envrc_still_loads_names_direnv_rather_than_folder_trust() {
        let home = std::env::temp_dir().join(format!("priory-late-direnv-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        let spool = runtime::direnv_spool(&home, "s");
        fs::create_dir_all(&spool).unwrap();
        fs::write(spool.join("direnv.error"), "direnv: loading .envrc\n").unwrap();
        let run = state::LiveRun {
            id: 1,
            task: 1,
            step: "implement".into(),
            agent: "implementer".into(),
            workflow: "w".into(),
            tmux: format!("priory-late-direnv-{}", std::process::id()),
            session: "s".into(),
            worktree: Some("/w/1".into()),
            ready: false,
            waited: Some(600),
            direnv: 1,
            turn: None,
            prompted: None,
        };

        let (ended_as, reason, _) =
            verdict(&run, &Factory::default(), &home, &|| true, &|| None).unwrap();

        assert_eq!(ended_as, "exited");
        assert!(
            reason.starts_with(
                "direnv did not finish loading /w/1/.envrc within 600s, so claude never started"
            ),
            "{reason}"
        );
        assert!(!reason.contains("trust the"), "{reason}");
        assert!(
            reason.contains("fix the .envrc. Resume the task to try again"),
            "{reason}"
        );
    }

    /// Writes the factory's file once `reload::EVERY` passed, then polls.
    fn edit_then_poll(
        live: &mut Live,
        db: &mut Connection,
        base: &Trace,
        edit: impl FnOnce(),
    ) -> bool {
        edit();
        std::thread::sleep(reload::EVERY);
        live.poll(db, base, true).unwrap()
    }

    fn steps(live: &Live) -> Vec<String> {
        let workflow = &live.factory().workflows["w"].value;
        workflow.steps.iter().map(|step| step.id.clone()).collect()
    }

    #[test]
    fn a_held_edit_waits_while_a_newer_edit_settles() {
        let dir = std::env::temp_dir().join(format!("priory-held-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("workflows")).unwrap();
        fs::write(dir.join("factory.toml"), "name = \"t\"\n").unwrap();
        fs::write(dir.join("workflows/w.toml"), workflow(&["a", "b"])).unwrap();
        let start = priory_factory::rules::validated(&dir).unwrap();
        let base = Trace {
            factory: &start,
            revision: None,
            machine: None,
            runner: None,
            usage: |_, _| None,
        };
        let mut db = state::memory();
        db.execute(
            "INSERT INTO tasks (key, workflow, step, state, trigger, source)
             VALUES ('cli:a', 'w', 'b', 'running', 'cli', 'a')",
            [],
        )
        .unwrap();
        let mut live = Live::new(&start, reload::Watch::new(&start), None, None);

        // The held edit removes step b, where the task is, and names a `writing` guide.
        let held = || {
            fs::write(dir.join("W.md"), "Write plainly.\n").unwrap();
            fs::write(
                dir.join("factory.toml"),
                "name = \"t\"\nwriting = \"W.md\"\n",
            )
            .unwrap();
            fs::write(dir.join("workflows/w.toml"), workflow(&["a"])).unwrap();
        };
        assert!(!edit_then_poll(&mut live, &mut db, &base, held));
        assert!(!edit_then_poll(&mut live, &mut db, &base, || {}));
        assert_eq!(steps(&live), ["a", "b"]);

        db.execute("UPDATE tasks SET state = 'done'", []).unwrap();
        let newer = || fs::write(dir.join("workflows/w.toml"), workflow(&["a", "c"])).unwrap();
        assert!(!edit_then_poll(&mut live, &mut db, &base, newer));
        assert!(edit_then_poll(&mut live, &mut db, &base, || {}));
        assert_eq!(steps(&live), ["a", "c"]);
        // The held edit was dropped, so it never reverts the newer one.
        assert!(!edit_then_poll(&mut live, &mut db, &base, || {}));
        assert_eq!(steps(&live), ["a", "c"]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_held_edit_applied_mid_edit_keeps_the_newer_edit() {
        let dir = std::env::temp_dir().join(format!("priory-mid-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("workflows")).unwrap();
        fs::write(dir.join("factory.toml"), "name = \"t\"\n").unwrap();
        fs::write(dir.join("workflows/w.toml"), workflow(&["a", "b"])).unwrap();
        let start = priory_factory::rules::validated(&dir).unwrap();
        let base = Trace {
            factory: &start,
            revision: None,
            machine: None,
            runner: None,
            usage: |_, _| None,
        };
        let mut db = state::memory();
        db.execute(
            "INSERT INTO tasks (key, workflow, step, state, trigger, source)
             VALUES ('cli:a', 'w', 'b', 'running', 'cli', 'a')",
            [],
        )
        .unwrap();
        let mut live = Live::new(&start, reload::Watch::new(&start), None, None);

        // The held edit removes step b and names a `writing` guide, which changes what is watched.
        let held = || {
            fs::write(dir.join("W.md"), "Write plainly.\n").unwrap();
            fs::write(
                dir.join("factory.toml"),
                "name = \"t\"\nwriting = \"W.md\"\n",
            )
            .unwrap();
            fs::write(dir.join("workflows/w.toml"), workflow(&["a"])).unwrap();
        };
        assert!(!edit_then_poll(&mut live, &mut db, &base, held));
        assert!(!edit_then_poll(&mut live, &mut db, &base, || {}));

        // The task leaves and a newer edit lands before the watch is due: the held edit applies.
        db.execute("UPDATE tasks SET state = 'done'", []).unwrap();
        fs::write(dir.join("workflows/w.toml"), workflow(&["a", "c"])).unwrap();
        assert!(live.poll(&mut db, &base, true).unwrap());
        assert_eq!(steps(&live), ["a"]);

        // The newer edit still settles and loads.
        assert!(!edit_then_poll(&mut live, &mut db, &base, || {}));
        assert!(edit_then_poll(&mut live, &mut db, &base, || {}));
        assert_eq!(steps(&live), ["a", "c"]);
        let _ = fs::remove_dir_all(&dir);
    }
}
