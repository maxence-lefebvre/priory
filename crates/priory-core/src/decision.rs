//! A decision model: a command that answers typed questions with probabilities. Priory acts on an
//! answer only once `priory calibrate` measured the model against a person's labels.
//! docs/design/deciders.md explains it, and docs/daemon/decisions.md says what the daemon does.

use crate::trace::{self, Trace};
use herdr_bus::Message;
use priory_factory::factory::{Decision, Factory};
use priory_factory::labels::{self, Class};
use priory_runtime::{room, runtime, transcript};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::process::Command;
use std::time::Instant;

/// The decision point this module serves.
const TRIAGE: &str = "triage";

/// The most bytes of state Priory sends in one call.
const STATE_LIMIT: usize = 16 * 1024;

/// The key Priory marks a feedback item with when a calibrated model sorted it `nothing-to-do`.
pub(crate) const DECIDED: &str = "decided";

/// What a decision model answered one choice question.
#[derive(Debug, Clone, PartialEq)]
pub struct Answer {
    pub model: String,
    pub choice: String,
    pub probability: f64,
    pub input_tokens: Option<u64>,
}

/// The answer to the choice question `name` among `options` in `stdout`, or why it is not one.
pub fn parse(stdout: &[u8], name: &str, options: &[&str]) -> Result<Answer, String> {
    let not = |why: &str| format!("the answer {why}");
    let answer: Value = serde_json::from_slice(stdout).map_err(|_| not("is not JSON"))?;
    let model = answer["model"]
        .as_str()
        .filter(|model| !model.is_empty())
        .ok_or_else(|| not("names no model"))?;
    let question = &answer["answers"][name];
    let choice = question["choice"]
        .as_str()
        .ok_or_else(|| not(&format!("has no choice for {name}")))?;
    if !options.contains(&choice) {
        return Err(not(&format!("picks {choice:?}, not one of the options")));
    }
    let probability = question["probabilities"][choice]
        .as_f64()
        .filter(|probability| (0.0..=1.0).contains(probability))
        .ok_or_else(|| not(&format!("gives {choice} no probability between 0 and 1")))?;
    Ok(Answer {
        model: model.to_owned(),
        choice: choice.to_owned(),
        probability,
        input_tokens: answer["usage"]["input_tokens"].as_u64(),
    })
}

/// The triage question about a feedback item that says `state`.
fn question(state: Value) -> Value {
    let criteria: serde_json::Map<String, Value> = Class::ALL
        .iter()
        .map(|class| {
            let meaning = match class {
                Class::Trivial => "One obvious change inside the pull request's scope",
                Class::PersonsCall => {
                    "A behaviour, design or scope change, a disagreement, or anything unclear"
                }
                Class::NothingToDo => "Thanks, an approval, or a remark that asks for no change",
            };
            (class.name().to_owned(), json!(meaning))
        })
        .collect();
    json!({
        "state": state,
        "questions": { TRIAGE: {
            "type": "choice",
            "instructions": "How should the agent that wrote this pull request treat this feedback?",
            "criteria": criteria,
        }},
    })
}

/// Runs the decision model on `question`, with the key and no other secret in its environment.
fn call(factory: &Factory, decision: &Decision, question: &Value) -> Result<Answer, String> {
    let key = std::env::var(&decision.key_env)
        .ok()
        .filter(|key| !key.is_empty())
        .ok_or_else(|| format!("{} is unset", decision.key_env))?;
    let mut command = Command::new("sh");
    command
        .args(["-c", &decision.run])
        .current_dir(&factory.dir)
        .env_clear()
        .env(&decision.key_env, key);
    for name in ["PATH", "HOME"] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    let timeout = decision.timeout();
    let output = runtime::fed_within(&mut command, Some(question.to_string().into()), timeout)
        .map_err(|error| format!("cannot run it: {error}"))?
        .ok_or_else(|| format!("it ran past {}s", timeout.as_secs()))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let first = stderr.lines().next().unwrap_or_default();
        return Err(match output.status.code() {
            Some(code) => format!("it exited {code}: {first}"),
            None => format!("it was killed: {first}"),
        });
    }
    let options = Class::ALL.map(Class::name);
    parse(&output.stdout, TRIAGE, &options)
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// The triage decision model of `factory`, with the hashes its calibration is tied to.
struct Point<'a> {
    decision: &'a Decision,
    run_sha256: String,
    cases_sha256: String,
}

fn point(factory: &Factory) -> Result<Point<'_>, String> {
    let decision = factory
        .triage()
        .ok_or("the factory names no [decisions.triage]")?;
    let path = factory.dir.join(&decision.evaluation);
    let cases = std::fs::read(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(Point {
        decision,
        run_sha256: sha256(decision.run.as_bytes()),
        cases_sha256: sha256(&cases),
    })
}

/// One labeled case, as `priory calibrate` prints it.
pub struct Row {
    pub case: String,
    pub label: Class,
    pub answer: Result<Answer, String>,
}

impl Row {
    pub fn agreed(&self) -> bool {
        self.answer
            .as_ref()
            .is_ok_and(|answer| answer.choice == self.label.name())
    }

    pub fn json(&self) -> Value {
        let (choice, probability, error) = match &self.answer {
            Ok(answer) => (json!(answer.choice), json!(answer.probability), Value::Null),
            Err(error) => (Value::Null, Value::Null, json!(error)),
        };
        json!({
            "case": self.case, "label": self.label.name(), "choice": choice,
            "probability": probability, "agreed": self.agreed(), "error": error,
        })
    }
}

/// What a calibration measured, and whether it stored it.
pub struct Calibrated {
    pub rows: Vec<Row>,
    pub needs: f64,
    pub threshold: f64,
    /// Why nothing was stored, when nothing was.
    pub refused: Option<String>,
}

impl Calibrated {
    pub fn agreed(&self) -> usize {
        self.rows.iter().filter(|row| row.agreed()).count()
    }

    /// The cases labeled other than `nothing-to-do` that the model sorted `nothing-to-do` at or
    /// above the threshold: each one a request the agent would not have seen.
    pub fn misses(&self) -> Vec<&str> {
        self.rows
            .iter()
            .filter(|row| {
                row.label != Class::NothingToDo
                    && row.answer.as_ref().is_ok_and(|answer| {
                        answer.choice == Class::NothingToDo.name()
                            && answer.probability >= self.threshold
                    })
            })
            .map(|row| row.case.as_str())
            .collect()
    }

    pub fn agreement(&self) -> f64 {
        self.agreed() as f64 / self.rows.len().max(1) as f64
    }

    /// Whether the decision model's answers count after this calibration.
    pub fn counts(&self) -> bool {
        self.refused.is_none() && self.agreement() >= self.needs && self.misses().is_empty()
    }

    /// The line after the table.
    pub fn line(&self) -> String {
        let measured = format!(
            "agreement: {}/{} labeled cases, {}, needs {}",
            self.agreed(),
            self.rows.len(),
            crate::judge::percent(self.agreement()),
            crate::judge::percent(self.needs)
        );
        if let Some(refused) = &self.refused {
            return format!("{measured}: {refused}, so nothing was stored");
        }
        let misses = self.misses();
        if !misses.is_empty() {
            return format!(
                "{measured}: not calibrated: it sorted {} as nothing-to-do at the threshold",
                misses.join(", ")
            );
        }
        match self.counts() {
            true => format!("{measured}: calibrated"),
            false => format!("{measured}: not calibrated"),
        }
    }
}

/// `priory calibrate triage`: asks the decision model each labeled case and, when every call
/// answered with one model, appends a `factory.decision.calibrated` record.
pub fn calibrate(db: &mut Connection, factory: &Factory) -> Result<Calibrated, String> {
    let point = point(factory)?;
    let decision = point.decision;
    let path = factory.dir.join(&decision.evaluation);
    let cases = labels::load_triage(&path).map_err(|why| format!("{}: {why}", path.display()))?;
    let rows: Vec<Row> = cases
        .cases
        .iter()
        .map(|case| {
            let kind = case
                .kind
                .as_deref()
                .unwrap_or("github.pull_request.comment");
            let state = json!({ "kind": kind, "body": case.body });
            Row {
                case: case.name.clone(),
                label: case.label,
                answer: call(factory, decision, &question(state)),
            }
        })
        .collect();
    let mut calibrated = Calibrated {
        rows,
        needs: decision.agreement(),
        threshold: decision.threshold(),
        refused: None,
    };
    let models: std::collections::BTreeSet<&str> = calibrated
        .rows
        .iter()
        .filter_map(|row| row.answer.as_ref().ok())
        .map(|answer| answer.model.as_str())
        .collect();
    if let Some(failed) = calibrated.rows.iter().find(|row| row.answer.is_err()) {
        calibrated.refused = Some(format!("the call for {} failed", failed.case));
        return Ok(calibrated);
    }
    if models.len() > 1 {
        let models: Vec<&str> = models.into_iter().collect();
        calibrated.refused = Some(format!("{} answered, not one model", models.join(" and ")));
        return Ok(calibrated);
    }
    let model = models.into_iter().next().unwrap_or_default().to_owned();
    let trace = Trace {
        factory,
        revision: runtime::revision(&factory.dir),
        runner: None,
        machine: room::hostname(),
        usage: transcript::usage,
    };
    let data = json!({
        "factory": factory.name,
        "factory_revision": trace.revision,
        "point": TRIAGE,
        "model": model,
        "run_sha256": point.run_sha256,
        "cases_sha256": point.cases_sha256,
        "threshold": calibrated.threshold,
        "cases": calibrated.rows.len(),
        "agreed": calibrated.agreed(),
        "agreement": (calibrated.agreement() * 10_000.0).round() / 10_000.0,
        "needs": calibrated.needs,
        "misses": calibrated.misses(),
    });
    let error = |error: rusqlite::Error| error.to_string();
    let tx = db.transaction().map_err(error)?;
    trace::record(&tx, &trace, "factory.decision.calibrated", data).map_err(error)?;
    tx.commit().map_err(error)?;
    Ok(calibrated)
}

/// The model the triage point is calibrated on, or why its answers do not count.
fn calibrated(db: &Connection, factory: &Factory) -> Result<String, String> {
    let point = point(factory).map_err(|why| format!("not calibrated: {why}"))?;
    let stored: Option<(String, f64, i64)> = db
        .query_row(
            "SELECT json_extract(event, '$.data.model'), json_extract(event, '$.data.agreement'),
               json_array_length(json_extract(event, '$.data.misses'))
             FROM trace WHERE type = 'factory.decision.calibrated'
               AND json_extract(event, '$.data.point') = ?1
               AND json_extract(event, '$.data.run_sha256') = ?2
               AND json_extract(event, '$.data.cases_sha256') = ?3
               AND json_extract(event, '$.data.threshold') = ?4
             ORDER BY seq DESC LIMIT 1",
            params![
                TRIAGE,
                point.run_sha256,
                point.cases_sha256,
                point.decision.threshold()
            ],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(|error| format!("not calibrated: {error}"))?;
    let Some((model, agreement, misses)) = stored else {
        return Err(
            "not calibrated: no calibration for this command, threshold and cases: run priory calibrate triage"
                .to_owned(),
        );
    };
    let needs = point.decision.agreement();
    if agreement < needs {
        return Err(format!(
            "not calibrated: it agreed on {}, under {}",
            crate::judge::percent(agreement),
            crate::judge::percent(needs)
        ));
    }
    if misses > 0 {
        return Err("not calibrated: it sorted a request as nothing-to-do".to_owned());
    }
    Ok(model)
}

/// Whether the feedback item in `data` was sorted `nothing-to-do` by a calibrated decision model.
pub(crate) fn settled(data: &Value) -> bool {
    data[DECIDED]["class"] == Class::NothingToDo.name()
}

/// The class a calibrated decision model gave the feedback item in `data`, if it gave one.
pub(crate) fn class(data: &Value) -> Option<&str> {
    data[DECIDED]["class"].as_str()
}

/// Before the daemon routes `message`: removes a mark it arrived with, then, when it is a feedback
/// item a task holds and the factory has a calibrated triage model, asks the model and marks the
/// item when the model sorts it `nothing-to-do` at the threshold. Every outcome but "no model" and
/// "not an item" appends a `factory.decision.answered` record.
pub(crate) fn triage(
    db: &mut Connection,
    trace: &Trace,
    message: &mut Message,
) -> rusqlite::Result<()> {
    if let Some(data) = message.data.as_object_mut() {
        data.remove(DECIDED);
    }
    let factory = trace.factory;
    let Some(decision) = factory.triage() else {
        return Ok(());
    };
    let Some(item) = crate::triage::item(&message.kind, &message.data).map(str::to_owned) else {
        return Ok(());
    };
    if crate::route::outsider(message) || !held(db, message)? {
        return Ok(());
    }
    let mut record = json!({ "point": TRIAGE, "item": item, "acted": false });
    let started = Instant::now();
    let answered = asked(db, factory, decision, message)?;
    record["millis"] = json!(started.elapsed().as_millis() as u64);
    match answered {
        Err(why) => record["why"] = json!(why),
        Ok((answer, calibrated)) => {
            record["model"] = json!(answer.model);
            record["choice"] = json!(answer.choice);
            record["probability"] = json!(answer.probability);
            record["input_tokens"] = json!(answer.input_tokens);
            let why = if answer.model != calibrated {
                Some(format!(
                    "answered by {}, calibrated on {calibrated}",
                    answer.model
                ))
            } else if answer.probability < decision.threshold() {
                Some(format!("under the threshold {}", decision.threshold()))
            } else if answer.choice != Class::NothingToDo.name() {
                Some(format!(
                    "Priory acts on nothing-to-do only, not {}",
                    answer.choice
                ))
            } else {
                None
            };
            match why {
                Some(why) => record["why"] = json!(why),
                None => {
                    record["acted"] = json!(true);
                    message.data[DECIDED] = json!({
                        "class": answer.choice,
                        "probability": answer.probability,
                        "model": answer.model,
                    });
                }
            }
        }
    }
    if let Value::Object(fields) = &mut record {
        fields.retain(|_, value| !value.is_null());
    }
    let tx = db.transaction()?;
    trace::record(&tx, trace, "factory.decision.answered", record)?;
    tx.commit()
}

/// The model's answer and the model it is calibrated on, or why Priory made no call or the call
/// failed.
fn asked(
    db: &Connection,
    factory: &Factory,
    decision: &Decision,
    message: &Message,
) -> rusqlite::Result<Result<(Answer, String), String>> {
    let calibrated = match calibrated(db, factory) {
        Ok(model) => model,
        Err(why) => return Ok(Err(why)),
    };
    let today: u32 = db.query_row(
        "SELECT count(*) FROM trace WHERE type = 'factory.decision.answered'
           AND json_extract(event, '$.data.model') IS NOT NULL
           AND date(time, 'localtime') = date('now', 'localtime')",
        [],
        |row| row.get(0),
    )?;
    if today >= decision.daily_calls() {
        return Ok(Err(format!(
            "the day's {} calls are spent",
            decision.daily_calls()
        )));
    }
    let data = &message.data;
    let state = json!({
        "kind": message.kind,
        "body": data["body"],
        "path": data["path"],
        "review_state": data["state"],
    });
    let state = match state {
        Value::Object(mut fields) => {
            fields.retain(|_, value| !value.is_null());
            Value::Object(fields)
        }
        state => state,
    };
    if state.to_string().len() > STATE_LIMIT {
        return Ok(Err(format!("the item is over {STATE_LIMIT} bytes")));
    }
    Ok(call(factory, decision, &question(state)).map(|answer| (answer, calibrated)))
}

/// Whether a task owns or subscribes to a resource `message` is about.
fn held(db: &Connection, message: &Message) -> rusqlite::Result<bool> {
    for resource in crate::route::resources(message) {
        let held: bool = db.query_row(
            "SELECT EXISTS (SELECT 1 FROM owns WHERE resource = ?1)
               OR EXISTS (SELECT 1 FROM subscriptions WHERE resource = ?1)",
            [&resource],
            |row| row.get(0),
        )?;
        if held {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    const OPTIONS: [&str; 3] = ["trivial", "nothing-to-do", "persons-call"];

    #[test]
    fn parse_reads_the_choice_and_its_probability() {
        let stdout = br#"{"model":"jev-1.14.0","answers":{"triage":{"type":"choice","choice":"trivial","probabilities":{"trivial":0.8,"persons-call":0.2}}},"usage":{"input_tokens":12}}"#;

        assert_eq!(
            parse(stdout, "triage", &OPTIONS),
            Ok(Answer {
                model: "jev-1.14.0".to_owned(),
                choice: "trivial".to_owned(),
                probability: 0.8,
                input_tokens: Some(12),
            })
        );
    }

    #[test]
    fn parse_refuses_an_answer_off_the_options_or_without_a_probability() {
        let off =
            br#"{"model":"m","answers":{"triage":{"choice":"maybe","probabilities":{"maybe":1}}}}"#;
        let none = br#"{"model":"m","answers":{"triage":{"choice":"trivial","probabilities":{}}}}"#;
        let over = br#"{"model":"m","answers":{"triage":{"choice":"trivial","probabilities":{"trivial":1.5}}}}"#;

        assert_eq!(
            parse(off, "triage", &OPTIONS),
            Err("the answer picks \"maybe\", not one of the options".to_owned())
        );
        assert!(parse(none, "triage", &OPTIONS).is_err());
        assert!(parse(over, "triage", &OPTIONS).is_err());
        assert_eq!(
            parse(b"{}", "triage", &OPTIONS),
            Err("the answer names no model".to_owned())
        );
    }

    proptest::proptest! {
        #[test]
        fn parse_reads_any_output_without_panicking(
            text in "[ -~\n]{0,200}",
            cut in 0usize..200,
            flip in proptest::collection::vec(proptest::prelude::any::<(usize, u8)>(), 0..8),
        ) {
            let _ = parse(text.as_bytes(), "triage", &OPTIONS);
            let good = br#"{"model":"m","answers":{"triage":{"choice":"trivial","probabilities":{"trivial":0.9}}}}"#.to_vec();
            let _ = parse(&good[..cut.min(good.len())], "triage", &OPTIONS);
            let mut mutated = good;
            for (at, byte) in flip {
                let len = mutated.len();
                mutated[at % len] = byte;
            }
            let _ = parse(&mutated, "triage", &OPTIONS);
        }
    }
}
