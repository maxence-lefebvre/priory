//! The command an agent's ask carries, which a person's answer runs at most once (R6.8).

use priory_factory::factory::Factory;
use serde_json::Value;
use std::path::PathBuf;

/// A command as the `ask` tool stores it: its target resolved to a folder when it was asked, so a
/// later factory change never moves it.
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct Command {
    pub argv: Vec<String>,
    pub target: String,
    pub dir: PathBuf,
    pub runs: Vec<String>,
}

/// The `command` field as an agent sends it.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Asked {
    argv: Vec<String>,
    target: Option<String>,
    runs: Option<Vec<String>>,
}

pub(crate) const WORKTREE: &str = "worktree";

/// The command of an ask with `options` and `recommendation`, from a task with `worktree`, or why
/// the tool refuses it.
pub(crate) fn parse(
    value: &Value,
    options: &[&str],
    recommendation: &str,
    worktree: Option<&str>,
    factory: &Factory,
) -> Result<Command, String> {
    let asked: Asked = serde_json::from_value(value.clone()).map_err(|error| {
        format!("command must be an object with argv, and optionally target and runs: {error}")
    })?;
    if asked.argv.first().is_none_or(String::is_empty) {
        return Err("command argv must start with a program".to_owned());
    }
    let target = asked.target.unwrap_or_else(|| WORKTREE.to_owned());
    let dir = match (target.as_str(), worktree) {
        (WORKTREE, Some(worktree)) => PathBuf::from(worktree),
        (WORKTREE, None) => return Err("command target worktree: this task has no worktree".into()),
        (name, _) => factory.target(name).ok_or_else(|| {
            format!(
                "command target {name:?} is not allowed: use {WORKTREE} or a name in [asks.targets]: {}",
                factory.targets().join(", ")
            )
        })?,
    };
    let runs = match asked.runs {
        Some(runs) => runs,
        None if options.contains(&recommendation) => vec![recommendation.to_owned()],
        None => {
            return Err(format!(
                "command needs runs: the recommendation {recommendation:?} is not one of the options"
            ));
        }
    };
    if let Some(run) = runs.iter().find(|run| !options.contains(&run.as_str())) {
        return Err(format!(
            "command runs {run:?}, which is not one of the options"
        ));
    }
    let dir = std::path::absolute(dir).map_err(|error| format!("command target: {error}"))?;
    Ok(Command {
        argv: asked.argv,
        target,
        dir,
        runs,
    })
}

impl Command {
    /// The argv as a JSON array, which shows every word, space and quote as it is.
    pub fn shown(&self) -> String {
        serde_json::to_string(&self.argv).expect("argv serializes")
    }

    /// The program to exec: a path with a slash resolved against the target folder, a bare name
    /// left for `PATH`.
    pub fn program(&self) -> String {
        let program = &self.argv[0];
        match program.contains('/') {
            true => self.dir.join(program).to_string_lossy().into_owned(),
            false => program.clone(),
        }
    }
}
