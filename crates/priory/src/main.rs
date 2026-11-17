#[expect(
    dead_code,
    reason = "check validates fields that only the daemon will read"
)]
mod factory;
mod intake;
mod rules;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    match args.next().as_deref().and_then(|verb| verb.to_str()) {
        Some("check") => check(
            args.next()
                .map_or_else(|| PathBuf::from("."), PathBuf::from),
        ),
        Some("intake") => {
            let (once, dirs): (Vec<_>, Vec<_>) = args.partition(|arg| arg == "--once");
            match dirs.as_slice() {
                [] => intake(PathBuf::from("."), !once.is_empty()),
                [dir] => intake(PathBuf::from(dir), !once.is_empty()),
                _ => usage(),
            }
        }
        _ => usage(),
    }
}

fn usage() -> ExitCode {
    eprintln!("usage: priory check [DIR]\n       priory intake [DIR] [--once]");
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

fn intake(dir: PathBuf, once: bool) -> ExitCode {
    let Some(factory) = load(&dir) else {
        return ExitCode::FAILURE;
    };
    match intake::run(&factory, once) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("priory intake: {error}");
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
