use priory_core::memory::Gate;
use priory_core::{daemon, report, stall, state, sweep, tools};
use priory_factory::{factory, rules};
use priory_intake::{intake, request};
use priory_runtime::{mcp, runtime};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::{Duration, Instant};

fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    match args.next().as_deref().and_then(|verb| verb.to_str()) {
        Some("check") => check(
            args.next()
                .map_or_else(|| PathBuf::from("."), PathBuf::from),
        ),
        Some("draw") => {
            let args: Vec<_> = args.collect();
            match &args[..] {
                [workflow] => draw(Path::new("."), workflow),
                [dir, workflow] => draw(Path::new(dir), workflow),
                _ => usage(),
            }
        }
        Some(verb @ ("intake" | "daemon")) => {
            let (once, dirs): (Vec<_>, Vec<_>) = args.partition(|arg| arg == "--once");
            let dir = match dirs.as_slice() {
                [] => PathBuf::from("."),
                [dir] => PathBuf::from(dir),
                _ => return usage(),
            };
            let run = if verb == "intake" {
                |factory: &factory::Factory, once| intake::run(factory, once, watched)
            } else {
                daemon::run
            };
            serve(verb, &dir, run, !once.is_empty())
        }
        Some("request") => request(args.map(|arg| arg.to_string_lossy().into_owned()).collect()),
        Some("inbox") => list(
            "seq\ttype\tsubject\ttask\toutcome",
            "SELECT seq, type, subject, task, outcome FROM inbox ORDER BY seq",
        ),
        Some("tasks") => {
            let args: Vec<_> = args.map(|arg| arg.to_string_lossy().into_owned()).collect();
            match &args[..] {
                [] => tasks(None),
                [flag, lane] if flag == "--lane" => tasks(Some(lane)),
                _ => usage(),
            }
        }
        Some("runs") => list(
            "id\ttask\tstep\ttmux\tsession\tstarted\tended\tend",
            "SELECT id, task, step, tmux, coalesce(thread, session), started, ended, ended_as FROM runs
             ORDER BY id",
        ),
        Some("outbox") => list(
            "id\ttask\trun\tstate\ttext",
            "SELECT id, task, run, state, replace(text, char(10), '\\n') FROM outbox ORDER BY id",
        ),
        Some("asks") => list(
            "id\ttask\trevision\tquestion\toptions",
            "SELECT id, task, revision, replace(question, char(10), '\\n'), options FROM asks
             WHERE answer IS NULL AND withdrawn IS NULL ORDER BY id",
        ),
        Some("waits") => list(
            "id\ttask\trun\tkind\ton\tcreated",
            "SELECT id, task, run, kind, coalesce(resource, until, 'task ' || target,
               printf('%.1f GB free', bytes / 1073741824.0)), created
             FROM waits WHERE fired IS NULL AND withdrawn IS NULL ORDER BY id",
        ),
        Some("desk") => desk(),
        Some("trace") => trace(),
        Some("report") => report(args.map(|arg| arg.to_string_lossy().into_owned()).collect()),
        Some("move") => move_task(args.map(|arg| arg.to_string_lossy().into_owned()).collect()),
        Some(verb @ ("resume" | "cancel" | "revive" | "release")) => act(
            verb,
            args.map(|arg| arg.to_string_lossy().into_owned()).collect(),
            None,
        ),
        Some("hold") => hold(args.map(|arg| arg.to_string_lossy().into_owned()).collect()),
        Some("actions") => list(
            "id\ttask\tstep\taction\tstatus\tpull_request\tcommit\terror",
            "SELECT id, task, step, action, status, pull_request, commit_sha,
             replace(error, char(10), '\\n') FROM actions ORDER BY id",
        ),
        Some("memory") => memory(args.collect()),
        Some("answer") => answer(args.map(|arg| arg.to_string_lossy().into_owned()).collect()),
        Some("mcp") => match (args.next(), args.next(), args.next()) {
            (Some(flag), Some(session), None) if flag == "--session" => {
                exit("mcp", mcp::serve(&session.to_string_lossy()))
            }
            _ => usage(),
        },
        Some("hook") => match (args.next(), args.next()) {
            (Some(event), None) => hook(&event.to_string_lossy()),
            _ => usage(),
        },
        _ => usage(),
    }
}

fn usage() -> ExitCode {
    eprintln!(
        "usage: priory check [DIR]\n       priory draw [DIR] WORKFLOW\n       priory intake [DIR] [--once]\n       priory daemon [DIR] [--once]\n       priory request [DIR] --workflow <name> --key <key> [--band <band>] [--lane <lane>] [--repo <owner/name>] [--hold <reason> [CONDITION]] <text>\n       priory inbox\n       priory tasks [--lane <lane>]\n       priory runs\n       priory outbox\n       priory asks\n       priory waits\n       priory actions\n       priory desk\n       priory trace\n       priory report [DIR] --workflow <name> [--since <instant-or-duration>] [--cost] [--by lane]\n       priory move TASK [--band <band>] [--top | --before TASK | --after TASK]\n       priory resume TASK\n       priory cancel TASK\n       priory revive TASK [--step STEP]\n       priory hold TASK --reason TEXT [--until TIME | --after TASK | --merge PR [--if-closed release|drop|ask] | --blockers]\n       priory release TASK\n       priory memory [acquire --gb N [--timeout D] -- COMMAND...]\n       priory answer ASK OPTION-OR-TEXT --revision N\n       priory mcp --session ID\n       priory hook EVENT"
    );
    ExitCode::from(2)
}

/// The factory, or `None` after printing its problems.
fn load(dir: &Path) -> Option<factory::Factory> {
    let (factory, mut problems) = factory::load(dir);
    // Rules on a partly loaded factory would report references to the files that failed to parse.
    if problems.is_empty() {
        problems = rules::check(&factory);
    }
    for problem in &problems {
        eprintln!("{problem}");
    }
    problems.is_empty().then_some(factory)
}

/// Runs `run` on the factory in `dir`, once it loads with no problems.
/// The open waits the intake polls for, read from `PRIORY_DB`; none when the intake runs without
/// a state file.
fn watched() -> Result<Vec<intake::Watch>, String> {
    if std::env::var_os("PRIORY_DB").is_none() {
        return Ok(Vec::new());
    }
    let db = state::open()?;
    let watched = state::watched(&db).map_err(|error| format!("open waits: {error}"))?;
    watched
        .into_iter()
        .map(|(resource, since)| {
            let since = since
                .parse()
                .map_err(|error| format!("wait on {resource} since `{since}`: {error}"))?;
            Ok(intake::Watch { resource, since })
        })
        .collect()
}

fn serve(
    verb: &str,
    dir: &Path,
    run: fn(&factory::Factory, bool) -> Result<(), String>,
    once: bool,
) -> ExitCode {
    let Some(factory) = load(dir) else {
        return ExitCode::FAILURE;
    };
    match run(&factory, once) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("priory {verb}: {error}");
            ExitCode::FAILURE
        }
    }
}

/// A stalled queue's banner, what waits on a person, then every worktree under `PRIORY_HOME` that
/// stays and why.
fn desk() -> ExitCode {
    let printed = state::open().and_then(|db| {
        stall::banner(&db)
            .map_err(|error| error.to_string())?
            .inspect(|banner| state::emit(banner));
        state::print(
            &db,
            "since\tkind\tid\ttask\tlane\twhat\tgeneration",
            &format!("{} {DESK}", queue()),
            [],
        )
        .and_then(|()| sweep::print_stays(&db, &runtime::home()))
        .map_err(|error| error.to_string())
    });
    exit("desk", printed)
}

/// Every trace record, oldest first, one CloudEvents JSON object per line.
fn trace() -> ExitCode {
    let printed = state::open().and_then(|db| {
        let events = state::events(&db).map_err(|error| error.to_string())?;
        events.iter().for_each(|event| state::emit(event));
        Ok(())
    });
    exit("trace", printed)
}

/// Prints `priory report [DIR] --workflow <name> [--since <window>] [--cost]`. Only `--cost` reads
/// the factory, for its prices.
fn report(args: Vec<String>) -> ExitCode {
    let mut workflow = None;
    let mut since = report::SINCE.to_owned();
    let mut cost = false;
    let mut by_lane = false;
    let mut dirs = Vec::new();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--workflow" => workflow = args.next(),
            "--cost" => cost = true,
            "--by" if args.next().as_deref() == Some("lane") => by_lane = true,
            "--since" => match args.next() {
                Some(window) => since = window,
                None => return usage(),
            },
            _ if arg.starts_with("--") => return usage(),
            _ => dirs.push(arg),
        }
    }
    if cost && by_lane {
        return usage();
    }
    let (Some(workflow), [] | [_]) = (workflow, &dirs[..]) else {
        return usage();
    };
    if !cost {
        let printed =
            state::open().and_then(|db| report::outcomes(&db, &workflow, &since, by_lane));
        return exit("report", printed);
    }
    let dir = dirs
        .first()
        .map_or_else(|| PathBuf::from("."), PathBuf::from);
    let Some(factory) = load(&dir) else {
        return ExitCode::FAILURE;
    };
    let printed =
        state::open().and_then(|db| report::cost(&db, &workflow, &since, &factory.prices));
    exit("report", printed)
}

/// Asks, blocked tasks and uncertain prompts oldest first, then the queue in start order.
const DESK: &str = "SELECT since, kind, id, task, lane, what,
               (SELECT generation FROM tasks WHERE tasks.id = task) FROM (
               SELECT 0 AS part, asks.created AS since, 'ask' AS kind, asks.id AS id,
                 asks.task AS task, tasks.lane AS lane,
                 replace(asks.question, char(10), '\\n') || ' ' || asks.options AS what
               FROM asks JOIN tasks ON tasks.id = asks.task
               WHERE asks.answer IS NULL AND asks.withdrawn IS NULL
                 AND tasks.state NOT IN ('done', 'failed', 'dropped')
               UNION ALL
               SELECT 0, changed, 'blocked', id, id, lane, reason FROM tasks WHERE state = 'blocked'
               UNION ALL
               SELECT 0, changed, 'failed', id, id, lane,
                 coalesce(reason, 'failed') || '; revive: priory revive ' || id
               FROM tasks WHERE state = 'failed'
               UNION ALL
               SELECT 0, outbox.attempted, 'prompt', outbox.id, outbox.task, tasks.lane,
                 replace(outbox.text, char(10), '\\n')
               FROM outbox JOIN tasks ON tasks.id = outbox.task
               WHERE outbox.state = 'uncertain' AND tasks.state NOT IN ('done', 'failed', 'dropped')
               UNION ALL
               SELECT 0, holds.created, 'held', holds.id, holds.task, tasks.lane, tasks.reason
               FROM holds JOIN tasks ON tasks.id = holds.task
               WHERE holds.released IS NULL AND tasks.state = 'held'
               UNION ALL
               SELECT 0, sweeps.started, 'sweep', sweeps.id, stalls.task, tasks.lane,
                 sweeps.script || ': ' || coalesce(sweeps.status
                   || coalesce(nullif(': ' || replace(sweeps.output, char(10), '\\n'), ': '), ''),
                   'running')
               FROM sweeps JOIN stalls ON stalls.id = sweeps.stall JOIN tasks ON tasks.id = stalls.task
               WHERE stalls.ended IS NULL
               UNION ALL
               SELECT 0, grants.created, 'memory', grants.id, grants.task, tasks.lane,
                 'waits for ' || printf('%.1f', grants.bytes / 1073741824.0) || ' GB: '
                 || replace(grants.command, char(10), '\\n')
               FROM grants LEFT JOIN tasks ON tasks.id = grants.task WHERE grants.state = 'waiting'
               UNION ALL
               SELECT 0, retired, 'retired', id, id, lane,
                 'its worktree was removed to free disk; it resurrects from ' || branch
                   || ' when its wait fires'
               FROM tasks WHERE state = 'asleep' AND retired IS NOT NULL
               UNION ALL
               SELECT 1, tasks.changed, 'queue', queue.place, tasks.id, tasks.lane,
                 coalesce(tasks.band, '-') || coalesce(': ' || tasks.reason, '')
               FROM tasks JOIN queue USING (id)
             ) ORDER BY part, CASE WHEN part = 0 THEN since END, kind, id";

/// Each queued or ready task's place in the start order, from 1.
fn queue() -> String {
    format!(
        "WITH queue AS (SELECT id, row_number() OVER (ORDER BY {}) AS place
         FROM tasks WHERE state IN ('queued', 'ready'))",
        state::START_ORDER
    )
}

fn tasks(lane: Option<&String>) -> ExitCode {
    let printed = state::open().and_then(|db| {
        state::print(
            &db,
            "id\tkey\tworkflow\tstep\tstate\tband\tlane\treason\tposition\tgeneration\tretired",
            &format!(
                "{} SELECT id, key, workflow, step, state, band, lane, reason, queue.place, generation,
                   retired
                 FROM tasks LEFT JOIN queue USING (id) WHERE ?1 IS NULL OR lane = ?1 ORDER BY {}",
                queue(),
                state::START_ORDER
            ),
            [lane],
        )
        .map_err(|error| error.to_string())
    });
    exit("tasks", printed)
}

fn list(header: &str, query: &str) -> ExitCode {
    let printed = state::open()
        .and_then(|db| state::print(&db, header, query, []).map_err(|error| error.to_string()));
    match printed {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("priory: {error}");
            ExitCode::FAILURE
        }
    }
}

fn request(args: Vec<String>) -> ExitCode {
    let Some((dir, request)) = request::parse(args) else {
        return usage();
    };
    let Some(factory) = load(&dir) else {
        return ExitCode::FAILURE;
    };
    match request::run(&factory, &request) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("priory request: {error}");
            ExitCode::FAILURE
        }
    }
}

fn exit(verb: &str, done: Result<(), String>) -> ExitCode {
    match done {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("priory {verb}: {error}");
            ExitCode::FAILURE
        }
    }
}

/// `priory memory` lists the memory grants, held and waiting.
fn memory(args: Vec<OsString>) -> ExitCode {
    match args.split_first() {
        None => list(
            "id\ttask\tstate\tgb\tcommand\tage",
            "SELECT id, task, state, printf('%.1f', bytes / 1073741824.0),
             replace(command, char(10), '\\n'),
             (strftime('%s', 'now') - strftime('%s', coalesce(granted, created))) || 's'
             FROM grants ORDER BY id",
        ),
        Some((verb, args)) if verb == "acquire" => acquire(args),
        Some(_) => usage(),
    }
}

/// The exit code of `priory memory acquire` when the gate itself fails, as `env` and `timeout` use.
const GATE_FAILED: u8 = 125;

/// Runs a command once the daemon grants it `--gb` GB of memory, and releases the grant when it
/// exits. Exits with the command's code.
fn acquire(args: &[OsString]) -> ExitCode {
    let Some(split) = args.iter().position(|arg| arg == "--") else {
        return usage();
    };
    let (mut gb, mut timeout) = (None, None);
    for pair in args[..split].chunks(2) {
        match pair {
            [flag, value] if flag == "--gb" => {
                let Some(value) = value
                    .to_str()
                    .and_then(|value| value.parse::<f64>().ok())
                    .filter(|gb| *gb > 0.0 && gb.is_finite())
                else {
                    return usage();
                };
                gb = Some(value);
            }
            [flag, value] if flag == "--timeout" => {
                let Some(value) = value
                    .to_str()
                    .and_then(|value| value.parse::<jiff::SignedDuration>().ok())
                    .and_then(|value| Duration::try_from(value).ok())
                else {
                    return usage();
                };
                timeout = Some(value);
            }
            _ => return usage(),
        }
    }
    let command = &args[split + 1..];
    let (Some(gb), Some((program, rest))) = (gb, command.split_first()) else {
        return usage();
    };
    let failed = |error: String| {
        eprintln!("priory memory: {error}; the command did not run");
        ExitCode::from(GATE_FAILED)
    };
    let acquired = gate(&Gate::Acquire {
        bytes: (gb * f64::from(1u32 << 30)) as u64,
        pid: std::process::id(),
        session: std::env::var("PRIORY_SESSION").ok(),
        command: command.join(" ".as_ref()).to_string_lossy().into_owned(),
    });
    let mut reply = match acquired {
        Ok(reply) => reply,
        Err(error) => return failed(error),
    };
    let (id, _) = reply.split_once(' ').unwrap_or_default();
    let grant: i64 = id.parse().expect("the daemon replies with the grant id");
    let deadline = timeout.map(|timeout| Instant::now() + timeout);
    let mut said = String::new();
    while !reply.ends_with(" held") {
        if said != reply {
            eprintln!("priory memory: grant {reply} for {gb} GB");
            said = reply.clone();
        }
        if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            let _ = gate(&Gate::Release { grant });
            let waited = timeout.unwrap_or_default();
            return failed(format!("grant {grant} still waits after {waited:?}"));
        }
        std::thread::sleep(Duration::from_millis(250));
        reply = match gate(&Gate::Poll { grant }) {
            Ok(reply) => reply,
            Err(error) => return failed(error),
        };
    }
    let status = Command::new(program)
        .args(rest)
        .spawn()
        .and_then(|mut child| {
            // The command holds the grant from here, so it keeps its memory if this process dies.
            let _ = gate(&Gate::Run {
                grant,
                pid: child.id(),
            });
            child.wait()
        });
    if let Err(error) = gate(&Gate::Release { grant }) {
        eprintln!(
            "priory memory: grant {grant} stays until the daemon sees this process end: {error}"
        );
    }
    match status {
        Ok(status) => {
            use std::os::unix::process::ExitStatusExt;
            let code = status
                .code()
                .or_else(|| status.signal().map(|signal| 128 + signal));
            ExitCode::from(code.unwrap_or(1) as u8)
        }
        Err(error) => {
            eprintln!("priory memory: {}: {error}", program.to_string_lossy());
            ExitCode::from(127)
        }
    }
}

/// Sends `gate` to the daemon and returns its reply.
fn gate(gate: &Gate) -> Result<String, String> {
    let line = serde_json::to_string(gate).expect("a gate message serializes");
    runtime::call(&runtime::home(), &line).and_then(|reply| {
        serde_json::from_str::<Result<String, String>>(&reply)
            .map_err(|error| format!("unreadable reply {reply:?}: {error}"))?
    })
}

/// Sends `priory answer ASK TEXT --revision N` to the daemon, which applies it.
fn answer(mut args: Vec<String>) -> ExitCode {
    let Some(flag) = args.iter().position(|arg| arg == "--revision") else {
        return usage();
    };
    let revision = args.drain(flag..(flag + 2).min(args.len())).nth(1);
    let (Some(revision), [ask, text]) = (revision.and_then(|n| n.parse().ok()), &args[..]) else {
        return usage();
    };
    let Ok(ask) = ask.parse() else {
        return usage();
    };
    let line = serde_json::to_string(&tools::Answer {
        ask,
        revision,
        text: text.clone(),
    })
    .expect("an answer serializes");
    send("answer", &line)
}

/// Sets a task's band and its place in the band's person order.
fn move_task(args: Vec<String>) -> ExitCode {
    let mut args = args.into_iter();
    let Some(Ok(task)) = args.next().map(|task| task.parse()) else {
        return usage();
    };
    let (mut band, mut place) = (None, None);
    while let Some(flag) = args.next() {
        let mut task = || args.next().and_then(|task| task.parse().ok());
        let parsed = match flag.as_str() {
            "--band" if band.is_none() => {
                band = args.next();
                band.is_some()
            }
            _ if place.is_some() => false,
            "--top" => {
                place = Some(state::Place::Top);
                true
            }
            "--before" => {
                place = task().map(state::Place::Before);
                place.is_some()
            }
            "--after" => {
                place = task().map(state::Place::After);
                place.is_some()
            }
            _ => false,
        };
        if !parsed {
            return usage();
        }
    }
    if band.is_none() && place.is_none() {
        return usage();
    }
    let moved =
        state::open().and_then(|mut db| state::move_task(&mut db, task, band.as_deref(), place));
    exit("move", moved)
}

/// Sends `priory hold TASK --reason TEXT [CONDITION]` to the daemon, which applies it.
fn hold(args: Vec<String>) -> ExitCode {
    let mut args = args.into_iter();
    let Some(task) = args.next() else {
        return usage();
    };
    let mut hold = factory::Hold::default();
    let mut reason = None;
    while let Some(flag) = args.next() {
        if flag == "--blockers" {
            hold.blockers = true;
            continue;
        }
        let Some(value) = args.next() else {
            return usage();
        };
        match flag.as_str() {
            "--reason" => reason = Some(value),
            "--until" => hold.until = Some(value),
            "--after" => match value.parse() {
                Ok(other) => hold.after = Some(other),
                Err(_) => return usage(),
            },
            "--merge" => hold.merge = Some(value),
            "--if-closed" => hold.if_closed = Some(value),
            _ => return usage(),
        }
    }
    let Some(reason) = reason else {
        return usage();
    };
    hold.reason = reason;
    act("hold", vec![task], Some(hold))
}

/// Sends `priory resume`, `cancel`, `revive TASK [--step STEP]`, `release` or `hold` for one task to
/// the daemon, which applies it.
fn act(act: &str, mut args: Vec<String>, hold: Option<factory::Hold>) -> ExitCode {
    let step = match args.iter().position(|arg| arg == "--step") {
        Some(flag) if act == "revive" && flag + 1 < args.len() => args.drain(flag..flag + 2).nth(1),
        Some(_) => return usage(),
        None => None,
    };
    let [task] = &args[..] else {
        return usage();
    };
    let Ok(task) = task.parse() else {
        return usage();
    };
    let line = serde_json::to_string(&tools::Act {
        act: act.to_owned(),
        task,
        step,
        hold,
    })
    .expect("an action serializes");
    send(act, &line)
}

/// Sends `line` to the daemon and prints its reply.
fn send(verb: &str, line: &str) -> ExitCode {
    let reply = runtime::call(&runtime::home(), line).and_then(|reply| {
        serde_json::from_str::<Result<String, String>>(&reply)
            .map_err(|error| format!("unreadable reply {reply:?}: {error}"))?
    });
    match reply {
        Ok(text) => {
            println!("{text}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("priory {verb}: {error}");
            ExitCode::FAILURE
        }
    }
}

fn check(dir: PathBuf) -> ExitCode {
    let Some(factory) = load(&dir) else {
        return ExitCode::FAILURE;
    };
    println!(
        "ok: {}, {}, {}, {}",
        count(factory.agents.len(), "agent"),
        count(factory.triggers.len(), "trigger"),
        count(factory.workflows.len(), "workflow"),
        count(factory.runners.len(), "runner"),
    );
    ExitCode::SUCCESS
}

fn draw(dir: &Path, workflow: &std::ffi::OsStr) -> ExitCode {
    let Some(factory) = load(dir) else {
        return ExitCode::FAILURE;
    };
    let name = workflow.to_string_lossy();
    match factory.workflows.get(name.as_ref()) {
        Some(workflow) => {
            print!("{}", priory_factory::draw::mermaid(&workflow.value));
            ExitCode::SUCCESS
        }
        None => exit("draw", Err(format!("unknown workflow `{name}`"))),
    }
}

fn count(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

/// Reports a harness hook to the daemon: `event`, with the session and prompt from the hook's JSON
/// on stdin. A run's pane sets `PRIORY_SESSION`; a stdin session apart from it is the harness's own.
fn hook(event: &str) -> ExitCode {
    let input: serde_json::Value = match serde_json::from_reader(std::io::stdin()) {
        Ok(input) => input,
        Err(error) => {
            eprintln!("priory hook: stdin: {error}");
            return ExitCode::FAILURE;
        }
    };
    let Some(session) = input["session_id"].as_str() else {
        eprintln!("priory hook: stdin has no session_id");
        return ExitCode::FAILURE;
    };
    let priory = std::env::var("PRIORY_SESSION")
        .ok()
        .filter(|id| !id.is_empty());
    let thread = priory
        .as_ref()
        .filter(|id| *id != session)
        .map(|_| session.to_owned());
    let signal = state::Signal {
        event: event.to_owned(),
        session: priory.unwrap_or_else(|| session.to_owned()),
        prompt: input["prompt"].as_str().map(str::to_owned),
        message: input["message"].as_str().map(str::to_owned),
        transcript: input["transcript_path"].as_str().map(str::to_owned),
        thread,
        source: input["source"].as_str().map(str::to_owned),
    };
    let line = serde_json::to_string(&signal).expect("signal serializes");
    match runtime::send(&runtime::home(), &line) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("priory hook: {error}");
            ExitCode::FAILURE
        }
    }
}
