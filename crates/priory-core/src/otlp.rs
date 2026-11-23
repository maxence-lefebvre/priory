use crate::trace::Trace;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use uuid::Uuid;

/// How many trace records one request carries at most.
const BATCH: i64 = 200;

/// Namespaces the ids derived from a record's source and task, run or record id.
const IDS: Uuid = Uuid::from_u128(0x5d0c_7a1e_3b8f_4e2a_9c61_0f4d_2b7e_8a93);

/// How far an endpoint has received: the last `trace.seq` and the last `task_ends.seq` it accepted.
#[derive(Clone, Copy, Default)]
pub(crate) struct Cursor {
    seq: i64,
    ended: i64,
}

/// What is pending past `endpoint`'s cursor, at most a batch of trace records and a batch of task
/// ends, as the cursor after them and an OTLP/HTTP JSON request. The request is `None` when none of
/// them makes a span, so the cursor moves on without a send. `None` overall when nothing is pending.
pub(crate) fn pending(
    db: &Connection,
    endpoint: &str,
    trace: &Trace,
) -> rusqlite::Result<Option<(Cursor, Option<Value>)>> {
    let mut cursor = db
        .query_row(
            "SELECT seq, ended FROM exports WHERE endpoint = ?1",
            [endpoint],
            |row| {
                Ok(Cursor {
                    seq: row.get(0)?,
                    ended: row.get(1)?,
                })
            },
        )
        .optional()?
        .unwrap_or_default();
    let mut statement = db.prepare(
        "SELECT trace.seq, trace.event, runs.agent,
         CAST(round(unixepoch(runs.started, 'subsec') * 1000) AS INTEGER),
         CAST(round(unixepoch(json_extract(trace.event, '$.data.started'), 'subsec') * 1000) AS INTEGER),
         CAST(round(unixepoch(trace.time, 'subsec') * 1000) AS INTEGER)
         FROM trace LEFT JOIN runs
           ON runs.id = CAST(json_extract(trace.event, '$.data.run') AS INTEGER)
         WHERE trace.seq > ?1
         ORDER BY trace.seq LIMIT ?2",
    )?;
    let rows = statement.query_map(params![cursor.seq, BATCH], |row| {
        Ok(Row {
            seq: row.get(0)?,
            event: serde_json::from_str(&row.get::<_, String>(1)?).unwrap_or(Value::Null),
            agent: row.get(2)?,
            run_started: row.get(3)?,
            started: row.get(4)?,
            time: row.get(5)?,
        })
    })?;
    let mut pending = false;
    let mut spans = Vec::new();
    for row in rows {
        let row = row?;
        pending = true;
        cursor.seq = row.seq;
        match row.event["type"].as_str() {
            Some("factory.step.ended") => spans.extend(step_spans(&row)),
            Some(crate::reload::RELOADED | crate::reload::REFUSED) => {
                spans.extend(reload_span(&row))
            }
            _ => {}
        }
    }
    let source = crate::trace::source(trace);
    let mut ends = db.prepare(
        "SELECT task_ends.seq, task_ends.task, task_ends.generation, task_ends.state,
         CAST(round(unixepoch(task_ends.created, 'subsec') * 1000) AS INTEGER),
         CAST(round(unixepoch(task_ends.ended, 'subsec') * 1000) AS INTEGER),
         tasks.key, tasks.workflow, tasks.lane, tasks.band, tasks.grade,
         task_ends.run,
         coalesce((SELECT max(earlier.run) FROM task_ends earlier
           WHERE earlier.task = task_ends.task AND earlier.seq < task_ends.seq), 0)
         FROM task_ends JOIN tasks ON tasks.id = task_ends.task
         WHERE task_ends.seq > ?1 ORDER BY task_ends.seq LIMIT ?2",
    )?;
    let ends = ends.query_map(params![cursor.ended, BATCH], |row| {
        Ok(End {
            seq: row.get(0)?,
            task: row.get(1)?,
            generation: row.get(2)?,
            state: row.get(3)?,
            created: row.get(4)?,
            ended: row.get(5)?,
            key: row.get(6)?,
            workflow: row.get(7)?,
            lane: row.get(8)?,
            band: row.get(9)?,
            grade: row.get(10)?,
            last_run: row.get(11)?,
            first_run: row.get(12)?,
        })
    })?;
    for end in ends {
        let end = end?;
        pending = true;
        cursor.ended = end.seq;
        spans.extend(end_spans(db, &source, &end)?);
    }
    if !pending {
        return Ok(None);
    }
    if spans.is_empty() {
        return Ok(Some((cursor, None)));
    }
    let resource: Vec<Value> = [
        attribute("service.name", &json!("priory")),
        attribute("priory.factory", &json!(trace.factory.name)),
        attribute("host.name", &json!(trace.machine)),
    ]
    .into_iter()
    .flatten()
    .collect();
    let request = json!({ "resourceSpans": [{
        "resource": { "attributes": resource },
        "scopeSpans": [{ "scope": { "name": "priory" }, "spans": spans }],
    }] });
    Ok(Some((cursor, Some(request))))
}

/// Records that `endpoint` accepted every record up to `cursor`.
pub(crate) fn sent(db: &Connection, endpoint: &str, cursor: Cursor) -> rusqlite::Result<()> {
    db.execute(
        "INSERT INTO exports (endpoint, seq, ended) VALUES (?1, ?2, ?3)
         ON CONFLICT (endpoint) DO UPDATE SET seq = excluded.seq, ended = excluded.ended",
        params![endpoint, cursor.seq, cursor.ended],
    )?;
    Ok(())
}

/// A task's end: when its generation began and ended, and what it was by then.
struct End {
    seq: i64,
    task: i64,
    generation: i64,
    state: String,
    created: Option<i64>,
    ended: Option<i64>,
    key: String,
    workflow: String,
    lane: Option<String>,
    band: Option<String>,
    grade: Option<String>,
    first_run: i64,
    last_run: i64,
}

/// The task's root span, and a span for each of its runs that ended with no step, under it.
fn end_spans(db: &Connection, source: &str, end: &End) -> rusqlite::Result<Vec<Value>> {
    let (Some(created), Some(ended)) = (end.created, end.ended) else {
        return Ok(Vec::new());
    };
    let task = json!(end.task.to_string());
    let trace = derive(source, "task", &task).simple().to_string();
    let root = root_id(source, &task, end.generation);
    let attributes = [
        ("priory.task.id", &task),
        ("priory.key", &json!(end.key)),
        ("priory.workflow", &json!(end.workflow)),
        ("priory.lane", &json!(end.lane)),
        ("priory.band", &json!(end.band)),
        ("priory.grade", &json!(end.grade)),
        ("priory.state", &json!(end.state)),
        ("priory.generation", &json!(end.generation)),
    ];
    let mut spans = vec![span(
        &trace,
        &root,
        &format!("task {}", end.key),
        created * 1_000_000,
        ended * 1_000_000,
        &attributes,
    )];
    let mut runs = db.prepare(
        "SELECT runs.id, runs.step, runs.agent, runs.ended_as,
         CAST(round(unixepoch(runs.started, 'subsec') * 1000) AS INTEGER),
         CAST(round(unixepoch(runs.ended, 'subsec') * 1000) AS INTEGER)
         FROM runs
         WHERE runs.task = ?1 AND runs.ended IS NOT NULL
           AND runs.id > ?2 AND runs.id <= ?3
           AND NOT EXISTS (SELECT 1 FROM trace WHERE trace.type = 'factory.step.ended'
             AND json_extract(trace.event, '$.data.run') = CAST(runs.id AS TEXT))
         ORDER BY runs.id",
    )?;
    let runs = runs.query_map(params![end.task, end.first_run, end.last_run], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, Option<i64>>(4)?,
            row.get::<_, Option<i64>>(5)?,
        ))
    })?;
    for run in runs {
        let (id, step, agent, ended_as, started, run_ended) = run?;
        let (Some(started), Some(run_ended)) = (started, run_ended) else {
            continue;
        };
        let id = json!(id.to_string());
        let attributes = [
            ("priory.task.id", &task),
            ("priory.run.id", &id),
            ("priory.step", &json!(step)),
            ("priory.lane", &json!(end.lane)),
            ("priory.run.ended_as", &json!(ended_as)),
            ("gen_ai.operation.name", &json!("invoke_agent")),
            ("gen_ai.agent.name", &json!(agent)),
        ];
        let name = match &agent {
            Some(agent) => format!("invoke_agent {agent}"),
            None => "invoke_agent".to_owned(),
        };
        let mut run = span(
            &trace,
            &derive(source, "run", &id),
            &name,
            started * 1_000_000,
            run_ended * 1_000_000,
            &attributes,
        );
        run["parentSpanId"] = json!(span_id(&root));
        spans.push(run);
    }
    Ok(spans)
}

/// The id of the root span of `task`'s `generation`, which each run span of the generation names.
fn root_id(source: &str, task: &Value, generation: i64) -> Uuid {
    derive(
        source,
        "root",
        &json!(format!(
            "{}/{generation}",
            task.as_str().unwrap_or_default()
        )),
    )
}

/// A trace record with what the spans need from its run.
struct Row {
    seq: i64,
    event: Value,
    agent: Option<String>,
    run_started: Option<i64>,
    started: Option<i64>,
    time: Option<i64>,
}

/// A reload's span: an instant in a trace of its own, named after its record.
fn reload_span(row: &Row) -> Option<Value> {
    let data = &row.event["data"];
    let source = row.event["source"].as_str().unwrap_or_default();
    let time = row.time? * 1_000_000;
    let id = derive(source, "reload", &row.event["id"]);
    let lines = |key: &str| {
        data[key]
            .as_array()
            .filter(|lines| !lines.is_empty())
            .map(|lines| {
                let lines: Vec<_> = lines.iter().filter_map(Value::as_str).collect();
                json!(lines.join("\n"))
            })
            .unwrap_or(Value::Null)
    };
    let attributes = [
        ("priory.factory.revision", &data["factory_revision"]),
        ("priory.reload.restart_only", &lines("restart_only")),
        ("priory.reload.problems", &lines("problems")),
    ];
    let name = row.event["type"].as_str().unwrap_or_default();
    Some(span(
        &id.simple().to_string(),
        &id,
        name,
        time,
        time,
        &attributes,
    ))
}

/// The step's span and, for a model step, its run's span, which is the step span's parent.
fn step_spans(row: &Row) -> Vec<Value> {
    let data = &row.event["data"];
    let source = row.event["source"].as_str().unwrap_or_default();
    let trace = derive(source, "task", &data["task"]).simple().to_string();
    let (Some(started), Some(time)) = (row.started, row.time) else {
        return Vec::new();
    };
    let start = started * 1_000_000;
    let end = start + data["duration_ms"].as_i64().unwrap_or(0) * 1_000_000;
    let tokens = &data["tokens"];
    // GenAI conventions count cached input in input_tokens, and break it out separately.
    let input = tokens["input"].as_i64().map(|input| {
        json!(
            input
                + tokens["cache_read"].as_i64().unwrap_or(0)
                + tokens["cache_write"].as_i64().unwrap_or(0)
        )
    });
    let input = input.unwrap_or(Value::Null);
    let step_attributes = [
        ("priory.task.id", &data["task"]),
        ("priory.run.id", &data["run"]),
        ("priory.step", &data["step"]),
        ("priory.outcome", &data["outcome"]),
        ("priory.grade", &data["grade"]),
        ("priory.lane", &data["lane"]),
        ("priory.workflow", &data["workflow"]),
        ("priory.key", &data["key"]),
        ("priory.generation", &data["generation"]),
        ("priory.decider", &data["decider"]),
        ("priory.effect", &data["effect"]),
        ("priory.harness", &data["harness"]),
        ("gen_ai.request.model", &data["model"]),
        ("gen_ai.usage.input_tokens", &input),
        ("gen_ai.usage.output_tokens", &tokens["output"]),
        (
            "gen_ai.usage.cache_read.input_tokens",
            &tokens["cache_read"],
        ),
        (
            "gen_ai.usage.cache_creation.input_tokens",
            &tokens["cache_write"],
        ),
    ];
    let mut step = span(
        &trace,
        &derive(source, "step", &row.event["id"]),
        data["step"].as_str().unwrap_or_default(),
        start,
        end,
        &step_attributes,
    );
    let root = root_id(
        source,
        &data["task"],
        data["generation"].as_i64().unwrap_or(1),
    );
    if data["run"].is_null() {
        step["parentSpanId"] = json!(span_id(&root));
        return vec![step];
    }
    let run_id = derive(source, "run", &data["run"]);
    step["parentSpanId"] = json!(span_id(&run_id));
    let root = root_id(
        source,
        &data["task"],
        data["generation"].as_i64().unwrap_or(1),
    );
    let agent = json!(row.agent);
    let run_attributes = [
        ("priory.task.id", &data["task"]),
        ("priory.run.id", &data["run"]),
        ("priory.step", &data["step"]),
        ("priory.lane", &data["lane"]),
        ("gen_ai.operation.name", &json!("invoke_agent")),
        ("gen_ai.agent.name", &agent),
        ("gen_ai.request.model", &data["model"]),
    ];
    let name = match &row.agent {
        Some(agent) => format!("invoke_agent {agent}"),
        None => "invoke_agent".to_owned(),
    };
    let run_start = row.run_started.map_or(start, |started| started * 1_000_000);
    let mut run = span(
        &trace,
        &run_id,
        &name,
        run_start,
        time * 1_000_000,
        &run_attributes,
    );
    run["parentSpanId"] = json!(span_id(&root));
    vec![run, step]
}

fn span(
    trace: &str,
    id: &Uuid,
    name: &str,
    start: i64,
    end: i64,
    attributes: &[(&str, &Value)],
) -> Value {
    let attributes: Vec<Value> = attributes
        .iter()
        .filter_map(|(key, value)| attribute(key, value))
        .collect();
    json!({
        "traceId": trace,
        "spanId": span_id(id),
        "name": name,
        "kind": 1,
        "startTimeUnixNano": start.to_string(),
        "endTimeUnixNano": end.max(start).to_string(),
        "attributes": attributes,
    })
}

/// A stable id for `kind` `id` of the factory and machine `source`, so a resend repeats it.
fn derive(source: &str, kind: &str, id: &Value) -> Uuid {
    let id = id.as_str().map_or_else(|| id.to_string(), str::to_owned);
    Uuid::new_v5(&IDS, format!("{source}/{kind}/{id}").as_bytes())
}

fn span_id(id: &Uuid) -> String {
    id.simple().to_string()[..16].to_owned()
}

/// An OTLP attribute, or none for a missing value.
pub(crate) fn attribute(key: &str, value: &Value) -> Option<Value> {
    let value = match value {
        Value::String(text) => json!({ "stringValue": text }),
        Value::Number(number) if number.is_i64() => json!({ "intValue": number.to_string() }),
        Value::Number(number) => json!({ "doubleValue": number }),
        Value::Bool(flag) => json!({ "boolValue": flag }),
        _ => return None,
    };
    Some(json!({ "key": key, "value": value }))
}

/// Writes the OTLP headers from `[telemetry] headers_env` to the file `name` in `home`, readable by
/// this user only, and returns its path. Refuses an unset, empty or malformed variable.
pub(crate) fn header_file(
    factory: &priory_factory::factory::Factory,
    home: &std::path::Path,
    name: &str,
) -> Result<Option<std::path::PathBuf>, String> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let Some(variable) = factory.otlp_headers_env() else {
        return Ok(None);
    };
    let text = crate::serve::env(variable, "the OTLP headers, as key1=value1,key2=value2")?;
    let lines = header_lines(&text).map_err(|error| format!("{variable}: {error}"))?;
    let file = home.join(name);
    // A file left by an older process keeps its mode, so it is replaced rather than truncated.
    let _ = std::fs::remove_file(&file);
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&file)
        .and_then(|mut out| out.write_all(lines.as_bytes()))
        .map_err(|error| format!("{}: {error}", file.display()))?;
    Ok(Some(file))
}

/// The header lines `curl --header @file` reads, from OTLP's `key1=value1,key2=value2` with
/// URL-encoded values. Its errors never quote the text, which holds credentials.
pub(crate) fn header_lines(text: &str) -> Result<String, String> {
    let refused = "OTLP headers need the form key1=value1,key2=value2 with URL-encoded values";
    let mut lines = String::new();
    for pair in text.split(',').filter(|pair| !pair.trim().is_empty()) {
        let (key, value) = pair.split_once('=').ok_or(refused)?;
        let key = key.trim();
        let value = decode(value.trim()).ok_or(refused)?;
        let token = |byte: u8| byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte);
        let control = |c: char| c.is_ascii_control() && c != '\t';
        if key.is_empty() || !key.bytes().all(token) || value.contains(control) {
            return Err(refused.to_owned());
        }
        lines.push_str(&format!("{key}: {value}\n"));
    }
    if lines.is_empty() {
        return Err(refused.to_owned());
    }
    Ok(lines)
}

/// `text` with each `%XX` replaced by its byte, or none for a broken escape or invalid UTF-8.
fn decode(text: &str) -> Option<String> {
    let mut bytes = Vec::with_capacity(text.len());
    let mut rest = text.as_bytes();
    while let Some((&byte, tail)) = rest.split_first() {
        rest = tail;
        if byte != b'%' {
            bytes.push(byte);
            continue;
        }
        let hex = rest
            .get(..2)
            .filter(|hex| hex.iter().all(u8::is_ascii_hexdigit))?;
        bytes.push(u8::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok()?);
        rest = &rest[2..];
    }
    String::from_utf8(bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn pairs_become_header_lines_with_their_values_decoded() {
        assert_eq!(
            header_lines(" x-honeycomb-team = a%2Cb ,authorization=Basic%20dTpw,").unwrap(),
            "x-honeycomb-team: a,b\nauthorization: Basic dTpw\n"
        );
    }

    #[test]
    fn a_pair_that_would_break_the_header_file_is_refused_without_quoting_it() {
        for text in [
            "secret",
            "=secret",
            "a b=secret",
            "x/secret=1",
            "k=secret%0Ax-evil: 1",
            "k=secret%01",
            "k=secret%2",
            " , ",
        ] {
            let error = header_lines(text).unwrap_err();
            assert!(!error.contains("secret"), "{text}: {error}");
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(512))]

        #[test]
        fn text_of_any_shape_gives_one_line_per_header_or_is_refused(text in "\\PC{0,200}") {
            if let Ok(lines) = header_lines(&text) {
                prop_assert!(lines.lines().all(|line| line.contains(": ")));
                prop_assert!(!lines.contains('\r'));
            }
        }
    }
}
