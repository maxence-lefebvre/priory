use crate::trace::Trace;
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use uuid::Uuid;

/// How many trace records one request carries at most.
const BATCH: i64 = 200;

/// Namespaces the ids derived from a record's source and task, run or record id.
const IDS: Uuid = Uuid::from_u128(0x5d0c_7a1e_3b8f_4e2a_9c61_0f4d_2b7e_8a93);

/// The records past `endpoint`'s cursor, at most a batch, as the seq of the last one and an OTLP/HTTP
/// JSON request. The request is `None` when none of them is a step record, so the cursor moves on
/// without a send. `None` overall when nothing is pending.
pub(crate) fn pending(
    db: &Connection,
    endpoint: &str,
    trace: &Trace,
) -> rusqlite::Result<Option<(i64, Option<Value>)>> {
    let mut statement = db.prepare(
        "SELECT trace.seq, trace.event, runs.agent,
         CAST(round(unixepoch(runs.started, 'subsec') * 1000) AS INTEGER),
         CAST(round(unixepoch(json_extract(trace.event, '$.data.started'), 'subsec') * 1000) AS INTEGER),
         CAST(round(unixepoch(trace.time, 'subsec') * 1000) AS INTEGER)
         FROM trace LEFT JOIN runs
           ON runs.id = CAST(json_extract(trace.event, '$.data.run') AS INTEGER)
         WHERE trace.seq > coalesce((SELECT seq FROM exports WHERE endpoint = ?1), 0)
         ORDER BY trace.seq LIMIT ?2",
    )?;
    let rows = statement.query_map(params![endpoint, BATCH], |row| {
        Ok(Row {
            seq: row.get(0)?,
            event: serde_json::from_str(&row.get::<_, String>(1)?).unwrap_or(Value::Null),
            agent: row.get(2)?,
            run_started: row.get(3)?,
            started: row.get(4)?,
            time: row.get(5)?,
        })
    })?;
    let mut last = None;
    let mut spans = Vec::new();
    for row in rows {
        let row = row?;
        last = Some(row.seq);
        if row.event["type"] != "factory.step.ended" {
            continue;
        }
        spans.extend(step_spans(&row));
    }
    let Some(last) = last else {
        return Ok(None);
    };
    if spans.is_empty() {
        return Ok(Some((last, None)));
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
    Ok(Some((last, Some(request))))
}

/// Records that `endpoint` accepted every record up to `seq`.
pub(crate) fn sent(db: &Connection, endpoint: &str, seq: i64) -> rusqlite::Result<()> {
    db.execute(
        "INSERT INTO exports (endpoint, seq) VALUES (?1, ?2)
         ON CONFLICT (endpoint) DO UPDATE SET seq = excluded.seq",
        params![endpoint, seq],
    )?;
    Ok(())
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
    if data["run"].is_null() {
        return vec![step];
    }
    let run_id = derive(source, "run", &data["run"]);
    step["parentSpanId"] = json!(span_id(&run_id));
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
    let run = span(
        &trace,
        &run_id,
        &name,
        run_start,
        time * 1_000_000,
        &run_attributes,
    );
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
fn attribute(key: &str, value: &Value) -> Option<Value> {
    let value = match value {
        Value::String(text) => json!({ "stringValue": text }),
        Value::Number(number) if number.is_i64() => json!({ "intValue": number.to_string() }),
        Value::Number(number) => json!({ "doubleValue": number }),
        Value::Bool(flag) => json!({ "boolValue": flag }),
        _ => return None,
    };
    Some(json!({ "key": key, "value": value }))
}
