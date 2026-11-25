//! `priory recap`: what the day's tasks concluded and what people answered, bounded and redacted,
//! for a dream to read. It never reads a transcript, a screen, a file or an outside event's body.

use crate::tools::SAYS;
use rusqlite::Connection;

/// How far back `priory recap` reads when it names no `--since`.
pub const SINCE: &str = "24h";
/// The most bytes `priory recap` prints when it names no `--max-bytes`.
pub const MAX_BYTES: usize = 200_000;
/// How long a dropped candidate's name stays in the recap, so a dream does not propose it again.
const DROPPED_FOR: &str = "-30 days";

/// One record of the recap and the time it happened, which orders the records.
struct Record {
    time: String,
    text: String,
}

/// `text` with each credential shape replaced by `[redacted]`, or wholly replaced when one still
/// shows.
fn redacted(text: &str) -> String {
    crate::screen::redact(text, &[], None).map_or_else(|| "[redacted]".to_owned(), |(text, _)| text)
}

/// A fenced block of `text`, which names its task and its author. The fence is longer than any run
/// of backticks in `text`, so no quoted text can close it.
fn quote(task: &str, author: &str, text: &str) -> String {
    let mut longest = 0;
    let mut run = 0;
    for char in text.chars() {
        run = if char == '`' { run + 1 } else { 0 };
        longest = longest.max(run);
    }
    let fence = "`".repeat((longest + 1).max(3));
    let info = format!("{task}, by {author}").replace(['`', '\n', '\r'], "'");
    format!("{fence} {info}\n{}\n{fence}", redacted(text.trim_end()))
}

/// Who answered an ask, from its `answered_by`.
fn answerer(by: Option<&str>) -> String {
    match by {
        Some(by) if by.starts_with("agent:") => format!("agent {}", &by["agent:".len()..]),
        Some(by) if by.starts_with(crate::tools::SESSION) => {
            format!("session {}", &by[crate::tools::SESSION.len()..])
        }
        Some(by @ (crate::tools::AUTO | crate::tools::AUTOPILOT)) => by.to_owned(),
        Some(by) if !by.is_empty() && by != "cli" && by != "serve" => format!("person {by}"),
        _ => "a person".to_owned(),
    }
}

/// The task's label in a record: its id and key.
fn label(task: i64, key: &str) -> String {
    redacted(&format!("task {task} ({key})")).replace('\n', " ")
}

/// The recap of the window from `start` to `now`, oldest first, in at most `max_bytes` bytes: it
/// keeps the candidates' names, then the newest records that fit, and says how many it left out.
pub fn recap(
    db: &Connection,
    start: &str,
    now: &str,
    max_bytes: usize,
) -> rusqlite::Result<String> {
    let records = records(db, start, now)?;
    let names = names(db, now)?;
    let header = format!("# Recap from {start} to {now}\n");
    let names_text = if names.is_empty() {
        String::new()
    } else {
        format!(
            "\n## Memory names not to propose again\n\nA candidate waits for a person, or a person \
             dropped it. Each is a name only.\n\n{}\n",
            names.join("\n")
        )
    };
    let total = records.len();
    let mut room = max_bytes.saturating_sub(header.len() + names_text.len() + FOOTER_ROOM);
    let mut kept = Vec::new();
    for record in records.iter().rev() {
        let block = format!("\n{}\n", record.text);
        if block.len() > room {
            break;
        }
        room -= block.len();
        kept.push(block);
    }
    kept.reverse();
    let mut out = header;
    if max_bytes >= out.len() + names_text.len() + FOOTER_ROOM {
        out.push_str(&names_text);
    }
    if total == 0 {
        out.push_str("\nNo task concluded, and nobody answered or said anything, in the window.\n");
    }
    for block in &kept {
        out.push_str(block);
    }
    let left = total - kept.len();
    if left > 0 {
        out.push_str(&format!(
            "\nLeft out the {left} oldest of {total} records to stay under {max_bytes} bytes.\n"
        ));
    }
    // A last pass hides a shape that two records could only form together.
    let mut out = redacted(&out);
    if out.len() > max_bytes {
        let mut end = max_bytes;
        while !out.is_char_boundary(end) {
            end -= 1;
        }
        out.truncate(end);
    }
    Ok(out)
}

/// What the closing line about left-out records may take.
const FOOTER_ROOM: usize = 120;

/// The window's records, oldest second first. Within one second they come as results, answers,
/// says, step ends, then stopped tasks.
fn records(db: &Connection, start: &str, now: &str) -> rusqlite::Result<Vec<Record>> {
    let mut records = Vec::new();
    let within = "julianday(?1) <= julianday(TIME) AND julianday(TIME) <= julianday(?2)";
    let window = |time: &str| within.replace("TIME", time);

    let mut results = db.prepare(&format!(
        "SELECT results.created, results.task, tasks.key, tasks.workflow, results.step,
           results.outcome, results.summary, results.evidence, runs.agent
         FROM results JOIN tasks ON tasks.id = results.task
         LEFT JOIN runs ON runs.id = results.run
         WHERE {} ORDER BY results.rowid",
        window("results.created")
    ))?;
    let rows = results.query_map([start, now], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, String>(6)?,
            row.get::<_, Option<String>>(7)?,
            row.get::<_, Option<String>>(8)?,
        ))
    })?;
    for row in rows {
        let (time, task, key, workflow, step, outcome, summary, evidence, agent) = row?;
        let task = label(task, &key);
        let author = format!("agent {}", agent.as_deref().unwrap_or("unknown"));
        let mut text = format!(
            "## Result of {task}\n\nWorkflow {workflow}, step {step}, outcome {outcome}, at {time}.\n\n{}",
            quote(&task, &author, &summary)
        );
        if let Some(evidence) = evidence.filter(|evidence| !evidence.trim().is_empty()) {
            text.push_str(&format!(
                "\n\nEvidence:\n\n{}",
                quote(&task, &author, &evidence)
            ));
        }
        records.push(Record { time, text });
    }

    let mut asks = db.prepare(&format!(
        "SELECT asks.answered, asks.task, tasks.key, asks.step, asks.question, asks.options,
           asks.recommendation, asks.answer, asks.answered_by, asks.kind, runs.agent
         FROM asks JOIN tasks ON tasks.id = asks.task
         LEFT JOIN runs ON runs.id = asks.run
         WHERE asks.answer IS NOT NULL AND {} ORDER BY asks.id",
        window("asks.answered")
    ))?;
    let rows = asks.query_map([start, now], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, Option<String>>(6)?,
            row.get::<_, String>(7)?,
            row.get::<_, Option<String>>(8)?,
            row.get::<_, String>(9)?,
            row.get::<_, Option<String>>(10)?,
        ))
    })?;
    for row in rows {
        let (time, task, key, step, question, options, recommendation, answer, by, kind, agent) =
            row?;
        let task = label(task, &key);
        let asker = match (kind.as_str(), agent) {
            ("agent", Some(agent)) => format!("agent {agent}"),
            _ => "the workflow".to_owned(),
        };
        let options: Vec<String> = serde_json::from_str(&options).unwrap_or_default();
        let chosen = answer
            .split_once(':')
            .map_or(answer.as_str(), |(option, _)| option)
            .trim();
        let mut text = format!(
            "## Answer on {task}\n\nStep {}, answered at {time}.",
            step.as_deref().unwrap_or("-")
        );
        if !options.is_empty() {
            text.push_str(&format!(
                " Options: {}.",
                redacted(&options.join(", ")).replace('\n', " ")
            ));
        }
        if let Some(recommendation) = &recommendation {
            // A recommendation names its option first, as `sqlite: one file`.
            let recommended = recommendation
                .split_once(':')
                .map_or(recommendation.as_str(), |(option, _)| option)
                .trim();
            let against = if recommended == chosen {
                "followed"
            } else {
                "went against"
            };
            text.push_str(&format!(
                " Recommended: {}. The answer {against} it.",
                redacted(recommendation).replace('\n', " ")
            ));
        }
        text.push_str(&format!(
            "\n\nQuestion:\n\n{}\n\nAnswer:\n\n{}",
            quote(&task, &asker, &question),
            quote(&task, &answerer(by.as_deref()), &answer)
        ));
        records.push(Record { time, text });
    }

    let mut said = db.prepare(&format!(
        "SELECT outbox.created, outbox.task, tasks.key, outbox.text
         FROM outbox JOIN tasks ON tasks.id = outbox.task
         WHERE instr(char(10) || outbox.text, char(10) || ?3) > 0 AND {} ORDER BY outbox.id",
        window("outbox.created")
    ))?;
    let rows = said.query_map(rusqlite::params![start, now, SAYS], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
        ))
    })?;
    for row in rows {
        let (time, task, key, prompt) = row?;
        let task = label(task, &key);
        for line in prompt.lines().filter_map(|line| line.strip_prefix(SAYS)) {
            records.push(Record {
                time: time.clone(),
                text: format!(
                    "## A person said to {task}\n\nAt {time}.\n\n{}",
                    quote(&task, "a person", line)
                ),
            });
        }
    }

    let mut ended = db.prepare(&format!(
        "SELECT time, json_extract(event, '$.data.task'), json_extract(event, '$.data.key'),
           json_extract(event, '$.data.workflow'), json_extract(event, '$.data.step'),
           json_extract(event, '$.data.outcome'), json_extract(event, '$.data.tokens')
         FROM trace WHERE type = 'factory.step.ended' AND {} ORDER BY seq",
        window("time")
    ))?;
    let rows = ended.query_map([start, now], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, Option<String>>(4)?,
            row.get::<_, Option<String>>(5)?,
            row.get::<_, Option<String>>(6)?,
        ))
    })?;
    for row in rows {
        let (time, task, key, workflow, step, outcome, tokens) = row?;
        let task = label(
            task.and_then(|task| task.parse().ok()).unwrap_or_default(),
            key.as_deref().unwrap_or("-"),
        );
        let tokens = tokens
            .and_then(|tokens| serde_json::from_str::<serde_json::Value>(&tokens).ok())
            .map_or_else(
                || "tokens unknown".to_owned(),
                |tokens| {
                    let count = |field: &str| tokens[field].as_u64().unwrap_or(0);
                    format!(
                        "{} tokens in, {} out, {} read from cache, {} written to cache",
                        count("input"),
                        count("output"),
                        count("cache_read"),
                        count("cache_write")
                    )
                },
            );
        let line = format!(
            "## Step ended on {task}\n\nWorkflow {}, step {}, outcome {}, at {time}: {tokens}.",
            workflow.as_deref().unwrap_or("-"),
            step.as_deref().unwrap_or("-"),
            outcome.as_deref().unwrap_or("-"),
        );
        records.push(Record {
            time,
            text: redacted(&line),
        });
    }

    let mut stopped = db.prepare(&format!(
        "SELECT changed, id, key, workflow, step, state, reason FROM tasks
         WHERE state IN ('failed', 'blocked') AND {} ORDER BY id",
        window("changed")
    ))?;
    let rows = stopped.query_map([start, now], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, Option<String>>(6)?,
        ))
    })?;
    for row in rows {
        let (time, task, key, workflow, step, state, reason) = row?;
        let task = label(task, &key);
        let mut text =
            format!("## {task} is {state}\n\nWorkflow {workflow}, step {step}, since {time}.");
        if let Some(reason) = reason {
            text.push_str(&format!(
                "\n\nReason:\n\n{}",
                quote(&task, "Priory", &reason)
            ));
        }
        records.push(Record { time, text });
    }

    // The state keeps whole seconds, so a stable sort by second keeps a stopped task after a step
    // end of its own second.
    records.sort_by_cached_key(|record| {
        record
            .time
            .parse::<jiff::Timestamp>()
            .map(|time| time.as_second())
            .unwrap_or_default()
    });
    Ok(records)
}

/// A line for each pending candidate and each candidate dropped in the last 30 days, by name.
fn names(db: &Connection, now: &str) -> rusqlite::Result<Vec<String>> {
    db.prepare(&format!(
        "SELECT name, CASE WHEN dropped IS NULL THEN 'pending since ' || substr(written, 1, 10)
           ELSE 'dropped on ' || substr(decided, 1, 10) END
         FROM memory_candidates
         WHERE logged IS NOT NULL AND accepted IS NULL AND (dropped IS NULL
           OR julianday(decided) >= julianday(?1, '{DROPPED_FOR}'))
         ORDER BY coalesce(decided, written), rowid"
    ))?
    .query_map([now], |row| {
        Ok(format!(
            "- `{}`: {}",
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?
        ))
    })?
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use rusqlite::params;

    const NOW: &str = "2026-11-22T03:00:00Z";
    const START: &str = "2026-11-21T03:00:00Z";

    fn day(summaries: &[String], answers: &[String]) -> Connection {
        let db = crate::state::memory();
        db.execute(
            "INSERT INTO tasks (key, workflow, step, state, trigger, source)
             VALUES ('github:o/r#1', 'fix', 'implement', 'running', 't', 's')",
            [],
        )
        .unwrap();
        for (index, summary) in summaries.iter().enumerate() {
            db.execute(
                "INSERT INTO runs (task, step, tmux, session, agent, ended)
                 VALUES (1, 'implement', 't', ?1, 'coder', '2026-11-21T10:00:00Z')",
                [format!("s{index}")],
            )
            .unwrap();
            db.execute(
                "INSERT INTO results (run, task, step, outcome, summary, evidence, created)
                 VALUES (last_insert_rowid(), 1, 'implement', 'passed', ?1, ?1, ?2)",
                params![summary, format!("2026-11-21T10:{:02}:00Z", index % 60)],
            )
            .unwrap();
        }
        for (index, answer) in answers.iter().enumerate() {
            db.execute(
                "INSERT INTO asks (task, step, kind, question, options, recommendation, answer,
                   answered, answered_by)
                 VALUES (1, 'implement', 'step', ?1, '[\"a\",\"b\"]', 'a', ?1, ?2, 'ana')",
                params![answer, format!("2026-11-21T11:{:02}:00Z", index % 60)],
            )
            .unwrap();
        }
        db
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(16))]

        #[test]
        fn a_recap_never_panics_never_passes_its_limit_and_shows_no_secret_shape(
            summaries in prop::collection::vec(
                prop_oneof![
                    any::<String>(),
                    "[a-z ]{0,40}(ghp_[A-Za-z0-9]{24}|sk-[A-Za-z0-9]{24}|AKIA[0-9A-Z]{16})[`a-z ]{0,40}",
                    "-----BEGIN RSA PRIVATE KEY-----\n[A-Za-z0-9\n]{0,80}",
                ],
                0..6,
            ),
            answers in prop::collection::vec(any::<String>(), 0..4),
            max_bytes in 0usize..6000,
        ) {
            let db = day(&summaries, &answers);
            let out = recap(&db, START, NOW, max_bytes).unwrap();
            prop_assert!(out.len() <= max_bytes, "{} bytes over {max_bytes}", out.len());
            prop_assert!(!crate::screen::holds_secret(&out), "{out}");
        }
    }

    #[test]
    fn a_task_stopped_in_the_second_of_a_step_end_is_the_newest_record() {
        let db = day(&["one".to_owned()], &[]);
        db.execute(
            "INSERT INTO trace (id, type, time, event) VALUES ('e1', 'factory.step.ended',
               '2026-11-21T10:00:00.615Z', '{\"data\":{\"task\":\"1\",\"step\":\"implement\"}}')",
            [],
        )
        .unwrap();
        for sql in [
            "UPDATE tasks SET state = 'blocked'",
            "UPDATE tasks SET changed = '2026-11-21T10:00:00Z'",
        ] {
            db.execute(sql, []).unwrap();
        }
        let out = recap(&db, START, NOW, MAX_BYTES).unwrap();
        let at = |text: &str| out.find(text).unwrap_or_else(|| panic!("{text} in {out}"));
        assert!(at("## Result of") < at("## Step ended"), "{out}");
        assert!(at("## Step ended") < at("is blocked"), "{out}");
    }

    #[test]
    fn a_quote_fence_outlasts_the_backticks_it_holds() {
        let quoted = quote("task 1 (k)", "a person", "a ```` b");
        assert!(
            quoted.starts_with("````` task 1 (k), by a person\n"),
            "{quoted}"
        );
        assert!(quoted.ends_with("\n`````"), "{quoted}");
    }
}
