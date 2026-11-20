use priory_core::memory::Gate;
use priory_core::{daemon, embed, listing, report, search, serve, snooze, state, tools};
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
        Some(verb @ ("inbox" | "tasks" | "asks" | "desk" | "queue")) => {
            listed(verb, args.map(|arg| arg.to_string_lossy().into_owned()).collect())
        }
        Some("snooze") => snooze(args.map(|arg| arg.to_string_lossy().into_owned()).collect()),
        Some("serve") => serve_http(args.map(|arg| arg.to_string_lossy().into_owned()).collect()),
        Some("runs") => list(
            "id\ttask\tstep\ttmux\tsession\tstarted\tended\tend",
            "SELECT id, task, step, tmux, coalesce(thread, session), started, ended, ended_as FROM runs
             ORDER BY id",
        ),
        Some("outbox") => list(
            "id\ttask\trun\tstate\ttext",
            "SELECT id, task, run, state, replace(text, char(10), '\\n') FROM outbox ORDER BY id",
        ),
        Some("waits") => list(
            "id\ttask\trun\tkind\ton\tcreated",
            "SELECT id, task, run, kind, coalesce(resource, until, 'task ' || target,
               printf('%.1f GB free', bytes / 1073741824.0)), created
             FROM waits WHERE fired IS NULL AND withdrawn IS NULL ORDER BY id",
        ),
        Some("attach") => attach(args.map(|arg| arg.to_string_lossy().into_owned()).collect()),
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
        Some("tldr") => match (args.next(), args.next(), args.next()) {
            (Some(task), Some(text), None) => tldr(&task.to_string_lossy(), &text.to_string_lossy()),
            _ => usage(),
        },
        Some("search") => search(args.map(|arg| arg.to_string_lossy().into_owned()).collect()),
        Some("memory") => memory(args.collect()),
        Some("answer") => answer(args.map(|arg| arg.to_string_lossy().into_owned()).collect()),
        Some("dismiss") => dismiss(args.map(|arg| arg.to_string_lossy().into_owned()).collect()),
        Some("ask") => match (args.next(), args.next(), args.next()) {
            (Some(name), Some(fields), None) => ask(&name.to_string_lossy(), &fields.to_string_lossy()),
            _ => usage(),
        },
        Some("mcp") => match (args.next(), args.next(), args.next()) {
            (Some(flag), Some(session), None) if flag == "--session" => {
                let session = session.to_string_lossy();
                let local = |tool: &str, arguments: &serde_json::Value| {
                    search::tool(&session, tool, arguments, &runtime::home())
                };
                exit("mcp", mcp::serve(&session, &local))
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
        "usage: priory check [DIR]\n       priory draw [DIR] WORKFLOW\n       priory intake [DIR] [--once]\n       priory daemon [DIR] [--once]\n       priory request [DIR] --workflow <name> --key <key> [--band <band>] [--lane <lane>] [--repo <owner/name>] [--hold <reason> [CONDITION]] <text>\n       priory serve [DIR] [--listen ADDR]\n       priory inbox [--json]\n       priory tasks [--lane <lane>] [--grade red|yellow|green] [--json]\n       priory queue [--json]\n       priory runs\n       priory outbox\n       priory asks [--all] [--json]\n       priory waits\n       priory actions\n       priory attach (TASK | --run RUN) [--read-only] [--print]\n       priory desk [--all] [--json]\n       priory snooze (ask | blocked | failed | prompt) ID (--for DURATION | --until TIME | --clear)\n       priory trace\n       priory report [DIR] --workflow <name> [--since <instant-or-duration>] [--cost] [--by lane]\n       priory move TASK [--band <band>] [--top | --before TASK | --after TASK]\n       priory resume TASK\n       priory cancel TASK\n       priory revive TASK [--step STEP]\n       priory hold TASK --reason TEXT [--until TIME | --after TASK | --merge PR [--if-closed release|drop|ask] | --blockers]\n       priory release TASK\n       priory tldr TASK TEXT\n       priory search QUERY [--state STATE[,STATE...]] [--limit N]\n       priory memory [acquire --gb N [--timeout D] -- COMMAND...]\n       priory answer [--revision N | --expected N] [--] ASK OPTION-OR-TEXT\n       priory ask ID JSON\n       priory dismiss [--expected N] ASK\n       priory mcp --session ID\n       priory hook EVENT"
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

/// Attaches this terminal to the tmux session of a task's live run, or of one run, or prints the
/// command that would. tmux keeps owning the session.
fn attach(args: Vec<String>) -> ExitCode {
    let (mut target, mut print, mut read_only) = (None, false, false);
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--print" => print = true,
            "--read-only" => read_only = true,
            "--run" if target.is_none() => match args.next().map(|run| run.parse()) {
                Some(Ok(run)) => target = Some(state::Attach::Run(run)),
                _ => return usage(),
            },
            _ if target.is_none() => match arg.parse() {
                Ok(task) => target = Some(state::Attach::Task(task)),
                Err(_) => return usage(),
            },
            _ => return usage(),
        }
    }
    let Some(target) = target else {
        return usage();
    };
    let command = match state::open()
        .and_then(|db| state::live_run(&db, &target))
        .and_then(|run| {
            runtime::attach(&run.tmux, &run.session, read_only).ok_or_else(|| {
                format!(
                    "tmux session {} on this tmux server runs no live pane of run {}'s agent; \
                     run attach with the daemon's TMUX and TMUX_TMPDIR",
                    run.tmux, run.id
                )
            })
        }) {
        Ok(command) => command,
        Err(error) => return exit("attach", Err(error)),
    };
    if print {
        let quoted: Vec<_> = command.iter().map(|word| shell_word(word)).collect();
        println!("{}", quoted.join(" "));
        return ExitCode::SUCCESS;
    }
    use std::os::unix::process::CommandExt;
    let error = Command::new(&command[0]).args(&command[1..]).exec();
    exit("attach", Err(format!("{}: {error}", command[0])))
}

/// `word` as one shell word, in single quotes unless every character is plainly safe.
fn shell_word(word: &str) -> String {
    let safe = |c: char| c.is_ascii_alphanumeric() || "-_./:@%+,".contains(c);
    if !word.is_empty() && word.chars().all(safe) {
        return word.to_owned();
    }
    format!("'{}'", word.replace('\'', r"'\''"))
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

/// Sends `priory snooze KIND ID (--for DURATION | --until TIME | --clear)` to the daemon, with the
/// time in UTC to the second.
fn snooze(args: Vec<String>) -> ExitCode {
    let [kind, id, rest @ ..] = &args[..] else {
        return usage();
    };
    let Ok(id) = id.parse() else {
        return usage();
    };
    let until = match rest {
        [flag] if flag == "--clear" => None,
        [flag, span] if flag == "--for" => {
            let Ok(span) = span.parse::<jiff::Span>() else {
                return usage();
            };
            match jiff::Zoned::now().checked_add(span) {
                Ok(at) => Some(at.timestamp()),
                Err(error) => return exit("snooze", Err(error.to_string())),
            }
        }
        [flag, time] if flag == "--until" => match time.parse::<jiff::Timestamp>() {
            Ok(at) => Some(at),
            Err(error) => return exit("snooze", Err(format!("{time}: {error}"))),
        },
        _ => return usage(),
    };
    // Rounded up, so the item never comes back before the time asked for.
    let second = jiff::TimestampRound::new()
        .smallest(jiff::Unit::Second)
        .mode(jiff::RoundMode::Ceil);
    let until = match until.map(|at| at.round(second)).transpose() {
        Ok(until) => until,
        Err(error) => return exit("snooze", Err(error.to_string())),
    };
    let line = serde_json::to_string(&snooze::Snooze {
        snooze: snooze::Item {
            kind: kind.clone(),
            id,
        },
        until: until.map(|at| at.strftime("%Y-%m-%dT%H:%M:%SZ").to_string()),
    })
    .expect("a snooze serializes");
    send("snooze", &line)
}

/// Prints `priory inbox|tasks|asks|desk|queue`, as a table or, with `--json`, as the JSON that
/// `serve` returns.
fn listed(verb: &str, mut args: Vec<String>) -> ExitCode {
    let json = args
        .iter()
        .position(|arg| arg == "--json")
        .map(|at| args.remove(at));
    let all = args
        .iter()
        .position(|arg| arg == "--all" && ["asks", "desk"].contains(&verb))
        .map(|at| args.remove(at))
        .is_some();
    let (mut lane, mut grade) = (None, None);
    for pair in args.chunks(2) {
        match (verb, pair) {
            ("tasks", [flag, value]) if flag == "--lane" && lane.is_none() => {
                lane = Some(value.as_str())
            }
            ("tasks", [flag, value])
                if flag == "--grade"
                    && grade.is_none()
                    && tools::GRADES.contains(&value.as_str()) =>
            {
                grade = Some(value.as_str())
            }
            _ => return usage(),
        }
    }
    let home = runtime::home();
    let shown = state::open().and_then(|db| {
        let value = match verb {
            "inbox" => listing::inbox(&db),
            "tasks" => listing::tasks(&db, lane, grade),
            "asks" => listing::asks(&db),
            "desk" => listing::desk(&db, &home, all),
            _ => listing::queue(&db),
        }
        .map_err(|error| error.to_string())?;
        if json.is_some() {
            state::emit(&value.to_string());
            return Ok(());
        }
        match verb {
            "inbox" => listing::print(&["seq", "type", "subject", "task", "outcome"], &value),
            "tasks" => listing::print(
                &[
                    "id",
                    "key",
                    "workflow",
                    "step",
                    "state",
                    "band",
                    "lane",
                    "reason",
                    "position",
                    "generation",
                    "retired",
                    "grade",
                    "grade_reason",
                ],
                &value,
            ),
            "asks" => {
                let open = value
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|ask| ask["state"] == "open" && (all || ask["snoozed_until"].is_null()))
                    .cloned()
                    .collect();
                listing::print(
                    &[
                        "id",
                        "task",
                        "revision",
                        "question",
                        "options",
                        "snoozed_until",
                        "back_early",
                        "title",
                        "text",
                        "what_waits",
                        "next_action",
                        "command",
                        "coordinator",
                        "key",
                        "run_id",
                        "session",
                    ],
                    &serde_json::Value::Array(open),
                );
            }
            "desk" => {
                if let Some(banner) = value["banner"].as_str() {
                    state::emit(banner);
                }
                listing::print(
                    &[
                        "since",
                        "kind",
                        "id",
                        "task",
                        "lane",
                        "what",
                        "generation",
                        "snoozed_until",
                        "back_early",
                    ],
                    &value["items"],
                );
            }
            _ => listing::print(
                &[
                    "position",
                    "band_position",
                    "id",
                    "key",
                    "workflow",
                    "step",
                    "state",
                    "band",
                    "lane",
                    "reason",
                ],
                &value["waiting"],
            ),
        }
        Ok(())
    });
    exit(verb, shown)
}

/// Serves the state over HTTP until killed: `priory serve [DIR] [--listen ADDR]`.
fn serve_http(args: Vec<String>) -> ExitCode {
    let (mut dirs, mut listen) = (Vec::new(), None);
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--listen" => match args.next().map(|addr| addr.parse()) {
                Some(Ok(addr)) => listen = Some(addr),
                _ => return usage(),
            },
            _ if arg.starts_with("--") => return usage(),
            _ => dirs.push(arg),
        }
    }
    let dir = match &dirs[..] {
        [] => PathBuf::from("."),
        [dir] => PathBuf::from(dir),
        _ => return usage(),
    };
    let Some(factory) = load(&dir) else {
        return ExitCode::FAILURE;
    };
    let Some(config) = factory.serve() else {
        return exit(
            "serve",
            Err("factory.toml has no [serve] table, so nothing listens".to_owned()),
        );
    };
    match serve::start(config, listen.unwrap_or(config.listen)) {
        Ok(workers) => {
            for worker in workers {
                let _ = worker.join();
            }
            ExitCode::SUCCESS
        }
        Err(error) => exit("serve", Err(error)),
    }
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
/// How long a grant waits without `--timeout`: two runs' grants can wait on each other's promises.
const GATE_TIMEOUT: Duration = Duration::from_secs(60 * 60);

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
    let timeout = timeout.unwrap_or(GATE_TIMEOUT);
    let deadline = Instant::now() + timeout;
    let mut said = String::new();
    while !reply.ends_with(" held") {
        if said != reply {
            eprintln!("priory memory: grant {reply} for {gb} GB");
            said = reply.clone();
        }
        if Instant::now() >= deadline {
            let _ = gate(&Gate::Release { grant });
            return failed(format!("grant {grant} still waits after {timeout:?}"));
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
fn answer(args: Vec<String>) -> ExitCode {
    let Some((Some(revision), rest)) = revised(args) else {
        return usage();
    };
    let [ask, text] = &rest[..] else {
        return usage();
    };
    let ask = match ask_id(ask) {
        Ok(ask) => ask,
        Err(error) => return exit("answer", Err(error)),
    };
    let line = serde_json::to_string(&tools::Answer {
        ask,
        revision,
        text: text.clone(),
    })
    .expect("an answer serializes");
    send("answer", &line)
}

/// Sends `priory dismiss [--expected N] ASK` to the daemon, which closes the ask unanswered.
fn dismiss(args: Vec<String>) -> ExitCode {
    let Some((revision, rest)) = revised(args) else {
        return usage();
    };
    let [ask] = &rest[..] else {
        return usage();
    };
    let dismiss = match ask_id(ask) {
        Ok(dismiss) => dismiss,
        Err(error) => return exit("dismiss", Err(error)),
    };
    let line =
        serde_json::to_string(&tools::Dismiss { dismiss, revision }).expect("a dismiss serializes");
    send("dismiss", &line)
}

/// Splits `[--revision N | --expected N] [--] ARGS` into the revision and the other arguments.
fn revised(args: Vec<String>) -> Option<(Option<i64>, Vec<String>)> {
    let (mut revision, mut rest, mut args) = (None, Vec::new(), args.into_iter());
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--" => {
                rest.extend(args);
                break;
            }
            "--revision" | "--expected" if revision.is_none() => {
                revision = Some(args.next()?.parse().ok()?);
            }
            _ => rest.push(arg),
        }
    }
    Some((revision, rest))
}

/// An ask's number, or the number of the outside ask that `priory ask` named so.
fn ask_id(ask: &str) -> Result<i64, String> {
    // `priory ask` refuses an all-digit ID, so only one is a number.
    if let Some(id) = ask
        .bytes()
        .all(|byte| byte.is_ascii_digit())
        .then(|| ask.parse().ok())
        .flatten()
    {
        return Ok(id);
    }
    tools::ask_named(&state::open()?, ask)
}

/// Stores or revises an ask from a session Priory did not start, from Fleet's JSON shape.
fn ask(name: &str, fields: &str) -> ExitCode {
    let stored = serde_json::from_str(fields)
        .map_err(|error| format!("ask fields must be a JSON object: {error}"))
        .and_then(|fields| {
            let mut db = state::open()?;
            tools::ask_outside(&mut db, name, &fields).map_err(|error| error.to_string())?
        })
        .map(|ignored| {
            for field in ignored {
                eprintln!("priory ask: ignored unknown field {field}");
            }
            println!("{name}");
        });
    exit("ask", stored)
}

/// Sets a task's TL;DR by hand.
fn tldr(task: &str, text: &str) -> ExitCode {
    let Ok(task) = task.parse() else {
        return usage();
    };
    let set = state::open()
        .and_then(|db| {
            let counter = search::counter(&db, &runtime::home());
            search::set(&db, task, text, counter.as_ref())
        })
        .map(|done| println!("{done}"));
    exit("tldr", set)
}

/// Prints the tasks that match a query, best first.
fn search(args: Vec<String>) -> ExitCode {
    let (mut query, mut states, mut limit) = (None, Vec::new(), search::LIMIT);
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match (arg.as_str(), query.is_some()) {
            ("--state", _) => match args.next() {
                Some(value) => states.extend(value.split(',').map(str::to_owned)),
                None => return usage(),
            },
            ("--limit", _) => match args.next().map(|value| value.parse()) {
                Some(Ok(value)) => limit = value,
                _ => return usage(),
            },
            (flag, _) if flag.starts_with("--") => return usage(),
            (_, true) => return usage(),
            (_, false) => query = Some(arg),
        }
    }
    let Some(query) = query else {
        return usage();
    };
    let mut embedder = embed::Lazy::new(&runtime::home(), false);
    let printed = state::open()
        .and_then(|db| search::search(&db, &query, &states, limit, &mut embedder))
        .map(|found| {
            if let Some(why) = found.unavailable {
                eprintln!("priory search: embeddings are unavailable, so this matched keywords only: {why}");
            }
            print!("{}", found.table);
        });
    exit("search", printed)
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
