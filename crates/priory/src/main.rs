use priory_core::memory::Gate;
use priory_core::{
    daemon, deliver, embed, grades, listing, post, progress, proof, relate, report, risk, runners,
    search, sections, serve, snooze, state, subscribe, todo, tools,
};
use priory_factory::{explain, factory, rules};
use priory_intake::{intake, request};
use priory_runtime::{eval, mcp, runtime};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::{Duration, Instant};

fn main() -> ExitCode {
    let words: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    if std::env::var_os("PRIORY_SESSION").is_some() && priory_factory::guard::person_verb(&words) {
        eprintln!(
            "priory {}: only a person runs it, and this shell is a run's (PRIORY_SESSION is set)",
            words[0]
        );
        return ExitCode::from(2);
    }
    let mut args = std::env::args_os().skip(1);
    match args.next().as_deref().and_then(|verb| verb.to_str()) {
        Some("--version" | "-V") => {
            println!("priory {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
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
                |factory: &factory::Factory, once| {
                    intake::run(factory, once, watched, merged, health, ended)
                }
            } else {
                daemon::run
            };
            serve(verb, &dir, run, !once.is_empty())
        }
        Some("batch") => batch(args.map(|arg| arg.to_string_lossy().into_owned()).collect()),
        Some("add") => add(args.map(|arg| arg.to_string_lossy().into_owned()).collect()),
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
        Some("links") => links(args.map(|arg| arg.to_string_lossy().into_owned()).collect()),
        Some("flows") => flows(args.map(|arg| arg.to_string_lossy().into_owned()).collect()),
        Some("views") => views(args.map(|arg| arg.to_string_lossy().into_owned()).collect()),
        Some("todo") => todo(args.map(|arg| arg.to_string_lossy().into_owned()).collect()),
        Some("deliver") => deliver(args.map(|arg| arg.to_string_lossy().into_owned()).collect()),
        Some("snooze") => snooze(args.map(|arg| arg.to_string_lossy().into_owned()).collect()),
        Some("serve") => serve_http(args.map(|arg| arg.to_string_lossy().into_owned()).collect()),
        Some("runs") => list(
            "id\ttask\tstep\ttmux\tsession\tstarted\tended\tend\tkit",
            "SELECT id, task, step, tmux, coalesce(thread, session), started, ended, ended_as, kit_hash
             FROM runs ORDER BY id",
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
        Some("costs") => costs(args.map(|arg| arg.to_string_lossy().into_owned()).collect()),
        Some("grades") => grades(args.map(|arg| arg.to_string_lossy().into_owned()).collect()),
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
            (Some(task), None, None) => task_text(&task.to_string_lossy()),
            _ => usage(),
        },
        Some(verb @ ("subscribe" | "unsubscribe")) => {
            match expected(args.map(|arg| arg.to_string_lossy().into_owned()).collect()) {
                Some((revision, args)) if revision.is_none() || verb == "subscribe" => {
                    match &args[..] {
                        [task, resource] => subscription(verb, task, resource, revision),
                        _ => usage(),
                    }
                }
                _ => usage(),
            }
        }
        Some("relate") => relate(args.map(|arg| arg.to_string_lossy().into_owned()).collect()),
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
        Some("gate") if args.next().is_none() => project_gate(),
        Some("answer") => answer(args.map(|arg| arg.to_string_lossy().into_owned()).collect()),
        Some("give") => {
            let mut args: Vec<String> = args.map(|arg| arg.to_string_lossy().into_owned()).collect();
            let before = args.len();
            args.retain(|arg| arg != "--force");
            let force = args.len() < before;
            let (dir, key, runner) = match &args[..] {
                [key, runner] => (".", key, runner),
                [dir, key, runner] => (dir.as_str(), key, runner),
                _ => return usage(),
            };
            let Some(factory) = load(Path::new(dir)) else {
                return ExitCode::FAILURE;
            };
            reply(
                "give",
                state::read_only().and_then(|db| runners::give(&factory, &db, key, runner, force)),
            )
        }
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
                        .or_else(|| relate::tool(&session, tool, arguments))
                };
                exit("mcp", mcp::serve(&session, &local))
            }
            (Some(flag), Some(log), describe) if flag == "--eval" => {
                let rest: Vec<String> = describe
                    .into_iter()
                    .chain(args)
                    .map(|arg| arg.to_string_lossy().into_owned())
                    .collect();
                let describe = match rest.as_slice() {
                    [] => None,
                    [flag, tool, text] if flag == "--describe" => Some((tool.as_str(), text.as_str())),
                    _ => return usage(),
                };
                exit("mcp", eval::serve(Path::new(&log), describe))
            }
            _ => usage(),
        },
        Some("eval") => prompt_eval(args.map(|arg| arg.to_string_lossy().into_owned()).collect()),
        Some("hook") => match (args.next(), args.next()) {
            (Some(event), None) => hook(&event.to_string_lossy()),
            _ => usage(),
        },
        _ => usage(),
    }
}

fn usage() -> ExitCode {
    eprintln!(
        "usage: priory --version\n       priory check [DIR] [--explain [--json]]\n       priory draw [DIR] WORKFLOW\n       priory intake [DIR] [--once]\n       priory daemon [DIR] [--once]    --once routes pending events, starts no task and exits\n       priory batch add [DIR] ISSUE... [--repo <owner/name>] [--entries <workflow>] [--band <band>] [--lane <lane>]\n       priory batch show [BATCH] [--json]\n       priory batch edit BATCH --expected REVISION (--drop ENTRY [--with-successors] | --move ENTRY (--top | --before OTHER | --after OTHER) | --moment ENTRY OTHER MOMENT)\n       priory add [DIR] [--workflow <name>] [--band <band>] [--lane <lane>] [--repo <owner/name>] [--intervention plan|hands-off] [--relates KIND:TASK]... <text>\n       priory request [DIR] --workflow <name> --key <key> [--band <band>] [--lane <lane>] [--repo <owner/name>] [--intervention plan|hands-off] [--relates KIND:TASK]... [--hold <reason> [CONDITION]...] <text>\n       priory serve [DIR] [--listen ADDR]\n       priory inbox [--unrouted] [--since SEQ] [--json]\n       priory inbox read SEQ\n       priory post SOURCE LINE\n       priory tasks [--lane <lane>] [--grade red|yellow|green] [--json]\n       priory tasks --graph [TASK] [--mermaid]\n       priory queue [DIR] [--json]\n       priory schedules [DIR] [--json]\n       priory links [DIR] [--json]\n       priory flows [DIR] --json\n       priory views [DIR] --json\n       priory runs\n       priory outbox\n       priory asks [--all] [--band <band>[,<band>...]]... [--risk <risk>[,<risk>...]]... [--json]\n       priory waits\n       priory actions\n       priory attach (TASK | --run RUN) [--read-only] [--print]\n       priory desk [--all] [--json] [--person NAME]\n       priory seen [--person NAME] [--at TIME]\n       priory wait --since REV [--timeout D]\n       priory snooze (ask | blocked | failed | prompt) ID (--for DURATION | --until TIME | --clear)\n       priory todo add TEXT [--band B] [--due TIME]\n       priory todo done ID\n       priory todo list [--json]\n       priory trace\n       priory report put TASK FILE|-\n       priory report show [TASK] [--json]\n       priory report pick PICK OPTION --expected REVISION\n       priory report comment ROW --expected REVISION TEXT\n       priory report (reply COMMENT TEXT | resolve COMMENT | reopen COMMENT)\n       priory report [DIR] --workflow <name> [--since <instant-or-duration>] [--cost] [--by lane]\n       priory report risk [DIR] [--workflow <name>] [--since <instant-or-duration>] [--json]\n       priory report attention [--since <instant-or-duration>] [--by workflow|lane|kind] [--json]\n       priory costs [DIR] [--since <instant-or-duration>] [--by workflow|agent|model|machine|repo] [--json]\n       priory grades [--since <instant-or-duration>] [--json]\n       priory move TASK [--band <band>] [--top | --before TASK | --after TASK] [--expected REVISION]\n       priory resume TASK\n       priory say TASK TEXT\n       priory pause [REASON]\n       priory unpause\n       priory cancel TASK [--expected REVISION]\n       priory revive TASK [--step STEP]\n       priory hold TASK --reason TEXT [--until TIME] [--after TASK-OR-ISSUE]... [--merge PR]... [--if-closed release|drop|ask] [--blockers] [--expected REVISION]\n       priory release TASK [--expected REVISION]\n       priory task TASK [--json]\n       priory relate TASK builds-on|fixes|takes-up|relates OTHER [--remove]\n       priory subscribe TASK RESOURCE [--expected REVISION]\n       priory unsubscribe TASK RESOURCE\n       priory tldr TASK TEXT\n       priory progress TASK JSON\n       priory search QUERY [--state STATE[,STATE...]] [--limit N]\n       priory gate\n       priory memory [(acquire | lease) [--gb N] [--timeout D] -- COMMAND...]\n       priory answer [--revision N | --expected N] [--run] [--] [DIR] [RUNNER:]ASK OPTION-OR-TEXT\n       priory give [DIR] KEY RUNNER [--force]\n       priory ask ID JSON\n       priory dismiss [--expected N] ASK\n       priory deliver ASK (--retry | --clear)\n       priory mcp --session ID\n       priory mcp --eval LOG [--describe TOOL TEXT]\n       priory eval FILE --budget USD [--case NAME] [--runs N] [--model MODEL]\n       priory hook EVENT"
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

/// The task pull requests merged since `since` whose merge commits the intake reads check runs at;
/// none when the intake runs without a state file.
fn merged(since: &str) -> Result<Vec<intake::Merge>, String> {
    if std::env::var_os("PRIORY_DB").is_none() {
        return Ok(Vec::new());
    }
    let db = state::open()?;
    state::merges(&db, since).map_err(|error| format!("merged pull requests: {error}"))
}

/// How many tasks with `key` ended, from `PRIORY_DB`; none when the intake runs without a state
/// file.
fn ended(key: &str) -> Result<u64, String> {
    if std::env::var_os("PRIORY_DB").is_none() {
        return Ok(0);
    }
    let db = state::open()?;
    state::ended(&db, key).map_err(|error| format!("tasks with key {key}: {error}"))
}

/// Records each intake source's poll in the state, when the intake has one.
fn health(outcomes: &intake::Outcomes, at: jiff::Timestamp) -> Result<(), String> {
    if std::env::var_os("PRIORY_DB").is_none() {
        return Ok(());
    }
    let mut db = state::open()?;
    let at = at.strftime("%Y-%m-%dT%H:%M:%SZ").to_string();
    for (source, error) in outcomes {
        post::polled(&mut db, source, error.as_deref(), &at)
            .map_err(|error| format!("record the {source} poll: {error}"))?;
    }
    Ok(())
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

/// Prints `priory costs [DIR] [--since <window>] [--by <split>] [--json]`, priced from the factory's
/// `prices.toml`.
fn costs(args: Vec<String>) -> ExitCode {
    let (mut since, mut by, mut json, mut dirs) = (None, "workflow".to_owned(), false, Vec::new());
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--json" => json = true,
            "--since" => match args.next() {
                Some(window) => since = Some(window),
                None => return usage(),
            },
            "--by" => match args.next() {
                Some(split) if report::BY.contains(&split.as_str()) => by = split,
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
    let printed = intake::now().and_then(|now| {
        let now = now.to_string();
        let db = state::open()?;
        let window = report::window(&db, since.as_deref(), &now)?;
        let costs = report::costs(&db, &factory.prices, &window, &by)?;
        match json {
            true => state::emit(&costs.json().to_string()),
            false => costs.print(),
        }
        Ok(())
    });
    exit("costs", printed)
}

/// Prints `priory grades [--since <window>] [--json]`: each grade against its tasks' outcomes.
fn grades(args: Vec<String>) -> ExitCode {
    let (mut since, mut json) = (None, false);
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--json" => json = true,
            "--since" => match args.next() {
                Some(window) => since = Some(window),
                None => return usage(),
            },
            _ => return usage(),
        }
    }
    let printed = intake::now().and_then(|now| {
        let db = state::open()?;
        let since = since
            .map(|since| report::start(&db, &since, &now.to_string()))
            .transpose()?;
        let grades = grades::grades(&db, since)?;
        match json {
            true => state::emit(&grades.json().to_string()),
            false => grades.print(),
        }
        Ok(())
    });
    exit("grades", printed)
}

/// Prints `priory report attention [--since <window>] [--by <split>] [--json]`.
fn report_attention(args: Vec<String>) -> ExitCode {
    let (mut since, mut by, mut json) = (None, "workflow".to_owned(), false);
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--json" => json = true,
            "--since" => match args.next() {
                Some(window) => since = Some(window),
                None => return usage(),
            },
            "--by" => match args.next() {
                Some(split) if report::attention::BY.contains(&split.as_str()) => by = split,
                _ => return usage(),
            },
            _ => return usage(),
        }
    }
    let printed = intake::now().and_then(|now| {
        let db = state::open()?;
        let window = report::window(&db, since.as_deref(), &now.to_string())?;
        let attention = report::attention::attention(&db, &window, &by)?;
        match json {
            true => state::emit(&attention.json().to_string()),
            false => attention.print(),
        }
        Ok(())
    });
    exit("report", printed)
}

/// Prints `priory report risk [DIR] [--workflow <name>] [--since <window>] [--json]`. It reads the
/// factory for each irreversible step and the human steps that approve it.
fn report_risk(args: Vec<String>) -> ExitCode {
    let (mut workflow, mut since, mut json) = (None, risk::SINCE.to_owned(), false);
    let mut dirs = Vec::new();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--json" => json = true,
            "--workflow" => match args.next() {
                Some(name) => workflow = Some(name),
                None => return usage(),
            },
            "--since" => match args.next() {
                Some(window) => since = window,
                None => return usage(),
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
    let printed = intake::now().and_then(|now| {
        let db = state::open()?;
        let start = report::start(&db, &since, &now.to_string())?;
        let risk = risk::risk(&db, &factory, workflow.as_deref(), &since, &start, now)?;
        match json {
            true => state::emit(&risk.json().to_string()),
            false => risk.print(),
        }
        Ok(())
    });
    exit("report", printed)
}

/// Prints `priory report [DIR] --workflow <name> [--since <window>] [--cost]`. Only `--cost` reads
/// the factory, for its prices.
fn report(args: Vec<String>) -> ExitCode {
    match args.first().map(String::as_str) {
        Some("put") => return report_put(&args[1..]),
        Some("show") => return report_show(&args[1..]),
        Some("pick") => return report_pick(&args[1..]),
        Some("attention") => return report_attention(args[1..].to_vec()),
        Some("risk") => return report_risk(args[1..].to_vec()),
        Some(verb @ ("comment" | "reply" | "resolve" | "reopen")) => {
            return report_comment(verb, &args[1..]);
        }
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

/// Sends `priory report comment ROW --expected REVISION TEXT`, `reply COMMENT TEXT`,
/// `resolve COMMENT` or `reopen COMMENT` to the daemon. A run's pane sets `PRIORY_SESSION`, which
/// tells the daemon an agent wrote it.
fn report_comment(verb: &str, args: &[String]) -> ExitCode {
    let (remark, on, text, expected) = match (verb, args) {
        ("comment", [row, flag, expected, text]) if flag == "--expected" => {
            let Ok(expected) = expected.parse() else {
                return usage();
            };
            (sections::Remark::Comment, row, Some(text), Some(expected))
        }
        ("reply", [comment, text]) => (sections::Remark::Reply, comment, Some(text), None),
        ("resolve", [comment]) => (sections::Remark::Resolve, comment, None, None),
        ("reopen", [comment]) => (sections::Remark::Reopen, comment, None, None),
        _ => return usage(),
    };
    let line = serde_json::to_string(&sections::Comment {
        comment: remark,
        on: on.clone(),
        text: text.cloned(),
        expected,
        session: std::env::var("PRIORY_SESSION")
            .ok()
            .filter(|id| !id.is_empty()),
    })
    .expect("a comment serializes");
    send(&format!("report {verb}"), &line)
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
    let at = at.unwrap_or_else(jiff::Timestamp::now);
    let person = person.map_or(listing::ME, String::as_str);
    let done = state::open()
        .and_then(|db| state::seen(&db, person, at).map_err(|error| error.to_string()));
    exit("seen", done.map(|text| println!("{text}")))
}

/// `priory todo add TEXT [--band B] [--due TIME]` and `priory todo done ID` go to the daemon;
/// `priory todo list [--json]` reads the state.
fn todo(args: Vec<String>) -> ExitCode {
    let change = match &args[..] {
        [verb, rest @ ..] if verb == "add" => {
            let (mut text, mut band, mut due) = (None, None, None);
            let mut rest = rest.iter();
            while let Some(arg) = rest.next() {
                let slot = match arg.as_str() {
                    "--band" => &mut band,
                    "--due" => &mut due,
                    _ if text.is_none() => {
                        text = Some(arg.clone());
                        continue;
                    }
                    _ => return usage(),
                };
                match rest.next() {
                    Some(value) if slot.is_none() => *slot = Some(value.clone()),
                    _ => return usage(),
                }
            }
            let Some(text) = text else {
                return usage();
            };
            todo::Change::Add(todo::Add { text, band, due })
        }
        [verb, id] if verb == "done" => match id.parse() {
            Ok(id) => todo::Change::Done(id),
            Err(_) => return usage(),
        },
        [verb, flags @ ..] if verb == "list" => {
            let json = match flags {
                [] => false,
                [flag] if flag == "--json" => true,
                _ => return usage(),
            };
            let listed =
                state::open().and_then(|db| todo::list(&db).map_err(|error| error.to_string()));
            return exit(
                "todo",
                listed.map(|rows| match json {
                    true => println!("{rows}"),
                    false => {
                        listing::print(&["id", "text", "band", "due", "created", "done"], &rows)
                    }
                }),
            );
        }
        _ => return usage(),
    };
    let line = serde_json::to_string(&todo::Line { todo: change }).expect("a to-do serializes");
    send("todo", &line)
}

/// Sends `priory deliver ASK (--retry | --clear)` to the daemon.
fn deliver(args: Vec<String>) -> ExitCode {
    let (ask, act) = match &args[..] {
        [ask, flag] if flag == "--retry" => (ask, deliver::Act::Retry),
        [ask, flag] if flag == "--clear" => (ask, deliver::Act::Clear),
        _ => return usage(),
    };
    let deliver = match ask_id(ask) {
        Ok(deliver) => deliver,
        Err(error) => return exit("deliver", Err(error)),
    };
    let line =
        serde_json::to_string(&deliver::Line { deliver, act }).expect("a deliver serializes");
    send("deliver", &line)
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
    if verb == "tasks" && args.first().is_some_and(|arg| arg == "--graph") {
        return graph(&args[1..]);
    }
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
    let unrouted = verb == "inbox"
        && args
            .iter()
            .position(|arg| arg == "--unrouted")
            .map(|at| args.remove(at))
            .is_some();
    // Without the factory folder, the queue knows no schedules or workflows, so up next lists none.
    let (mut schedules, mut workflows, mut now) = (Vec::new(), Vec::new(), jiff::Timestamp::now());
    if ["queue", "desk"].contains(&verb) {
        now = match intake::now() {
            Ok(now) => now,
            Err(error) => return exit(verb, Err(error)),
        };
    }
    if let ("queue", [dir]) = (verb, &args[..]) {
        if dir.starts_with("--") {
            return usage();
        }
        let Some(factory) = load(Path::new(dir)) else {
            return ExitCode::FAILURE;
        };
        schedules = listing::schedule_triggers(&factory);
        workflows = factory.workflows.into_keys().collect();
        args.clear();
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
        let name = if unrouted { "unrouted" } else { verb };
        let value = listing::revised(&db, name, |db| {
            match verb {
                "inbox" if unrouted => listing::unrouted(db, since.unwrap_or(0)),
                "inbox" => listing::inbox(db, since.unwrap_or(0)),
                "tasks" => listing::tasks(db, lane, grade),
                "asks" => listing::asks(db, &bands, &risks),
                "desk" => listing::desk(db, &home, all, person.unwrap_or(listing::ME), now),
                _ => listing::queue(db, &schedules, &workflows, now),
            }
            .map(Some)
        })
        .map_err(|error| error.to_string())?
        .expect("a listing always reads");
        if json.is_some() {
            state::emit(&value.to_string());
            return Ok(());
        }
        if unrouted {
            print_unrouted(&value[name]);
            print_outsider(&value["outsider"]);
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
                    .map(|ask| {
                        let mut ask = ask.clone();
                        if ask["merge_risk"].is_object() {
                            let lines = tools::RISK_DIMENSIONS.map(|dimension| {
                                ask["merge_risk"][dimension].as_str().unwrap_or_default()
                            });
                            ask["merge_risk"] = lines.join("; ").into();
                        }
                        ask
                    })
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
                        "merge_risk",
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
                if let Some(intake) = value["intake"].as_str() {
                    state::emit(intake);
                }
                if let Some(unrouted) = value["counts"]["unrouted"].as_u64().filter(|n| *n > 0) {
                    let events = if unrouted == 1 { "event" } else { "events" };
                    state::emit(&format!(
                        "{unrouted} {events} reached no task in the last 24 hours. See them with `priory inbox --unrouted`."
                    ));
                }
                if let Some(outsider) = value["counts"]["outsider"].as_u64().filter(|n| *n > 0) {
                    let (comments, them) = if outsider == 1 {
                        ("comment", "it")
                    } else {
                        ("comments", "them")
                    };
                    state::emit(&format!(
                        "{outsider} {comments} from outside the repository came in the last 24 hours. See {them} with `priory inbox --unrouted`."
                    ));
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
                print_since_you_looked(&value["since_you_looked"]);
            }
            _ => {
                listing::print(
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
                );
                print_asleep(&value["asleep"]);
            }
        }
        Ok(())
    });
    exit(verb, shown)
}

/// After a blank line, what happened since the person last looked, newest first, when anything did.
fn print_since_you_looked(events: &Value) {
    let Some(events) = events.as_array().filter(|events| !events.is_empty()) else {
        return;
    };
    state::emit("");
    state::emit("since you looked\ttask\twhat");
    for event in events {
        let cell = |field: &str| event[field].as_str().unwrap_or("-");
        state::emit(&format!(
            "{}\t{}\t{}",
            cell("time"),
            cell("key"),
            cell("what")
        ));
    }
}

/// After a blank line, each asleep task with what its waits wait on and since when, when one is.
fn print_asleep(asleep: &Value) {
    let Some(tasks) = asleep.as_array().filter(|tasks| !tasks.is_empty()) else {
        return;
    };
    state::emit("");
    state::emit("asleep\tkey\tsince\twaits on");
    for task in tasks {
        let waits: Vec<String> = task["waits"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|wait| {
                let text = |field: &str| wait[field].as_str().unwrap_or("-").to_owned();
                format!("{} {} since {}", text("kind"), text("on"), text("since"))
            })
            .collect();
        let cell = |field: &str| task[field].as_str().map_or("-".to_owned(), str::to_owned);
        state::emit(&format!(
            "{}\t{}\t{}\t{}",
            task["id"],
            cell("key"),
            cell("since"),
            if waits.is_empty() {
                "an ask".to_owned()
            } else {
                waits.join("; ")
            }
        ));
    }
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

/// The factory's links, as `GET /api/links` serves them: `priory links [DIR] [--json]`.
fn links(mut args: Vec<String>) -> ExitCode {
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
    let links = listing::links(&factory);
    match json {
        Some(_) => println!("{links}"),
        None => listing::print(&["pattern", "url"], &links["links"]),
    }
    ExitCode::SUCCESS
}

/// Prints what `GET /api/flows` serves: `priory flows [DIR] --json`. `priory check --explain`
/// prints the same facts as text.
fn flows(mut args: Vec<String>) -> ExitCode {
    let Some(at) = args.iter().position(|arg| arg == "--json") else {
        return usage();
    };
    args.remove(at);
    let dir = match &args[..] {
        [] => PathBuf::from("."),
        [dir] if !dir.starts_with("--") => PathBuf::from(dir),
        _ => return usage(),
    };
    let Some(factory) = load(&dir) else {
        return ExitCode::FAILURE;
    };
    println!("{}", listing::flows(&factory));
    ExitCode::SUCCESS
}

/// Prints the resolved views as `GET /api/views` serves them: `priory views [DIR] --json`.
fn views(mut args: Vec<String>) -> ExitCode {
    let Some(at) = args.iter().position(|arg| arg == "--json") else {
        return usage();
    };
    args.remove(at);
    let dir = match &args[..] {
        [] => PathBuf::from("."),
        [dir] if !dir.starts_with("--") => PathBuf::from(dir),
        _ => return usage(),
    };
    let Some(factory) = load(&dir) else {
        return ExitCode::FAILURE;
    };
    println!("{}", listing::views(&factory));
    ExitCode::SUCCESS
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
    if let Err(error) = related(request.relates()) {
        eprintln!("priory request: {error}");
        return ExitCode::FAILURE;
    }
    match request::run(&factory, &request, None) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("priory request: {error}");
            ExitCode::FAILURE
        }
    }
}

fn add(args: Vec<String>) -> ExitCode {
    let Some((dir, add)) = request::parse_add(args) else {
        return usage();
    };
    let Some(factory) = load(&dir) else {
        return ExitCode::FAILURE;
    };
    if let Err(error) = related(&add.relates) {
        eprintln!("priory add: {error}");
        return ExitCode::FAILURE;
    }
    match request::add(&factory, &add) {
        Ok(key) => {
            println!("{key}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("priory add: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Refuses `--relates` to a task the state file does not hold, before anything is published.
fn related(relates: &[request::Relation]) -> Result<(), String> {
    if relates.is_empty() {
        return Ok(());
    }
    let db = state::open()?;
    match relate::missing(&db, relates).map_err(|error| error.to_string())? {
        Some(missing) => Err(missing),
        None => Ok(()),
    }
}

/// `priory batch add [DIR] ISSUE...` publishes a batch's planning request.
fn batch(args: Vec<String>) -> ExitCode {
    match args.split_first() {
        Some((verb, rest)) if verb == "add" => {
            let Some((dir, batch)) = request::parse_batch(rest.to_vec()) else {
                return usage();
            };
            let Some(factory) = load(&dir) else {
                return ExitCode::FAILURE;
            };
            let added = request::add_batch(&factory, &batch).map_err(|error| error.to_string());
            exit("batch add", added.map(|key| println!("{key}")))
        }
        Some((verb, rest)) if verb == "show" => {
            let (json, which): (Vec<&String>, Vec<&String>) =
                rest.iter().partition(|arg| *arg == "--json");
            let which = match which[..] {
                [] => None,
                [which] if !which.starts_with("--") => Some(which.as_str()),
                _ => return usage(),
            };
            let shown = state::open()
                .and_then(|db| priory_core::batch::show(&db, which))
                .map(|shown| match (json.is_empty(), which) {
                    (false, _) => state::emit(&shown.to_string()),
                    (true, None) => listing::print(
                        &["id", "key", "state", "title", "repo", "band", "went"],
                        &shown["batches"],
                    ),
                    (true, Some(_)) => {
                        listing::print(&["id", "key", "state", "title", "repo"], &json!([shown]));
                        listing::print(
                            &["entry", "position", "issue", "task", "state", "stage"],
                            &shown["entries"],
                        );
                        listing::print(&["issue", "why", "closed"], &shown["closes"]);
                    }
                });
            exit("batch show", shown)
        }
        Some((verb, rest)) if verb == "edit" => {
            let Some((Some(expected), args)) = expected(rest.to_vec()) else {
                return usage();
            };
            let Some((which, change)) = batch_edit(&args) else {
                return usage();
            };
            let edited = state::open()
                .and_then(|mut db| priory_core::batch::edit(&mut db, which, expected, &change));
            exit("batch edit", edited.map(|edited| println!("{edited}")))
        }
        _ => usage(),
    }
}

/// `BATCH (--drop ENTRY [--with-successors] | --move ENTRY (--top | --before OTHER | --after
/// OTHER) | --moment ENTRY OTHER MOMENT)`, the rest of `priory batch edit` after `--expected`.
fn batch_edit(args: &[String]) -> Option<(&str, priory_core::batch::Edit)> {
    use priory_core::batch::{Edit, Place};
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    let (which, rest) = words.split_first()?;
    let change = match *rest {
        ["--drop", entry] | ["--drop", entry, "--with-successors"] => Edit::Drop {
            entry: entry.to_owned(),
            with_successors: rest.len() == 3,
        },
        ["--move", entry, "--top"] => Edit::Move {
            entry: entry.to_owned(),
            place: Place::Top,
        },
        ["--move", entry, "--before", other] => Edit::Move {
            entry: entry.to_owned(),
            place: Place::Before(other.to_owned()),
        },
        ["--move", entry, "--after", other] => Edit::Move {
            entry: entry.to_owned(),
            place: Place::After(other.to_owned()),
        },
        ["--moment", entry, other, moment] => Edit::Moment {
            entry: entry.to_owned(),
            other: other.to_owned(),
            moment: moment.to_owned(),
        },
        _ => return None,
    };
    Some((which, change))
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

/// `priory eval FILE --budget USD`: runs a prompt evaluation against a real model, prints its table,
/// and rewrites the doc's table when every case ran within the budget.
fn prompt_eval(args: Vec<String>) -> ExitCode {
    let Some((file, flags)) = args.split_first() else {
        return usage();
    };
    let (mut case, mut runs, mut model, mut budget) = (None, 5, "haiku".to_owned(), None);
    for pair in flags.chunks(2) {
        match pair {
            [flag, value] if flag == "--case" => case = Some(value.clone()),
            [flag, value] if flag == "--model" => model = value.clone(),
            [flag, value] if flag == "--runs" => match value.parse() {
                Ok(value) if value > 0 => runs = value,
                _ => return usage(),
            },
            [flag, value] if flag == "--budget" => match value.parse::<f64>() {
                Ok(value) if value > 0.0 && value.is_finite() => budget = Some(value),
                _ => return usage(),
            },
            _ => return usage(),
        }
    }
    let Some(budget) = budget else {
        eprintln!("priory eval: give --budget USD, the most the evaluation may spend");
        return ExitCode::FAILURE;
    };
    let path = Path::new(file);
    let done = eval::load(path).and_then(|loaded| {
        let scratch = std::env::temp_dir().join(format!("priory-eval-{}", std::process::id()));
        let options = eval::Options {
            runs,
            budget,
            case: case.as_deref(),
        };
        let report = eval::run(
            &loaded,
            &scratch,
            &options,
            &mut eval::Claude { model: &model },
        );
        let _ = std::fs::remove_dir_all(&scratch);
        let report = report?;
        let table = eval::table(&report, &model, runs);
        print!("{table}");
        match &loaded.doc {
            Some(doc) if case.is_none() && report.complete => {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                eval::rewrite(doc, &name, &table)
            }
            _ => Ok(()),
        }
    });
    exit("eval", done)
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
        match held(program, rest, grant) {
            Ok(status) => status,
            Err(error) => {
                let _ = gate(&Gate::Release { grant });
                return failed(error);
            }
        }
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

/// Runs a command that holds `grant` from before it starts: a shell waits for a go-ahead on a pipe,
/// the daemon takes its pid, and only then it execs the command. A holder that dies first closes
/// the pipe, so the command never runs without its grant.
fn held(
    program: &OsString,
    rest: &[OsString],
    grant: i64,
) -> Result<std::io::Result<std::process::ExitStatus>, String> {
    use std::io::Write;
    use std::os::fd::AsRawFd;
    let (reader, mut writer) = std::io::pipe().map_err(|error| error.to_string())?;
    // The shell opens the pipe's read end through /proc by this pid, not $PPID, which a dead holder turns into init, and which keeps the command's stdin as it was.
    let wait = format!(
        "read -r _ <\"/proc/{}/fd/{}\" && exec \"$@\"",
        std::process::id(),
        reader.as_raw_fd()
    );
    let mut child = match Command::new("sh")
        .args(["-c", &wait, "sh"])
        .arg(program)
        .args(rest)
        .spawn()
    {
        Ok(child) => child,
        Err(error) => return Ok(Err(error)),
    };
    let taken = gate(&Gate::Run {
        grant,
        pid: child.id(),
    });
    if let Err(error) =
        taken.and_then(|_| writer.write_all(b"\n").map_err(|error| error.to_string()))
    {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
    }
    drop(writer);
    Ok(child.wait())
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
    runtime::call(&runtime::home(), &line, runtime::REPLY).and_then(|reply| {
        serde_json::from_str::<Result<String, String>>(&reply)
            .map_err(|error| format!("unreadable reply {reply:?}: {error}"))?
    })
}

/// `priory gate`, from a run's pane: runs the factory's gate in the run's task worktree at its head,
/// records the run with the daemon, and exits with the gate's code, or 1 when a signal killed it.
fn project_gate() -> ExitCode {
    let Some(session) = std::env::var("PRIORY_SESSION")
        .ok()
        .filter(|id| !id.is_empty())
    else {
        eprintln!("priory gate: PRIORY_SESSION is unset; run it from a run's pane");
        return ExitCode::FAILURE;
    };
    let call = |line: &proof::Line| {
        let line = serde_json::to_string(line).expect("a gate line serializes");
        runtime::call(&runtime::home(), &line, runtime::REPLY).and_then(|reply| {
            serde_json::from_str::<Result<String, String>>(&reply)
                .map_err(|error| format!("unreadable reply {reply:?}: {error}"))?
        })
    };
    let started = call(&proof::Line::Start {
        session: session.clone(),
    })
    .and_then(|reply| {
        serde_json::from_str::<proof::Started>(&reply)
            .map_err(|error| format!("unreadable start {reply:?}: {error}"))
    });
    let started = match started {
        Ok(started) => started,
        Err(error) => {
            eprintln!("priory gate: {error}");
            return ExitCode::FAILURE;
        }
    };
    // The gate tests what is checked out, so a record for the branch head needs the two to match.
    let checked_out = || {
        Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&started.worktree)
            .output()
            .ok()
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
            .unwrap_or_default()
    };
    let away = |at: String| {
        eprintln!(
            "priory gate: the worktree is at {at}, not at the head of its branch, {}: check out the branch, then run it again",
            started.commit
        );
        ExitCode::FAILURE
    };
    let at = checked_out();
    if at != started.commit {
        return away(at);
    }
    let clock = Instant::now();
    let status = Command::new("sh")
        .args(["-c", &started.command])
        .current_dir(&started.worktree)
        .status();
    let exit = match status {
        // shortcut: a gate killed by a signal records -1, the way a shell would not; fine for a proof.
        Ok(status) => status.code().unwrap_or(-1),
        Err(error) => {
            eprintln!("priory gate: cannot run sh: {error}");
            return ExitCode::FAILURE;
        }
    };
    let at = checked_out();
    if at != started.commit {
        return away(at);
    }
    let recorded = call(&proof::Line::Ran {
        session,
        commit: started.commit,
        exit,
        duration_ms: clock.elapsed().as_millis() as u64,
    });
    match recorded {
        Ok(text) => eprintln!("priory gate: {text}"),
        Err(error) => {
            eprintln!("priory gate: the gate ran, and the daemon did not record it: {error}");
            return ExitCode::FAILURE;
        }
    }
    ExitCode::from(u8::try_from(exit).unwrap_or(1))
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
    let (dir, ask, text) = match &rest[..] {
        [ask, text] => (".", ask, text),
        [dir, ask, text] => (dir.as_str(), ask, text),
        _ => return usage(),
    };
    // This machine's ask by its number or name, else another runner's, which goes through the log
    // and applies when that runner reads it.
    let local = ask_id(ask);
    let remote = local
        .is_err()
        .then(|| ask.split_once(':'))
        .flatten()
        .and_then(|(runner, id)| Some((runner, id.parse::<i64>().ok()?)));
    let ask = match (remote, local) {
        (Some((_, id)), _) | (None, Ok(id)) => id,
        (None, Err(error)) => return exit("answer", Err(error)),
    };
    let answer = tools::Answer {
        ask,
        revision,
        text: text.clone(),
        run: run.is_some(),
        by: "cli".to_owned(),
    };
    if let Some((runner, _)) = remote {
        let Some(factory) = load(Path::new(dir)) else {
            return ExitCode::FAILURE;
        };
        return reply("answer", runners::answer(&factory, runner, &answer));
    }
    let line = serde_json::to_string(&answer).expect("an answer serializes");
    send("answer", &line)
}

fn reply(verb: &str, done: Result<String, String>) -> ExitCode {
    match done {
        Ok(text) => {
            println!("{text}");
            ExitCode::SUCCESS
        }
        Err(error) => exit(verb, Err(error)),
    }
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

/// Takes `--expected N` out of a task verb's arguments, which carry no free text.
fn expected(mut args: Vec<String>) -> Option<(Option<i64>, Vec<String>)> {
    let Some(at) = args.iter().position(|arg| arg == "--expected") else {
        return Some((None, args));
    };
    let revision = args.get(at + 1)?.parse().ok()?;
    args.drain(at..at + 2);
    Some((Some(revision), args))
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
/// Prints `priory inbox --unrouted`: each group of a resource a task owned, with that task, its
/// events and the command that routes the next one, then one count per source for the rest.
fn print_unrouted(unrouted: &serde_json::Value) {
    let events = |count: u64, what: &str| {
        format!("{count} {what}event{}", if count == 1 { "" } else { "s" })
    };
    for group in unrouted["owned"].as_array().into_iter().flatten() {
        let task = &group["task"];
        println!(
            "{}: {}, task {} {}",
            group["resource"].as_str().unwrap_or_default(),
            events(group["events"].as_array().map_or(0, Vec::len) as u64, ""),
            task["id"],
            task["state"].as_str().unwrap_or_default()
        );
        for event in group["events"].as_array().into_iter().flatten() {
            println!(
                "  {}  {}  {}  {}",
                event["seq"],
                event["type"].as_str().unwrap_or_default(),
                event["subject"].as_str().unwrap_or("-"),
                event["line"].as_str().unwrap_or("")
            );
        }
        println!(
            "  {}",
            group["follow_up"]
                .as_str()
                .unwrap_or("no command can key a new task by it: start one from its thread")
        );
    }
    let other: Vec<String> = unrouted["other"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|source| {
            let source_name = format!("{} ", source["source"].as_str().unwrap_or_default());
            events(source["events"].as_u64().unwrap_or(0), &source_name)
        })
        .collect();
    if !other.is_empty() {
        println!("No task ever owned: {}.", other.join(", "));
    }
}

/// `priory task ID`: the task's key, workflow, step and state, then its Inbox section.
fn task_text(task: &str) -> ExitCode {
    let Ok(id) = task.parse() else {
        return usage();
    };
    let shown = state::open().and_then(|db| {
        let task = listing::revised(&db, "task", |db| listing::task(db, id))
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("no task {id}"))?;
        let text = |value: &serde_json::Value| value.as_str().unwrap_or("-").to_owned();
        println!("task {id}  {}", text(&task["key"]));
        println!(
            "workflow {}, step {}, {}",
            text(&task["workflow"]),
            text(&task["step"]),
            text(&task["state"])
        );
        println!("\nInbox");
        for event in task["inbox"].as_array().into_iter().flatten() {
            println!(
                "  {}  {}  {}  {}  {}",
                event["seq"],
                text(&event["time"]),
                text(&event["type"]),
                text(&event["subject"]),
                handled(event, id)
            );
        }
        Ok(())
    });
    exit("task", shown)
}

/// What one event of a task's inbox did: its outcome, what became of its prompt, the wait it fired
/// and the task routing gave it to when that is another task.
fn handled(event: &serde_json::Value, task: i64) -> String {
    let mut text = event["outcome"].as_str().unwrap_or("-").to_owned();
    let prompt = match event["prompt"].as_str() {
        Some("confirmed") => "delivered as prompt",
        Some("queued") => "still queued",
        Some("attempted") => "being typed",
        Some("uncertain") => "typed, unconfirmed",
        Some("withdrawn") => "prompt withdrawn",
        _ if text == "kept" && event["task"].as_i64() == Some(task) => "for its next run",
        _ => "",
    };
    if !prompt.is_empty() {
        text = format!("{text}, {prompt}");
    }
    if let Some(wait) = event["wait"].as_i64() {
        text = format!("{text}, fired wait {wait}");
    }
    if let Some(other) = event["task"].as_i64().filter(|other| *other != task) {
        text = format!("{text} (routed to task {other})");
    }
    text
}

/// Prints the comments and reviews from outside the repository as a group of their own: its count
/// and one line per event, with who wrote it. No command routes them: they reach no task.
fn print_outsider(events: &serde_json::Value) {
    let Some(events) = events.as_array().filter(|events| !events.is_empty()) else {
        return;
    };
    let plural = if events.len() == 1 { "" } else { "s" };
    println!(
        "from outside the repository: {} event{plural}",
        events.len()
    );
    for event in events {
        println!(
            "  {}  {}  {}  {}",
            event["seq"],
            event["type"].as_str().unwrap_or_default(),
            event["subject"].as_str().unwrap_or("-"),
            event["actor"].as_str().unwrap_or("-")
        );
    }
}

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

/// `priory subscribe` or `priory unsubscribe`: starts or ends `task`'s subscription to `resource`.
/// `expected` refuses a subscribe once the state moved past that revision.
fn subscription(verb: &str, task: &str, resource: &str, expected: Option<i64>) -> ExitCode {
    let Ok(task) = task.parse() else {
        return usage();
    };
    let Some(resource) = subscribe::resource(resource) else {
        eprintln!(
            "priory {verb}: `{resource}` is not a resource: name github:owner/name#7, linear:ENG-7 or slack:C0123:1763500000.000100"
        );
        return ExitCode::from(2);
    };
    let done = state::open().and_then(|mut db| {
        let done = match verb {
            "subscribe" => subscribe::subscribe(&mut db, task, &resource, expected),
            _ => subscribe::unsubscribe(&mut db, task, &resource),
        };
        done.map_err(|error| error.to_string())?
            .map(|text| println!("{text}"))
    });
    exit(verb, done)
}

/// `priory tasks --graph [TASK] [--mermaid]`.
fn graph(args: &[String]) -> ExitCode {
    let (mermaid, rest): (Vec<_>, Vec<_>) = args.iter().partition(|arg| *arg == "--mermaid");
    let root = match &rest[..] {
        [] => None,
        [task] => match task.parse() {
            Ok(task) => Some(task),
            Err(_) => return usage(),
        },
        _ => return usage(),
    };
    let shown = state::open().and_then(|db| {
        relate::graph(&db, root, !mermaid.is_empty())
            .map_err(|error| error.to_string())?
            .map(|graph| print!("{graph}"))
    });
    exit("tasks", shown)
}

/// `priory relate TASK KIND OTHER [--remove]`: records or removes how two tasks relate.
fn relate(mut args: Vec<String>) -> ExitCode {
    let remove = args.iter().position(|arg| arg == "--remove");
    if let Some(at) = remove {
        args.remove(at);
    }
    let [task, kind, other] = &args[..] else {
        return usage();
    };
    let (Ok(task), Ok(other)) = (task.parse(), other.parse()) else {
        return usage();
    };
    let by = std::env::var("USER").unwrap_or_else(|_| "person".to_owned());
    let done = state::open().and_then(|mut db| {
        let done = match remove {
            Some(_) => relate::unrelate(&mut db, task, kind, other),
            None => relate::relate(&mut db, task, kind, other, &by),
        };
        done.map_err(|error| error.to_string())?
            .map(|text| println!("{text}"))
    });
    exit("relate", done)
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
    let Some((expected, args)) = expected(args) else {
        return usage();
    };
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
    let moved = state::open()
        .and_then(|mut db| state::move_task(&mut db, task, band.as_deref(), place, expected));
    exit("move", moved)
}

/// Sends `priory hold TASK --reason TEXT [CONDITION]` to the daemon, which applies it.
fn hold(args: Vec<String>) -> ExitCode {
    let mut args = args.into_iter();
    let Some(task) = args.next() else {
        return usage();
    };
    let mut hold = factory::Hold::default();
    let (mut reason, mut task) = (None, vec![task]);
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
            "--expected" if task.len() == 1 => task.extend([flag, value]),
            _ => return usage(),
        }
    }
    let Some(reason) = reason else {
        return usage();
    };
    hold.reason = reason;
    act("hold", task, Some(hold))
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
    let (expected, mut args) = match act {
        "hold" | "release" | "cancel" => match expected(args) {
            Some(split) => split,
            None => return usage(),
        },
        _ => (None, args),
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
        expected,
    })
    .expect("an action serializes");
    send(act, &line)
}

/// Sends `line` to the daemon and prints its reply.
fn send(verb: &str, line: &str) -> ExitCode {
    let reply = runtime::call(&runtime::home(), line, runtime::REPLY).and_then(|reply| {
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
    if let (Some(name), Some(guide)) = (
        factory
            .settings
            .as_ref()
            .and_then(|s| s.value.writing.as_ref()),
        &factory.writing,
    ) && guide.len() > factory::WRITING_LIMIT
    {
        eprintln!(
            "warning: {name} is {} bytes: every agent's prompt carries it, so keep it under {}",
            guide.len(),
            factory::WRITING_LIMIT
        );
    }
    warn_of_stale_herdr_layout(&factory, &dir);
    if explain {
        print_explanation(&factory, json);
        return ExitCode::SUCCESS;
    }
    let skills = match factory.skills.len() {
        0 => String::new(),
        n => format!(", {}", count(n, "skill")),
    };
    let scorers = match factory.scorers.len() {
        0 => String::new(),
        n => format!(", {}", count(n, "scorer")),
    };
    println!(
        "ok: {}, {}, {}, {}{skills}{scorers}",
        count(factory.agents.len(), "agent"),
        count(factory.triggers.len(), "trigger"),
        count(factory.workflows.len(), "workflow"),
        count(factory.runners.len(), "runner"),
    );
    ExitCode::SUCCESS
}

/// Warns when the Herdr layout `adapters/herdr/install` wrote on this machine was built from other
/// view files: its root id ends in 12 hex digits of the SHA-256 of what `priory views --json`
/// prints, and an id without them predates the hash. Another machine's layout is not checked.
fn warn_of_stale_herdr_layout(factory: &factory::Factory, dir: &Path) {
    let Some(app) = std::env::var_os("HERDR_APP_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| Path::new(&home).join(".config/herdr-app"))
        })
    else {
        return;
    };
    let path = app.join("layouts/priory.json");
    let Some(id) = std::fs::read_to_string(&path)
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|layout| layout["id"].as_str().map(str::to_owned))
    else {
        return;
    };
    let hash: String = Sha256::digest(listing::views(factory).to_string().as_bytes())[..6]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let current = id == format!("priory-generated-{hash}");
    let ours = id == "priory-generated"
        || id
            .strip_prefix("priory-generated-")
            .is_some_and(|rest| rest.len() == 12 && rest.bytes().all(|b| b.is_ascii_hexdigit()));
    if ours && !current {
        eprintln!(
            "warning: the Herdr layout {} was built from other view files: run `priory views {} --json | adapters/herdr/install` again, with --copy if you installed copies",
            path.display(),
            dir.display()
        );
    }
}

/// Prints each workflow's irreversible steps with their gates, its paths to one with no human edge,
/// the guarded calls each agent can make and the outcomes the daemon verifies, as four tables or
/// as JSON.
fn print_explanation(factory: &factory::Factory, json: bool) {
    if json {
        let mut flows = listing::flows(factory);
        if let Some(flows) = flows.as_object_mut() {
            flows.remove("workflows");
        }
        state::emit(&flows.to_string());
        return;
    }
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
    let tools = explain::tools(factory);
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
    state::emit("");
    state::emit("agent\tguard\tworkflow\twhy");
    for reach in &tools {
        state::emit(&format!(
            "{}\t{}\t{}\t{}",
            cell(&reach.agent),
            cell(&reach.guard),
            cell(&reach.workflow),
            cell(&reach.why)
        ));
    }
    state::emit("");
    state::emit("workflow\tstep\toutcome\tproof");
    for (name, workflow) in &factory.workflows {
        for (step, outcome, proof) in explain::proofs(&workflow.value) {
            state::emit(&format!(
                "{}\t{}\t{}\t{}",
                cell(name),
                cell(step),
                cell(outcome),
                proof.name()
            ));
        }
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

/// How long the tool guard waits for the daemon before it denies the call.
const GUARD_WAIT: Duration = Duration::from_secs(5);

/// How long the whole tool guard hook may take, well under Claude's 30-second hook timeout.
const GUARD_DEADLINE: Duration = Duration::from_secs(8);

/// The `PreToolUse` hook's reason to deny the call, if any: none for a call no guard matches, and
/// the daemon's answer for one that matches. Anything but the daemon's yes denies, since a hook
/// that fails lets the call through.
fn guard() -> Option<String> {
    let deny = |reason: &str| Some(reason.to_owned());
    let input: Value = match serde_json::from_reader(std::io::stdin()) {
        Ok(input) => input,
        Err(error) => {
            return deny(&format!(
                "Priory's tool guard could not read this call ({error}), so it denied it."
            ));
        }
    };
    let guards: priory_factory::guard::Guards = match std::env::var("PRIORY_GUARD") {
        Ok(json) if !json.is_empty() => match serde_json::from_str(&json) {
            Ok(guards) => guards,
            Err(error) => {
                return deny(&format!(
                    "Priory's tool guard could not read PRIORY_GUARD ({error}), so it denied this call."
                ));
            }
        },
        _ => Default::default(),
    };
    let tool = input["tool_name"].as_str().unwrap_or_default();
    let command = match &input["tool_input"]["command"] {
        Value::String(command) => Some(command.clone()),
        Value::Array(words) => Some(
            words
                .iter()
                .map(|word| {
                    format!(
                        "'{}'",
                        word.as_str().unwrap_or_default().replace('\'', r"'\''")
                    )
                })
                .collect::<Vec<_>>()
                .join(" "),
        ),
        _ => None,
    };
    let cwd = input["cwd"].as_str().map(PathBuf::from);
    let git = |dir: Option<&str>, args: &[&str]| {
        let mut git = Command::new("git");
        if let Some(cwd) = &cwd {
            git.current_dir(cwd);
        }
        if let Some(dir) = dir {
            git.args(["-C", dir]);
        }
        let output = git
            .args(args)
            .output()
            .ok()
            .filter(|output| output.status.success())?;
        Some(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    };
    let hits = guards.hits(tool, command.as_deref(), &git);
    let moves_head = command
        .as_deref()
        .is_some_and(priory_factory::guard::moves_head);
    let session = std::env::var("PRIORY_SESSION")
        .ok()
        .filter(|id| !id.is_empty())
        .or_else(|| input["session_id"].as_str().map(str::to_owned))
        .unwrap_or_default();
    // Each guard the call hits needs its own yes: one allowed command must not carry another.
    for hit in hits {
        let line = serde_json::json!({ "guard": {
            "session": session, "guard": hit.guard, "call": hit.call, "tool": tool,
            "moves_head": moves_head,
        } });
        let reply =
            runtime::call(&runtime::home(), &line.to_string(), GUARD_WAIT).and_then(|reply| {
                serde_json::from_str::<Result<String, String>>(&reply)
                    .map_err(|error| format!("unreadable reply {reply:?}: {error}"))
            });
        match reply {
            Ok(Ok(_)) => {}
            Ok(Err(reason)) => return deny(&reason),
            Err(error) => {
                return deny(&format!(
                    "Priory denied `{}`: it is irreversible (guard `{}`) and the daemon did not confirm an approval within {}s ({error}). Call the ask tool of the priory MCP server if a person must do or approve it.",
                    hit.call,
                    hit.guard,
                    GUARD_WAIT.as_secs()
                ));
            }
        }
    }
    report_connector_use(tool);
    None
}

/// Tells the daemon a run used a connector with an open sign-in to-do, so it can clear the item.
/// Best effort after the guards allowed the call: a daemon that does not answer holds it for 1s at most.
fn report_connector_use(tool: &str) {
    let Some(name) = tool.strip_prefix("mcp__") else {
        return;
    };
    let Some((server, short)) = name.rsplit_once("__") else {
        return;
    };
    let watched = std::env::var("PRIORY_CONNECTORS").unwrap_or_default();
    if priory_factory::guard::SIGN_IN_TOOLS.contains(&short)
        || !watched.split(',').any(|name| name == server)
    {
        return;
    }
    let line = serde_json::json!({ "connector_used": server });
    let _ = runtime::call(&runtime::home(), &line.to_string(), Duration::from_secs(1));
}

/// Reports a harness hook to the daemon: `event`, with the session and prompt from the hook's JSON
/// on stdin. A run's pane sets `PRIORY_SESSION`; a stdin session apart from it is the harness's own.
fn hook(event: &str) -> ExitCode {
    if event == "PreToolUse" {
        // A hook that fails or that Claude kills lets the call through, so a guard that panics or
        // hangs anywhere, in git, the connect or the reply, denies before Claude's timeout.
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || sender.send(guard()));
        let denied = receiver.recv_timeout(GUARD_DEADLINE).unwrap_or_else(|_| {
            Some(format!(
                "Priory's tool guard failed or did not finish within {}s on this call, so it denied it.",
                GUARD_DEADLINE.as_secs()
            ))
        });
        if let Some(reason) = denied {
            let output = serde_json::json!({ "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "deny",
                "permissionDecisionReason": reason,
            } });
            println!("{output}");
        }
        return ExitCode::SUCCESS;
    }
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
