use priory_core::memory::Gate;
use priory_core::{
    daemon, embed, listing, post, progress, report, search, sections, serve, snooze, state, tools,
};
use priory_factory::{explain, factory, rules};
use priory_intake::{intake, request};
use priory_runtime::{mcp, runtime};
use serde_json::Value;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::{Duration, Instant};

fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    match args.next().as_deref().and_then(|verb| verb.to_str()) {
        Some("check") => check(args.collect()),
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
        Some("post") => match (args.next(), args.next(), args.next()) {
            (Some(source), Some(line), None) => {
                post(&source.to_string_lossy(), &line.to_string_lossy())
            }
            _ => usage(),
        },
        Some(verb @ ("inbox" | "tasks" | "asks" | "desk" | "queue")) => {
            listed(verb, args.map(|arg| arg.to_string_lossy().into_owned()).collect())
        }
        Some("wait") => wait(args.map(|arg| arg.to_string_lossy().into_owned()).collect()),
        Some("schedules") => {
            schedules(args.map(|arg| arg.to_string_lossy().into_owned()).collect())
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
        Some(verb @ ("resume" | "cancel" | "revive" | "release" | "say")) => act(
            verb,
            args.map(|arg| arg.to_string_lossy().into_owned()).collect(),
            None,
        ),
        Some("pause") => match (args.next(), args.next()) {
            (reason, None) => {
                let reason = reason.map(|reason| reason.to_string_lossy().into_owned());
                let paused = state::open().and_then(|db| {
                    state::pause(&db, reason.as_deref()).map_err(|error| error.to_string())
                });
                exit("pause", paused.map(|text| println!("{text}")))
            }
            _ => usage(),
        },
        Some("seen") => seen(args.map(|arg| arg.to_string_lossy().into_owned()).collect()),
        Some("unpause") => match args.next() {
            None => {
                let unpaused = state::open()
                    .and_then(|db| state::unpause(&db).map_err(|error| error.to_string()));
                exit("unpause", unpaused.map(|text| println!("{text}")))
            }
            Some(_) => usage(),
        },
        Some("hold") => hold(args.map(|arg| arg.to_string_lossy().into_owned()).collect()),
        Some("actions") => list(
            "id\ttask\tstep\taction\tstatus\tpull_request\tcommit\terror\tstdout\tstderr",
            "SELECT id, task, step, action, status, pull_request, commit_sha,
             replace(error, char(10), '\\n'), replace(stdout, char(10), '\\n'),
             replace(stderr, char(10), '\\n') FROM actions ORDER BY id",
        ),
        Some("task") => match (args.next(), args.next(), args.next()) {
            (Some(task), Some(flag), None) if flag == "--json" => task_json(&task.to_string_lossy()),
            _ => usage(),
        },
        Some("tldr") => match (args.next(), args.next(), args.next()) {
            (Some(task), Some(text), None) => tldr(&task.to_string_lossy(), &text.to_string_lossy()),
            _ => usage(),
        },
        Some("progress") => match (args.next(), args.next(), args.next()) {
            (Some(task), Some(fields), None) => {
                progress(&task.to_string_lossy(), &fields.to_string_lossy())
            }
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
                    sections::tool(&session, tool, arguments)
                        .or_else(|| search::tool(&session, tool, arguments, &runtime::home()))
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
        "usage: priory check [DIR] [--explain [--json]]\n       priory draw [DIR] WORKFLOW\n       priory intake [DIR] [--once]\n       priory daemon [DIR] [--once]\n       priory request [DIR] --workflow <name> --key <key> [--band <band>] [--lane <lane>] [--repo <owner/name>] [--hold <reason> [CONDITION]...] <text>\n       priory serve [DIR] [--listen ADDR]\n       priory inbox [--since SEQ] [--json]\n       priory inbox read SEQ\n       priory post SOURCE LINE\n       priory tasks [--lane <lane>] [--grade red|yellow|green] [--json]\n       priory queue [DIR] [--json]\n       priory schedules [DIR] [--json]\n       priory runs\n       priory outbox\n       priory asks [--all] [--band <band>[,<band>...]]... [--risk <risk>[,<risk>...]]... [--json]\n       priory waits\n       priory actions\n       priory attach (TASK | --run RUN) [--read-only] [--print]\n       priory desk [--all] [--json] [--person NAME]\n       priory seen [--person NAME] [--at TIME]\n       priory wait --since REV [--timeout D]\n       priory snooze (ask | blocked | failed | prompt) ID (--for DURATION | --until TIME | --clear)\n       priory trace\n       priory report put TASK FILE|-\n       priory report show [TASK] [--json]\n       priory report pick PICK OPTION --expected REVISION\n       priory report [DIR] --workflow <name> [--since <instant-or-duration>] [--cost] [--by lane]\n       priory move TASK [--band <band>] [--top | --before TASK | --after TASK]\n       priory resume TASK\n       priory say TASK TEXT\n       priory pause [REASON]\n       priory unpause\n       priory cancel TASK\n       priory revive TASK [--step STEP]\n       priory hold TASK --reason TEXT [--until TIME] [--after TASK-OR-ISSUE]... [--merge PR]... [--if-closed release|drop|ask] [--blockers]\n       priory release TASK\n       priory task TASK --json\n       priory tldr TASK TEXT\n       priory progress TASK JSON\n       priory search QUERY [--state STATE[,STATE...]] [--limit N]\n       priory memory [(acquire | lease) [--gb N] [--timeout D] -- COMMAND...]\n       priory answer [--revision N | --expected N] [--run] [--] ASK OPTION-OR-TEXT\n       priory ask ID JSON\n       priory dismiss [--expected N] ASK\n       priory mcp --session ID\n       priory hook EVENT"
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
    match args.first().map(String::as_str) {
        Some("put") => return report_put(&args[1..]),
        Some("show") => return report_show(&args[1..]),
        Some("pick") => return report_pick(&args[1..]),
        _ => {}
    }
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

/// Stores a task's report from a file, or from stdin with `-`: `priory report put TASK FILE|-`.
fn report_put(args: &[String]) -> ExitCode {
    let [task, file] = args else {
        return usage();
    };
    let Ok(task) = task.parse() else {
        return usage();
    };
    let text = if file == "-" {
        std::io::read_to_string(std::io::stdin())
    } else {
        std::fs::read_to_string(file)
    };
    let put = text
        .map_err(|error| format!("{file}: {error}"))
        .and_then(|text| sections::parse(&text))
        .and_then(|report| state::open().and_then(|mut db| sections::put(&mut db, task, report)))
        .map(|done| println!("{done}"));
    exit("report put", put)
}

/// Sends `priory report pick PICK OPTION --expected REVISION` to the daemon, which applies it.
fn report_pick(args: &[String]) -> ExitCode {
    let [pick, option, flag, expected] = args else {
        return usage();
    };
    let (true, Ok(expected)) = (flag == "--expected", expected.parse()) else {
        return usage();
    };
    let line = serde_json::to_string(&sections::Choice {
        pick: pick.clone(),
        option: option.clone(),
        expected,
    })
    .expect("a choice serializes");
    send("report pick", &line)
}

/// Prints a task's report, or the one written last: `priory report show [TASK] [--json]`.
fn report_show(args: &[String]) -> ExitCode {
    let (json, rest): (Vec<_>, Vec<_>) = args.iter().partition(|arg| *arg == "--json");
    let task = match rest[..] {
        [] => None,
        [task] => match task.parse() {
            Ok(task) => Some(task),
            Err(_) => return usage(),
        },
        _ => return usage(),
    };
    let shown = state::open().and_then(|db| {
        let report = listing::revised(&db, "report", |db| sections::show(db, task))
            .map_err(|error| error.to_string())?
            .ok_or_else(|| match task {
                Some(task) => format!("task {task} has no report"),
                None => "no task has a report".to_owned(),
            })?;
        if !json.is_empty() {
            state::emit(&report.to_string());
            return Ok(());
        }
        let report = &report["report"];
        state::emit(&format!(
            "task {} revision {} {}: {}",
            report["task"], report["revision"], report["date"], report["greeting"]
        ));
        for section in report["sections"].as_array().into_iter().flatten() {
            state::emit(&format!("{}", section["title"]));
            for row in section["rows"].as_array().into_iter().flatten() {
                let pick = &row["pick"];
                let choice = match &pick["chosen"] {
                    Value::Null if pick.is_object() => {
                        format!("  pick {}: {}", pick["id"], pick["options"])
                    }
                    Value::Null => String::new(),
                    chosen => format!("  pick {}: {chosen}", pick["id"]),
                };
                state::emit(&format!("  {}{choice}", row["text"]));
            }
        }
        Ok(())
    });
    exit("report show", shown)
}

/// `priory seen [--person NAME] [--at TIME]` records a visit to the desk, now by default.
fn seen(args: Vec<String>) -> ExitCode {
    let (mut person, mut at) = (None, None);
    for pair in args.chunks(2) {
        match pair {
            [flag, value] if flag == "--person" && person.is_none() => person = Some(value),
            [flag, value] if flag == "--at" && at.is_none() => match value.parse() {
                Ok(time) => at = Some(time),
                Err(error) => return exit("seen", Err(format!("{value}: {error}"))),
            },
            _ => return usage(),
        }
    }
    // Whole seconds, as inbox times are: the desk lists events from the visit's second on, so an
    // event in that second shows again rather than never.
    let at: jiff::Timestamp = at.unwrap_or_else(jiff::Timestamp::now);
    let at = at.strftime("%Y-%m-%dT%H:%M:%SZ").to_string();
    let person = person.map_or(listing::ME, String::as_str);
    let done = state::open()
        .and_then(|db| state::seen(&db, person, &at).map_err(|error| error.to_string()));
    exit("seen", done.map(|text| println!("{text}")))
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
        [flag] if flag == "--clear" => snooze::until(None, None),
        [flag, span] if flag == "--for" => snooze::until(Some(span), None),
        [flag, time] if flag == "--until" => snooze::until(None, Some(time)),
        _ => return usage(),
    };
    let until = match until {
        Ok(until) => until,
        Err(error) => return exit("snooze", Err(error)),
    };
    let line = serde_json::to_string(&snooze::Snooze {
        snooze: snooze::Item {
            kind: kind.clone(),
            id,
        },
        until,
    })
    .expect("a snooze serializes");
    send("snooze", &line)
}

/// Prints `priory inbox|tasks|asks|desk|queue`, as a table or, with `--json`, as the JSON that
/// `serve` returns. `priory inbox read SEQ` marks a report read instead.
fn listed(verb: &str, mut args: Vec<String>) -> ExitCode {
    if verb == "inbox" && args.first().is_some_and(|arg| arg == "read") {
        let [_, seq] = &args[..] else {
            return usage();
        };
        let Ok(seq) = seq.parse() else {
            return usage();
        };
        return exit(
            "inbox read",
            state::open().and_then(|db| post::read(&db, seq)),
        );
    }
    let json = args
        .iter()
        .position(|arg| arg == "--json")
        .map(|at| args.remove(at));
    let all = args
        .iter()
        .position(|arg| arg == "--all" && ["asks", "desk"].contains(&verb))
        .map(|at| args.remove(at))
        .is_some();
    // Without the factory folder, the queue knows no schedules, so up next lists none.
    let (mut schedules, mut now) = (Vec::new(), jiff::Timestamp::now());
    if verb == "queue" {
        now = match intake::now() {
            Ok(now) => now,
            Err(error) => return exit(verb, Err(error)),
        };
        if let [dir] = &args[..] {
            if dir.starts_with("--") {
                return usage();
            }
            let Some(factory) = load(Path::new(dir)) else {
                return ExitCode::FAILURE;
            };
            schedules = listing::schedule_triggers(&factory);
            args.clear();
        }
    }
    let (mut lane, mut grade, mut since, mut person) = (None, None, None, None);
    let (mut bands, mut risks) = (Vec::new(), Vec::new());
    for pair in args.chunks(2) {
        match (verb, pair) {
            ("asks", [flag, value]) if flag == "--band" => bands.extend(value.split(',')),
            ("asks", [flag, value]) if flag == "--risk" => risks.extend(value.split(',')),
            ("desk", [flag, value]) if flag == "--person" && person.is_none() => {
                person = Some(value.as_str())
            }
            ("inbox", [flag, value]) if flag == "--since" && since.is_none() => {
                let Ok(value) = value.parse() else {
                    return usage();
                };
                since = Some(value)
            }
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
        for band in &bands {
            state::check_band(&db, band)?;
        }
        for risk in &risks {
            listing::check_risk(risk)?;
        }
        let value = listing::revised(&db, verb, |db| {
            match verb {
                "inbox" => listing::inbox(db, since.unwrap_or(0)),
                "tasks" => listing::tasks(db, lane, grade),
                "asks" => listing::asks(db, &bands, &risks),
                "desk" => listing::desk(db, &home, all, person.unwrap_or(listing::ME)),
                _ => listing::queue(db, &schedules, now),
            }
            .map(Some)
        })
        .map_err(|error| error.to_string())?
        .expect("a listing always reads");
        if json.is_some() {
            state::emit(&value.to_string());
            return Ok(());
        }
        let rows = &value[verb];
        match verb {
            "inbox" => listing::print(&["seq", "type", "subject", "task", "outcome", "line"], rows),
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
                rows,
            ),
            "asks" => {
                let open = rows
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
                if let Some(paused) = value["paused"].as_str() {
                    state::emit(paused);
                }
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

/// Lists each schedule trigger with its next firing: `priory schedules [DIR] [--json]`.
fn schedules(mut args: Vec<String>) -> ExitCode {
    let json = args
        .iter()
        .position(|arg| arg == "--json")
        .map(|at| args.remove(at));
    let dir = match &args[..] {
        [] => PathBuf::from("."),
        [dir] if !dir.starts_with("--") => PathBuf::from(dir),
        _ => return usage(),
    };
    let Some(factory) = load(&dir) else {
        return ExitCode::FAILURE;
    };
    let shown = intake::now().and_then(|now| {
        let db = state::open()?;
        let schedules = listing::schedule_triggers(&factory);
        let value = listing::revised(&db, "schedules", |db| {
            listing::schedules(db, &schedules, now).map(Some)
        })
        .map_err(|error| error.to_string())?
        .expect("a listing always reads");
        match json {
            Some(_) => state::emit(&value.to_string()),
            None => listing::print(
                &["trigger", "next", "key", "task", "workflow"],
                &value["schedules"],
            ),
        }
        Ok(())
    });
    exit("schedules", shown)
}

/// How long `priory wait` waits without `--timeout`, as long as `serve`'s `/api/events`.
const WAIT_TIMEOUT: Duration = Duration::from_secs(25);

/// Prints the state's revision once it differs from `--since`, or the same one after the timeout:
/// `priory wait --since REV [--timeout D]`.
fn wait(args: Vec<String>) -> ExitCode {
    let (mut since, mut timeout) = (None, WAIT_TIMEOUT);
    for pair in args.chunks(2) {
        match pair {
            [flag, value] if flag == "--since" && since.is_none() => match value.parse::<i64>() {
                Ok(value) => since = Some(value),
                Err(_) => return usage(),
            },
            [flag, value] if flag == "--timeout" => {
                let Some(value) = value
                    .parse::<jiff::SignedDuration>()
                    .ok()
                    .and_then(|value| Duration::try_from(value).ok())
                else {
                    return usage();
                };
                timeout = value;
            }
            _ => return usage(),
        }
    }
    let Some(since) = since else {
        return usage();
    };
    let waited = state::open().and_then(|db| {
        state::wait(&db, since, timeout)
            .map(|revision| state::emit(&revision.to_string()))
            .map_err(|error| error.to_string())
    });
    exit("wait", waited)
}

/// Stores one report line on the inbox and prints its sequence number: `priory post SOURCE LINE`.
fn post(source: &str, line: &str) -> ExitCode {
    let posted = state::open()
        .and_then(|db| post::post(&db, source, line))
        .map(|seq| state::emit(&seq.to_string()));
    exit("post", posted)
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
    match serve::start(&factory, listen.unwrap_or(config.listen)) {
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
    match request::run(&factory, &request, None) {
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
            "id\ttask\tstate\tgb\tcommand\tage\tpid",
            "SELECT id, task, state, printf('%.1f', bytes / 1073741824.0),
             replace(command, char(10), '\\n'),
             (strftime('%s', 'now') - strftime('%s', coalesce(granted, created))) || 's', pid
             FROM grants ORDER BY id",
        ),
        Some((verb, args)) if verb == "acquire" => acquire(args, false),
        Some((verb, args)) if verb == "lease" => acquire(args, true),
        Some(_) => usage(),
    }
}

/// The exit code of `priory memory acquire` when the gate itself fails, as `env` and `timeout` use.
const GATE_FAILED: u8 = 125;
/// How long a grant waits without `--timeout`: two runs' grants can wait on each other's promises.
const GATE_TIMEOUT: Duration = Duration::from_secs(60 * 60);
/// The GB a grant asks for without `--gb`, as Fleet's gate did and agents' instructions assume.
const GATE_GB: f64 = 10.0;

/// Runs a command once the daemon grants it `--gb` GB of memory, and releases the grant when it
/// exits. Exits with the command's code. A `lease` keeps the grant and stops the command when it dies.
fn acquire(args: &[OsString], lease: bool) -> ExitCode {
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
    let gb = gb.unwrap_or(GATE_GB);
    let Some((program, rest)) = command.split_first() else {
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
    let status = if lease {
        leased(program, rest)
    } else {
        Command::new(program)
            .args(rest)
            .spawn()
            .and_then(|mut child| {
                // The command holds the grant from here, so it keeps its memory if this process dies.
                let _ = gate(&Gate::Run {
                    grant,
                    pid: child.id(),
                });
                child.wait()
            })
    };
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

/// Runs a leased command in the process group of a guard that stops the group once its pipe from
/// this process closes, however this process ends.
fn leased(program: &OsString, rest: &[OsString]) -> std::io::Result<std::process::ExitStatus> {
    use std::os::unix::process::CommandExt;
    use std::process::Stdio;
    let guard = Command::new("sh")
        .args([
            "-c",
            "trap '' TERM; read _; kill -TERM -$$; sleep 5; kill -KILL -$$",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .map_err(|error| std::io::Error::other(format!("its guard did not start: {error}")))?;
    let status = Command::new(program)
        .args(rest)
        .stdin(Stdio::null())
        .process_group(guard.id() as i32)
        .spawn()
        .and_then(|mut child| child.wait());
    // Dropping the guard closes its pipe, so it also stops what the command left behind.
    drop(guard);
    status
}

/// Sends `gate` to the daemon and returns its reply.
fn gate(gate: &Gate) -> Result<String, String> {
    let line = serde_json::to_string(gate).expect("a gate message serializes");
    runtime::call(&runtime::home(), &line).and_then(|reply| {
        serde_json::from_str::<Result<String, String>>(&reply)
            .map_err(|error| format!("unreadable reply {reply:?}: {error}"))?
    })
}

/// Sends `priory answer ASK TEXT --revision N [--run]` to the daemon, which applies it. `--run`
/// confirms an answer that runs the ask's command.
fn answer(mut args: Vec<String>) -> ExitCode {
    let run = args
        .iter()
        .take_while(|arg| *arg != "--")
        .position(|arg| arg == "--run");
    if let Some(at) = run {
        args.remove(at);
    }
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
        run: run.is_some(),
        by: "cli".to_owned(),
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
        .map(|mut fields: serde_json::Value| {
            if let (Some(fields), Some(session)) = (fields.as_object_mut(), asking_session()) {
                fields.entry("session").or_insert(session.into());
            }
            fields
        })
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

/// The hc terminal that runs `priory ask`, then the Claude session in it when there is one, so the
/// answer can reach it, or resume it once the terminal closed.
fn asking_session() -> Option<String> {
    let terminal = std::env::var("HC_TERMINAL_ID")
        .ok()
        .filter(|id| !id.is_empty())?;
    Some(match std::env::var("CLAUDE_CODE_SESSION_ID") {
        Ok(claude) if !claude.is_empty() => format!("{terminal} {claude}"),
        _ => terminal,
    })
}

/// Sets a task's TL;DR by hand.
/// Prints `priory task TASK --json`: the body `/api/tasks/TASK` returns.
fn task_json(task: &str) -> ExitCode {
    let Ok(id) = task.parse() else {
        return usage();
    };
    let shown = state::open().and_then(|db| {
        match listing::revised(&db, "task", |db| listing::task(db, id))
            .map_err(|error| error.to_string())?
        {
            Some(task) => {
                state::emit(&task.to_string());
                Ok(())
            }
            None => Err(format!("no task {id}")),
        }
    });
    exit("task", shown)
}

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

/// Sends `priory progress TASK JSON` to the daemon.
fn progress(task: &str, fields: &str) -> ExitCode {
    let Ok(task) = task.parse() else {
        return usage();
    };
    let progress = match serde_json::from_str(fields) {
        Ok(progress) => progress,
        Err(error) => return exit("progress", Err(format!("{fields}: {error}"))),
    };
    let line = serde_json::to_string(&progress::Put { task, progress }).expect("a put serializes");
    send("progress", &line)
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
            "--until" if hold.until.is_none() => hold.until = Some(value),
            "--after" => match value.parse() {
                Ok(other) => hold.after.push(other),
                Err(error) => {
                    eprintln!("priory hold: {error}");
                    return ExitCode::FAILURE;
                }
            },
            "--merge" => hold.merge.push(value),
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

/// Sends `priory resume`, `cancel`, `revive TASK [--step STEP]`, `release`, `say TASK TEXT` or `hold` for one task to
/// the daemon, which applies it.
fn act(act: &str, mut args: Vec<String>, hold: Option<factory::Hold>) -> ExitCode {
    // Taken first, so a text such as `--step` stays text.
    let text = match act {
        "say" if args.len() == 2 => args.pop(),
        "say" => return usage(),
        _ => None,
    };
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
        text,
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

/// `priory check [DIR] [--explain [--json]]`.
fn check(args: Vec<OsString>) -> ExitCode {
    let (flags, dirs): (Vec<_>, Vec<_>) = args
        .iter()
        .partition(|arg| arg.to_str().is_some_and(|arg| arg.starts_with("--")));
    let flags: Vec<_> = flags.iter().filter_map(|flag| flag.to_str()).collect();
    let (explain, json) = match flags[..] {
        [] => (false, false),
        ["--explain"] => (true, false),
        ["--explain", "--json"] | ["--json", "--explain"] => (true, true),
        _ => return usage(),
    };
    let dir = match &dirs[..] {
        [] => PathBuf::from("."),
        [dir] => PathBuf::from(dir),
        _ => return usage(),
    };
    let Some(factory) = load(&dir) else {
        return ExitCode::FAILURE;
    };
    if explain {
        print_explanation(&factory, json);
        return ExitCode::SUCCESS;
    }
    println!(
        "ok: {}, {}, {}, {}",
        count(factory.agents.len(), "agent"),
        count(factory.triggers.len(), "trigger"),
        count(factory.workflows.len(), "workflow"),
        count(factory.runners.len(), "runner"),
    );
    ExitCode::SUCCESS
}

/// Prints each workflow's irreversible steps with their gates, and its paths to one with no human
/// edge, as two tables or as JSON.
fn print_explanation(factory: &factory::Factory, json: bool) {
    let gates: Vec<_> = factory
        .workflows
        .iter()
        .flat_map(|(name, workflow)| {
            explain::gates(&workflow.value)
                .into_iter()
                .map(move |gate| (name, gate))
        })
        .collect();
    let paths: Vec<_> = factory
        .workflows
        .iter()
        .flat_map(|(name, workflow)| {
            explain::unattended_paths(&workflow.value)
                .into_iter()
                .map(move |path| (name, path))
        })
        .collect();
    if json {
        let irreversible: Vec<_> = gates
            .iter()
            .map(|(name, gate)| {
                serde_json::json!({
                    "workflow": name,
                    "step": gate.step,
                    "approved_by": gate.approved_by,
                    "autonomous": gate.autonomous,
                })
            })
            .collect();
        let paths: Vec<_> = paths
            .iter()
            .map(|(name, path)| serde_json::json!({ "workflow": name, "path": path }))
            .collect();
        state::emit(
            &serde_json::json!({ "irreversible": irreversible, "paths": paths }).to_string(),
        );
        return;
    }
    let cell = |text: &str| {
        text.replace('\t', "\\t")
            .replace('\r', "\\r")
            .replace('\n', "\\n")
    };
    state::emit("workflow\tstep\tgate");
    for (name, gate) in &gates {
        let shown = match &gate.autonomous {
            Some(reason) => format!("autonomous: {reason}"),
            None => gate.approved_by.join(", "),
        };
        state::emit(&format!(
            "{}\t{}\t{}",
            cell(name),
            cell(&gate.step),
            cell(&shown)
        ));
    }
    state::emit("");
    state::emit("workflow\tpath with no human decision");
    for (name, path) in &paths {
        state::emit(&format!("{}\t{}", cell(name), cell(&path.join(" -> "))));
    }
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
