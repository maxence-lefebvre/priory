mod daemon;
#[expect(
    dead_code,
    reason = "check validates fields that only the daemon will read"
)]
mod factory;
mod intake;
mod route;
mod rules;
mod state;

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
        Some("inbox") => list(
            "seq\ttype\tsubject\ttask\toutcome",
            "SELECT seq, type, subject, task, outcome FROM inbox ORDER BY seq",
        ),
        Some("tasks") => list(
            "id\tkey\tworkflow\tstep\tstate\tband\tlane",
            "SELECT id, key, workflow, step, state, band, lane FROM tasks ORDER BY id",
        ),
        _ => usage(),
    }
}

fn usage() -> ExitCode {
    eprintln!(
        "usage: priory check [DIR]\n       priory intake [DIR] [--once]\n       priory daemon [DIR] [--once]\n       priory inbox\n       priory tasks"
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
