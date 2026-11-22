//! `priory bench capture`: turns a past task into a benchmark case, the folder a replay plays
//! again. It reads the state and the harness transcripts of the task's runs, and never GitHub.

use jiff::Timestamp;
use priory_factory::case::{self, Answer, Case, GhAnswer, Message, Reference};
use priory_factory::factory::{Effect, Factory, Harness};
use priory_factory::guard;
use priory_runtime::transcript;
use rusqlite::{Connection, OptionalExtension};
use serde_json::Value;
use std::collections::BTreeMap;
use std::io;

/// A captured case: its files by their path in the folder, and what a person should know of it.
pub struct Captured {
    pub files: Vec<(String, String)>,
    pub gh: usize,
    pub answers: usize,
    pub gaps: usize,
}

/// One run of the task before its first irreversible step.
struct Run {
    id: i64,
    step: String,
    agent: Option<String>,
    transcript: Option<String>,
    /// Where the run's turns start in its transcript: where the previous run of the same
    /// transcript stopped reading.
    from: u64,
    /// Where the run stopped reading its transcript.
    to: u64,
}

/// Captures `task` into a case. `read` reads a transcript file; a transcript that is gone or
/// unreadable leaves a gap rather than failing the capture.
pub fn capture(
    db: &Connection,
    factory: &Factory,
    task: i64,
    read: &dyn Fn(&str) -> io::Result<Vec<u8>>,
) -> Result<Captured, String> {
    let error = |error: rusqlite::Error| error.to_string();
    type Row = (
        String,
        String,
        String,
        Option<String>,
        Option<String>,
        String,
    );
    let row: Option<Row> = db
        .query_row(
            "SELECT workflow, key, step, repo, base_sha, coalesce(entered, '') FROM tasks WHERE id = ?1",
            [task],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )
        .optional()
        .map_err(error)?;
    let Some((workflow, key, current, repo, base, entered)) = row else {
        return Err(format!("no task {task}"));
    };
    let Some(start) = base else {
        return Err(format!(
            "task {task} has no start commit: it began before Priory stored one"
        ));
    };
    let ended = ended(db, task).map_err(error)?;
    let at_current = factory
        .workflows
        .get(&workflow)
        .and_then(|flow| flow.value.step(&current))
        .is_some_and(|step| step.effect == Effect::Irreversible);
    let boundary = ended
        .iter()
        .find(|step| step.effect == "irreversible")
        .map(|step| (step.step.clone(), step.started.clone()))
        .or_else(|| at_current.then(|| (current.clone(), entered.clone())));
    let Some((ends_at, cutoff)) = boundary else {
        return Err(format!(
            "task {task} reached no irreversible step, so a replay of it would have no end"
        ));
    };
    let cutoff: Timestamp = cutoff
        .parse()
        .map_err(|error| format!("task {task}: when it reached {ends_at}: {error}"))?;
    let before = |time: &str, step: &str| before(time, step, &ends_at, cutoff);

    let mut gaps = Vec::new();
    let request = request(db, task).map_err(error)?;
    if request.is_none() {
        gaps.push("the task's request is not in the inbox".to_owned());
    }
    let mut gh = Vec::new();
    let mut transcripts: BTreeMap<String, io::Result<Vec<u8>>> = BTreeMap::new();
    for run in runs(db, task, &before).map_err(error)? {
        let codex = run
            .agent
            .as_ref()
            .and_then(|agent| factory.agents.get(agent))
            .is_some_and(|agent| matches!(agent.value.harness, Harness::Codex));
        if codex {
            gaps.push(format!(
                "run {}: it ran on Codex, whose transcript the capture does not read yet",
                run.id
            ));
            continue;
        }
        let Some(path) = &run.transcript else {
            gaps.push(format!("run {}: no transcript was recorded", run.id));
            continue;
        };
        let bytes = transcripts
            .entry(path.clone())
            .or_insert_with(|| read(path));
        let bytes = match bytes {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                gaps.push(format!("run {}: its transcript is gone", run.id));
                continue;
            }
            Err(_) => {
                gaps.push(format!("run {}: its transcript cannot be read", run.id));
                continue;
            }
        };
        let to = (run.to as usize).min(bytes.len());
        let from = (run.from as usize).min(to);
        for call in transcript::shell_calls(&bytes[from..to]) {
            match keep(&call) {
                Kept::Ignored => {}
                Kept::Answer(args, output) => gh.push(GhAnswer {
                    run: run.id,
                    step: run.step.clone(),
                    args,
                    output,
                }),
                Kept::Gap(why) => {
                    gaps.push(format!("run {}: `{}` {why}", run.id, shown(&call.command)))
                }
            }
        }
    }
    let answers = answers(db, task, &before).map_err(error)?;
    let case = Case {
        workflow,
        key,
        repo,
        start,
        ends_at,
        partial: !gaps.is_empty(),
        gaps,
        request,
        reference: Reference {
            path: ended
                .iter()
                .map(|step| format!("{}:{}", step.step, step.outcome))
                .collect(),
            labels: labels(db, task).map_err(error)?,
        },
    };
    let files = case::files(task, &case, &gh, &answers)?;
    Ok(Captured {
        files,
        gh: gh.len(),
        answers: answers.len(),
        gaps: case.gaps.len(),
    })
}

/// Whether something that happened at `time` during `step` came before the task reached
/// `ends_at` at `cutoff`. Runs and asks record whole seconds, so in the cutoff's own second only
/// the irreversible step's own run or ask counts as after it.
fn before(time: &str, step: &str, ends_at: &str, cutoff: Timestamp) -> bool {
    let Ok(time) = time.parse::<Timestamp>() else {
        return false;
    };
    let second = |time: Timestamp| time.as_second();
    second(time) < second(cutoff) || (second(time) == second(cutoff) && step != ends_at)
}

struct Ended {
    step: String,
    outcome: String,
    effect: String,
    started: String,
}

/// The steps the task finished, oldest first, from its `factory.step.ended` records.
fn ended(db: &Connection, task: i64) -> rusqlite::Result<Vec<Ended>> {
    let mut statement = db.prepare(
        "SELECT json_extract(event, '$.data.step'), json_extract(event, '$.data.outcome'),
           json_extract(event, '$.data.effect'), json_extract(event, '$.data.started')
         FROM trace WHERE type = 'factory.step.ended' AND json_extract(event, '$.data.task') = ?1
           AND json_extract(event, '$.data.step') IS NOT NULL
           AND json_extract(event, '$.data.outcome') IS NOT NULL
         ORDER BY seq",
    )?;
    statement
        .query_map([task.to_string()], |row| {
            Ok(Ended {
                step: row.get(0)?,
                outcome: row.get(1)?,
                effect: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                started: row.get::<_, Option<String>>(3)?.unwrap_or_default(),
            })
        })?
        .collect()
}

/// The outcome labels the task holds: each label not cleared after it was given.
fn labels(db: &Connection, task: i64) -> rusqlite::Result<Vec<String>> {
    let mut statement = db.prepare(
        "SELECT DISTINCT json_extract(event, '$.data.label') FROM trace AS labeled
         WHERE type = 'factory.step.labeled' AND json_extract(event, '$.data.task') = ?1
           AND NOT EXISTS (SELECT 1 FROM trace AS cleared WHERE cleared.type = 'factory.step.unlabeled'
             AND cleared.seq > labeled.seq
             AND json_extract(cleared.event, '$.data.task') = ?1
             AND json_extract(cleared.event, '$.data.label') = json_extract(labeled.event, '$.data.label'))
         ORDER BY seq",
    )?;
    statement
        .query_map([task.to_string()], |row| row.get(0))?
        .collect()
}

/// The first message the inbox routed to the task: the one that asked for it.
fn request(db: &Connection, task: i64) -> rusqlite::Result<Option<Message>> {
    let row: Option<(String, Option<String>, String)> = db
        .query_row(
            "SELECT type, subject, data FROM inbox WHERE task = ?1 ORDER BY seq LIMIT 1",
            [task],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    Ok(row.map(|(kind, subject, data)| {
        let data = serde_json::from_str::<Value>(&data)
            .map(without_nulls)
            .ok()
            .and_then(|data| case::table(&data).ok())
            .unwrap_or_default();
        Message {
            kind,
            subject,
            data,
        }
    }))
}

/// `value` with its null fields and items left out, since TOML has no null.
fn without_nulls(value: Value) -> Value {
    match value {
        Value::Object(fields) => Value::Object(
            fields
                .into_iter()
                .filter(|(_, value)| !value.is_null())
                .map(|(key, value)| (key, without_nulls(value)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(
            items
                .into_iter()
                .filter(|item| !item.is_null())
                .map(without_nulls)
                .collect(),
        ),
        value => value,
    }
}

/// The task's runs that started before its first irreversible step, oldest first.
fn runs(
    db: &Connection,
    task: i64,
    before: &dyn Fn(&str, &str) -> bool,
) -> rusqlite::Result<Vec<Run>> {
    let mut statement = db.prepare(
        "SELECT id, step, agent, transcript, read, started,
           (SELECT max(earlier.read) FROM runs AS earlier
            WHERE earlier.transcript = runs.transcript AND earlier.id < runs.id)
         FROM runs WHERE task = ?1 ORDER BY id",
    )?;
    let rows = statement
        .query_map([task], |row| {
            let started: String = row.get(5)?;
            Ok((
                Run {
                    id: row.get(0)?,
                    step: row.get(1)?,
                    agent: row.get(2)?,
                    transcript: row.get(3)?,
                    from: row.get::<_, Option<i64>>(6)?.unwrap_or(0).max(0) as u64,
                    to: row.get::<_, i64>(4)?.max(0) as u64,
                },
                started,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows
        .into_iter()
        .filter(|(run, started)| before(started, &run.step))
        .map(|(run, _)| run)
        .collect())
}

/// The answers a person gave at the task's human steps before its first irreversible step.
fn answers(
    db: &Connection,
    task: i64,
    before: &dyn Fn(&str, &str) -> bool,
) -> rusqlite::Result<Vec<Answer>> {
    let mut statement = db.prepare(
        "SELECT step, answer, created FROM asks
         WHERE task = ?1 AND kind = 'step' AND answer IS NOT NULL AND withdrawn IS NULL
         ORDER BY id",
    )?;
    let rows = statement
        .query_map([task], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows
        .into_iter()
        .filter(|(step, _, created)| before(created, step))
        .map(|(step, answer, _)| Answer { step, answer })
        .collect())
}

/// What the capture does with one Bash call of a transcript.
#[derive(Debug, PartialEq)]
enum Kept {
    /// The call ran no `gh`.
    Ignored,
    /// One `gh` command alone, with its whole output: its arguments and what it printed.
    Answer(Vec<String>, String),
    /// A `gh` call the capture cannot answer for, and why.
    Gap(&'static str),
}

fn keep(call: &transcript::Shell) -> Kept {
    let commands = guard::commands(&call.command);
    if !commands.iter().any(|words| words[0] == "gh") {
        return Kept::Ignored;
    }
    let first = call.command.split_whitespace().next().unwrap_or_default();
    let alone =
        plain(&call.command) && commands.len() == 1 && (first == "gh" || first.ends_with("/gh"));
    if !alone {
        return Kept::Gap("is not one gh command alone");
    }
    match &call.output {
        None => Kept::Gap("has no output in the transcript"),
        Some(printed) if printed.failed => {
            Kept::Gap("failed, and the transcript does not hold its exit code")
        }
        Some(printed) if printed.truncated => Kept::Gap("printed more than the transcript holds"),
        Some(printed) => Kept::Answer(commands[0][1..].to_vec(), printed.text.clone()),
    }
}

/// Whether `command` is plain words: no pipe, list, redirection, substitution, glob or
/// expansion outside quotes, so the words alone say what ran.
fn plain(command: &str) -> bool {
    let (mut single, mut double, mut escaped) = (false, false, false);
    for c in command.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\\' if !single => escaped = true,
            '\'' if !double => single = !single,
            '"' if !single => double = !double,
            '$' | '`' if !single => return false,
            '|' | '&' | ';' | '<' | '>' | '(' | ')' | '\n' | '*' | '?' | '[' | '~' | '#'
                if !single && !double =>
            {
                return false;
            }
            _ => {}
        }
    }
    !single && !double && !escaped
}

/// A command as a gap line shows it: one line, at most 80 characters.
fn shown(command: &str) -> String {
    let line = command.split_whitespace().collect::<Vec<_>>().join(" ");
    match line.char_indices().nth(80) {
        Some((at, _)) => format!("{}...", &line[..at]),
        None => line,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use priory_runtime::transcript::{Printed, Shell};

    fn call(command: &str, text: &str) -> Shell {
        Shell {
            id: "t".to_owned(),
            command: command.to_owned(),
            output: Some(Printed {
                text: text.to_owned(),
                failed: false,
                truncated: false,
            }),
        }
    }

    #[test]
    fn only_one_gh_command_alone_with_its_whole_output_is_kept() {
        let kept = |command: &str| keep(&call(command, "out"));
        let answer = |args: &[&str]| {
            Kept::Answer(
                args.iter().map(|arg| arg.to_string()).collect(),
                "out".to_owned(),
            )
        };

        assert_eq!(kept("cargo test"), Kept::Ignored);
        assert_eq!(
            kept("gh issue view 42 --json title,body"),
            answer(&["issue", "view", "42", "--json", "title,body"])
        );
        assert_eq!(
            kept("gh api repos/o/r --jq '.items[] | .name'"),
            answer(&["api", "repos/o/r", "--jq", ".items[] | .name"])
        );
        for command in [
            "gh issue view 42 | head",
            "gh issue view 42 > out.txt",
            "gh issue view $N",
            "cd x && gh pr list",
            "GH_REPO=o/r gh pr list",
            "env GH_REPO=o/r gh pr list",
            "bash -c 'gh pr list'",
        ] {
            assert_eq!(
                kept(command),
                Kept::Gap("is not one gh command alone"),
                "{command}"
            );
        }
        let mut cut = call("gh pr list", "a");
        cut.output.as_mut().unwrap().truncated = true;
        assert_eq!(
            keep(&cut),
            Kept::Gap("printed more than the transcript holds")
        );
        cut.output = None;
        assert_eq!(keep(&cut), Kept::Gap("has no output in the transcript"));
    }

    #[test]
    fn only_the_irreversible_step_s_own_second_is_after_the_cutoff() {
        let cutoff: Timestamp = "2026-11-22T10:00:05.300Z".parse().unwrap();
        let before = |time: &str, step: &str| before(time, step, "open-pr", cutoff);

        assert!(before("2026-11-22T10:00:04Z", "open-pr"));
        assert!(before("2026-11-22T10:00:05Z", "describe"));
        assert!(!before("2026-11-22T10:00:05Z", "open-pr"));
        assert!(!before("2026-11-22T10:00:06Z", "describe"));
        assert!(!before("not a time", "describe"));
    }
}
