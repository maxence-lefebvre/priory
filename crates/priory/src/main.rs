use priory_core::{daemon, state, tools};
use priory_factory::{factory, rules};
use priory_intake::{intake, request};
use priory_runtime::{mcp, runtime};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    match args.next().as_deref().and_then(|verb| verb.to_str()) {
        Some("check") => check(
            args.next()
                .map_or_else(|| PathBuf::from("."), PathBuf::from),
        ),
        Some(verb @ ("intake" | "daemon")) => {
            let (once, dirs): (Vec<_>, Vec<_>) = args.partition(|arg| arg == "--once");
            let dir = match dirs.as_slice() {
                [] => PathBuf::from("."),
                [dir] => PathBuf::from(dir),
                _ => return usage(),
            };
            let run = if verb == "intake" {
                intake::run
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
        Some("tasks") => list(
            "id\tkey\tworkflow\tstep\tstate\tband\tlane\treason",
            "SELECT id, key, workflow, step, state, band, lane, reason FROM tasks ORDER BY id",
        ),
        Some("runs") => list(
            "id\ttask\tstep\ttmux\tsession\tstarted\tended\tend",
            "SELECT id, task, step, tmux, session, started, ended, ended_as FROM runs ORDER BY id",
        ),
        Some("outbox") => list(
            "id\ttask\trun\tstate\ttext",
            "SELECT id, task, run, state, replace(text, char(10), '\\n') FROM outbox ORDER BY id",
        ),
        Some("asks") => list(
            "id\ttask\trevision\tquestion\toptions",
            "SELECT id, task, revision, replace(question, char(10), '\\n'), options FROM asks
             WHERE answer IS NULL ORDER BY id",
        ),
        Some("waits") => list(
            "id\ttask\trun\tkind\ton\tcreated",
            "SELECT id, task, run, kind, coalesce(resource, until, 'task ' || target), created
             FROM waits WHERE fired IS NULL ORDER BY id",
        ),
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
        "usage: priory check [DIR]\n       priory intake [DIR] [--once]\n       priory daemon [DIR] [--once]\n       priory request [DIR] --workflow <name> --key <key> [--band <band>] [--lane <lane>] [--repo <owner/name>] <text>\n       priory inbox\n       priory tasks\n       priory runs\n       priory outbox\n       priory asks\n       priory waits\n       priory answer ASK OPTION-OR-TEXT --revision N\n       priory mcp --session ID\n       priory hook EVENT"
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

fn list(header: &str, query: &str) -> ExitCode {
    let printed = state::open()
        .and_then(|db| state::print(&db, header, query).map_err(|error| error.to_string()));
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
    let reply = runtime::call(&runtime::home(), &line).and_then(|reply| {
        serde_json::from_str::<Result<String, String>>(&reply)
            .map_err(|error| format!("unreadable reply {reply:?}: {error}"))?
    });
    match reply {
        Ok(text) => {
            println!("{text}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("priory answer: {error}");
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

fn count(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

/// Reports a Claude Code hook to the daemon: `event`, with the session and prompt from the hook's
/// JSON on stdin.
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
    let signal = state::Signal {
        event: event.to_owned(),
        session: session.to_owned(),
        prompt: input["prompt"].as_str().map(str::to_owned),
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
