//! The `intent` scorer: asks a decision model yes or no questions drawn from the intent and the
//! no-op cases of the task's trigger, and judges the task's latest finished run by the answers.
//! docs/daemon/score.md says what it asks and how it answers.

use crate::score::{Run, Task, Verdict};
use priory_factory::factory::Scorer;
use rusqlite::Connection;
use serde_json::{Map, Value, json};
use std::path::Path;
use std::process::Command;

/// What the decision model may answer each question.
const OPTIONS: [&str; 3] = ["YES", "NO", "UNKNOWN"];

/// The question about the intent; each no-op case's is `noop-<n>`, from 1.
const INTENT: &str = "intent";

/// What an intent scorer answered for one run: its verdict, its line of why, and what the trace
/// record keeps of the call.
pub(crate) struct Graded {
    pub verdict: Verdict,
    pub reason: String,
    pub record: Option<Value>,
}

fn graded(verdict: Verdict, reason: impl Into<String>, record: Option<Value>) -> Graded {
    Graded {
        verdict,
        reason: reason.into(),
        record,
    }
}

/// The questions about a task whose trigger has `intent` and `noop`, about `state`.
fn questions(intent: Option<&str>, noop: &[String], state: Value) -> Value {
    let criteria = |yes: &str, no: &str| {
        json!({
            "YES": yes,
            "NO": no,
            "UNKNOWN": "The state does not say",
        })
    };
    let mut asked = Map::new();
    if let Some(intent) = intent {
        asked.insert(
            INTENT.to_owned(),
            json!({
                "type": "choice",
                "instructions": format!("Did the task's work serve this intent? {intent}"),
                "criteria": criteria("It served the intent", "It did not serve the intent"),
            }),
        );
    }
    for (at, case) in noop.iter().enumerate() {
        asked.insert(
            format!("noop-{}", at + 1),
            json!({
                "type": "choice",
                "instructions": format!(
                    "Is the request this case, which must create no ask and no write? {case}"
                ),
                "criteria": criteria("The request is this case", "The request is not this case"),
            }),
        );
    }
    json!({ "state": state, "questions": asked })
}

/// Grades `run` of `task` with the intent scorer `scorer`, whose decision model lives in the
/// factory folder `dir`. A call that fails is `not evaluated`, never `UNKNOWN`.
pub(crate) fn grade(dir: &Path, scorer: &Scorer, task: &Task, run: &Run) -> Graded {
    let (intent, noop) = (task.intent.as_deref(), &task.noop);
    if intent.is_none() && noop.is_empty() {
        return graded(
            Verdict::NotApplicable,
            "the task's trigger names no intent and no no-op case",
            None,
        );
    }
    if !run.latest {
        return graded(
            Verdict::NotApplicable,
            "an intent scorer grades the task's latest finished run",
            None,
        );
    }
    // A gate grades a result it has not stored yet, so its commit counts here.
    let named = run
        .result
        .as_ref()
        .is_some_and(|result| result["commit"].is_string());
    let (asked, wrote) = (task.asked, task.wrote || named);
    let state = json!({
        "request": task.request,
        "result": run.result,
        "asked": asked,
        "wrote": wrote,
    });
    let question = questions(intent, noop, state);
    let answers = match call(dir, scorer, &question) {
        Ok(answers) => answers,
        Err(why) => {
            return graded(
                Verdict::Error,
                format!("not evaluated: {why}"),
                Some(json!({ "evaluated": false })),
            );
        }
    };
    let choices: Map<String, Value> = answers
        .names
        .iter()
        .zip(&answers.choices)
        .map(|(name, choice)| (name.clone(), json!(choice)))
        .collect();
    let mut record = json!({
        "evaluated": true,
        "model": answers.model,
        "answers": choices,
        // A model that reports no usage is unavailable, not free.
        "input_tokens": answers.input_tokens.map_or(json!("unavailable"), |tokens| json!(tokens)),
    });
    let choice = |name: &str| {
        answers
            .names
            .iter()
            .position(|asked| asked == name)
            .map(|at| answers.choices[at].as_str())
    };
    let matched = noop
        .iter()
        .enumerate()
        .find(|(at, _)| choice(&format!("noop-{}", at + 1)) == Some("YES"));
    if let Some((_, case)) = matched {
        record["noop_case"] = json!(case);
        let made: Vec<&str> = [(asked, "asked"), (wrote, "wrote")]
            .into_iter()
            .filter_map(|(did, what)| did.then_some(what))
            .collect();
        return match made.as_slice() {
            [] => graded(
                Verdict::Pass,
                format!("no-op case \"{case}\" holds, and the task asked and wrote nothing"),
                Some(record),
            ),
            made => graded(
                Verdict::Fail,
                format!(
                    "no-op case \"{case}\" holds, but the task {}",
                    made.join(" and ")
                ),
                Some(record),
            ),
        };
    }
    let unknown = noop
        .iter()
        .enumerate()
        .find(|(at, _)| choice(&format!("noop-{}", at + 1)) == Some("UNKNOWN"));
    if let Some((_, case)) = unknown {
        return graded(
            Verdict::NotApplicable,
            format!("UNKNOWN whether no-op case \"{case}\" holds"),
            Some(record),
        );
    }
    let (verdict, reason) = match choice(INTENT) {
        None => (Verdict::Pass, "no no-op case holds"),
        Some("YES") => (Verdict::Pass, "the task served its trigger's intent"),
        Some("NO") => (Verdict::Fail, "the task did not serve its trigger's intent"),
        Some(_) => (
            Verdict::NotApplicable,
            "UNKNOWN whether the task served its trigger's intent",
        ),
    };
    graded(verdict, reason, Some(record))
}

/// Whether `task` raised an ask, and whether it wrote: a result that names a commit, or a pull
/// request.
pub(crate) fn attention(db: &Connection, task: i64) -> rusqlite::Result<(bool, bool)> {
    db.query_row(
        "SELECT EXISTS (SELECT 1 FROM asks WHERE task = ?1),
           EXISTS (SELECT 1 FROM results WHERE task = ?1 AND commit_sha IS NOT NULL)
             OR EXISTS (SELECT 1 FROM pull_requests WHERE task = ?1)",
        [task],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
}

/// The decision model's answers, in the order of the questions.
struct Answers {
    model: String,
    names: Vec<String>,
    choices: Vec<String>,
    input_tokens: Option<u64>,
}

/// Runs the scorer's decision model on `question` and reads one answer to each question, all from
/// one model.
fn call(dir: &Path, scorer: &Scorer, question: &Value) -> Result<Answers, String> {
    let run = scorer.run.as_deref().ok_or("the scorer names no `run`")?;
    let program = crate::score::inside(dir, run)?;
    let mut command = Command::new(program);
    command.current_dir(dir);
    let stdout = crate::decision::run(
        command,
        scorer.key_env.as_deref(),
        question,
        scorer.timeout(),
    )?;
    let names: Vec<String> = question["questions"]
        .as_object()
        .map(|questions| questions.keys().cloned().collect())
        .unwrap_or_default();
    let mut answers = Answers {
        model: String::new(),
        names: Vec::new(),
        choices: Vec::new(),
        input_tokens: None,
    };
    for name in names {
        let answer = crate::decision::parse(&stdout, &name, &OPTIONS)?;
        answers.model = answer.model;
        answers.input_tokens = answer.input_tokens;
        answers.choices.push(answer.choice);
        answers.names.push(name);
    }
    Ok(answers)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn questions_ask_about_the_intent_then_each_noop_case_with_three_options() {
        let noop = vec!["A question.".to_owned(), "A duplicate.".to_owned()];

        let asked = questions(Some("Fix bugs."), &noop, json!({}));

        let mut names: Vec<_> = asked["questions"].as_object().unwrap().keys().collect();
        names.sort();
        assert_eq!(names, ["intent", "noop-1", "noop-2"]);
        assert_eq!(
            asked["questions"]["noop-2"]["instructions"],
            "Is the request this case, which must create no ask and no write? A duplicate."
        );
        let mut options: Vec<_> = asked["questions"]["intent"]["criteria"]
            .as_object()
            .unwrap()
            .keys()
            .collect();
        options.sort();
        assert_eq!(options, ["NO", "UNKNOWN", "YES"]);
    }
}
