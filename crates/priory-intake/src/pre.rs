//! A trigger's `pre` script: it runs on each matching event before the intake publishes the task
//! request, and passes, changes or drops it. See `docs/design/intake-scripts.md`.

use herdr_bus::Message;
use priory_factory::factory::{OnError, Pre, Trigger};
use priory_runtime::script::{self, Bounded, Ended};
use serde_json::{Map, Value, json};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The record of a request that `pre` dropped.
pub const DROPPED: &str = "factory.intake.dropped";
/// The most a script may print on each of stdout and stderr.
pub const CAP: usize = 64 * 1024;
const TITLE_MAX: usize = 200;
const TEXT_MAX: usize = 16 * 1024;
const REASON_MAX: usize = 500;
/// The fields a script may print, in the order `pre.changed` lists them.
const FIELDS: [&str; 6] = ["title", "text", "context", "band", "lane", "key"];
/// The fields of an event's data that hold its request text, in the order a task reads them.
const TEXT_FIELDS: [&str; 4] = ["text", "body", "description", "prompt"];

/// Records the outcome of a script that ran: the failure it reports, or none to mark the last one
/// read.
pub type Report = Box<dyn Fn(&str, Option<&str>) + Send + Sync>;
/// Stores a request whose script runs later, in the order stored.
pub type Store = Box<dyn Fn(&Pending) -> Result<(), String> + Send + Sync>;

/// What the process that builds requests needs to run `pre` scripts.
pub struct Context {
    /// The factory folder, where scripts run.
    pub dir: PathBuf,
    /// The factory's bands, which a script's `band` must name.
    pub bands: Vec<String>,
    pub report: Report,
    /// Set in the server, which replies to a webhook before its script ends: requests wait there.
    pub store: Option<Store>,
}

static CONTEXT: OnceLock<Context> = OnceLock::new();

/// Sets how this process runs `pre` scripts. The first call wins.
pub fn configure(context: Context) {
    let _ = CONTEXT.set(context);
}

/// A request whose `pre` script has not run yet, as the server stores it.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Pending {
    pub trigger: String,
    pub source: Message,
    pub request: Message,
}

/// What one script decided for one request: the message to publish, and its failure, if it failed.
#[derive(Debug)]
pub struct Decided {
    pub message: Message,
    pub failure: Option<String>,
}

impl Decided {
    pub fn dropped(&self) -> bool {
        self.message.kind == DROPPED
    }
}

/// The request to publish for `request`, which trigger `name` built from `source`: itself when the
/// trigger has no `pre`, else what its script decides, or none when the server stores it to run
/// the script later.
pub(crate) fn apply(
    name: &str,
    trigger: &Trigger,
    source: &Message,
    request: Message,
) -> Result<Option<Message>, String> {
    let Some(pre) = &trigger.pre else {
        return Ok(Some(request));
    };
    let context = CONTEXT
        .get()
        .ok_or_else(|| format!("trigger {name} has a pre script this process cannot run"))?;
    if let Some(store) = &context.store {
        store(&Pending {
            trigger: name.to_owned(),
            source: source.clone(),
            request,
        })?;
        return Ok(None);
    }
    Ok(Some(run(context, name, pre, source, request)))
}

/// Runs the script of a stored request, from the server's queue.
pub fn run_pending(pending: &Pending, trigger: &Trigger) -> Result<Message, String> {
    let context = CONTEXT.get().ok_or("this process cannot run pre scripts")?;
    let Some(pre) = &trigger.pre else {
        return Ok(pending.request.clone());
    };
    Ok(run(
        context,
        &pending.trigger,
        pre,
        &pending.source,
        pending.request.clone(),
    ))
}

fn run(context: &Context, name: &str, pre: &Pre, source: &Message, request: Message) -> Message {
    let decided = decide(&context.dir, &context.bands, name, pre, source, request);
    (context.report)(name, decided.failure.as_deref());
    decided.message
}

/// Runs `pre` on `source` and `request`, in `dir`, and returns what it decided. It publishes and
/// records nothing.
pub fn decide(
    dir: &Path,
    bands: &[String],
    name: &str,
    pre: &Pre,
    source: &Message,
    request: Message,
) -> Decided {
    let input = json!({
        "event": {
            "id": source.id.to_string(),
            "type": source.kind,
            "subject": source.subject,
            "time": source.time.to_string(),
            "data": source.data,
        },
        "request": {
            "workflow": request.data["workflow"],
            "key": request.data["key"],
            "band": request.data["band"],
            "lane": request.data["lane"],
            "trigger": name,
        },
    });
    let ran = script::run_bounded(
        &pre.argv,
        &environment(dir, name, pre),
        dir,
        input.to_string().as_bytes(),
        pre.timeout(),
        CAP,
    );
    judge(bands, name, pre, source, request, &ran)
}

/// The short list of variables a script gets: `PATH`, `LANG`, `HOME`, `PRIORY_FACTORY`,
/// `PRIORY_TRIGGER` and the names in `env`, each copied when this process has it.
fn environment(dir: &Path, name: &str, pre: &Pre) -> Vec<(String, OsString)> {
    let copied = ["PATH", "LANG", "HOME"]
        .into_iter()
        .map(str::to_owned)
        .chain(pre.env.iter().cloned())
        .filter_map(|name| std::env::var_os(&name).map(|value| (name, value)));
    copied
        .chain([
            ("PRIORY_FACTORY".to_owned(), dir.as_os_str().to_owned()),
            ("PRIORY_TRIGGER".to_owned(), OsString::from(name)),
        ])
        .collect()
}

/// The message `ran` decides for `request`.
fn judge(
    bands: &[String],
    name: &str,
    pre: &Pre,
    source: &Message,
    mut request: Message,
    ran: &Bounded,
) -> Decided {
    let script = pre.argv.first().cloned().unwrap_or_default();
    let seconds = (ran.seconds * 1000.0).round() / 1000.0;
    let outcome = match &ran.ended {
        Ended::Exit(0) => read(&ran.stdout).and_then(|changes| check(bands, changes)),
        Ended::Exit(1) => {
            let reason = String::from_utf8_lossy(&ran.stdout)
                .lines()
                .next()
                .map(|line| line.trim().chars().take(REASON_MAX).collect::<String>())
                .filter(|line| !line.is_empty());
            return Decided {
                message: dropped(
                    name,
                    &script,
                    source,
                    &request,
                    Some(1),
                    reason,
                    None,
                    seconds,
                ),
                failure: None,
            };
        }
        ended => Err(ended.to_string()),
    };
    match outcome {
        Ok(changes) => {
            let pre = changed(&mut request, source, &script, changes, seconds);
            request.data["pre"] = pre;
            Decided {
                message: request,
                failure: None,
            }
        }
        Err(why) => {
            let failed = match ran.stderr.as_str() {
                "" => why,
                tail => format!("{why}: {tail}"),
            };
            let report = format!("pre of trigger {name}: {failed}");
            let exit = match ran.ended {
                Ended::Exit(code) => Some(code),
                _ => None,
            };
            let message = match pre.on_error {
                OnError::Pass => {
                    request.data["pre"] =
                        json!({ "script": script, "seconds": seconds, "failed": failed });
                    request
                }
                OnError::Drop => dropped(
                    name,
                    &script,
                    source,
                    &request,
                    exit,
                    None,
                    Some(failed),
                    seconds,
                ),
            };
            Decided {
                message,
                failure: Some(report),
            }
        }
    }
}

/// A source event as the log holds it, such as `priory inbox --json` or the log lists it.
pub fn event(text: &str) -> Result<Message, serde_json::Error> {
    serde_json::from_str(text)
}

/// The fields a script printed, each checked.
pub type Changes = Map<String, Value>;

/// The object a script printed, or none for nothing; refuses anything else.
pub fn read(stdout: &[u8]) -> Result<Changes, String> {
    let text = std::str::from_utf8(stdout).map_err(|_| "stdout is not UTF-8".to_owned())?;
    if text.trim().is_empty() {
        return Ok(Changes::new());
    }
    let value: Value =
        serde_json::from_str(text).map_err(|error| format!("stdout is not JSON: {error}"))?;
    let Value::Object(changes) = value else {
        return Err("stdout is not a JSON object".to_owned());
    };
    if let Some(unknown) = changes.keys().find(|key| !FIELDS.contains(&key.as_str())) {
        return Err(format!(
            "stdout names `{unknown}`, which a pre script cannot set"
        ));
    }
    if let Some((field, _)) = changes.iter().find(|(_, value)| !value.is_string()) {
        return Err(format!("`{field}` is not a string"));
    }
    Ok(changes)
}

/// Refuses a field whose value breaks its rule.
fn check(bands: &[String], changes: Changes) -> Result<Changes, String> {
    let text = |field: &str| changes.get(field).and_then(Value::as_str);
    if let Some(title) = text("title")
        && (title.contains(['\n', '\r']) || title.chars().count() > TITLE_MAX)
    {
        return Err(format!(
            "`title` must be one line of at most {TITLE_MAX} characters"
        ));
    }
    for field in ["text", "context"] {
        if text(field).is_some_and(|text| text.len() > TEXT_MAX) {
            return Err(format!("`{field}` is over 16 KiB"));
        }
    }
    if let Some(band) = text("band")
        && !bands.iter().any(|known| known == band)
    {
        return Err(format!(
            "band `{band}` is not one of the factory's: {}",
            bands.join(", ")
        ));
    }
    if let Some(lane) = text("lane")
        && !priory_factory::lanes::well_formed(lane)
    {
        return Err(format!("lane `{lane}` is not `<team>/<theme>`"));
    }
    if let Some(key) = text("key")
        && (key.trim().is_empty() || key.contains(['\n', '\r']) || key.len() > TITLE_MAX)
    {
        return Err(format!(
            "`key` must be one line of 1 to {TITLE_MAX} characters"
        ));
    }
    Ok(changes)
}

/// Applies `changes` to `request` and returns its `pre` object: the script's name alone when it
/// changed nothing.
fn changed(
    request: &mut Message,
    source: &Message,
    script: &str,
    changes: Changes,
    seconds: f64,
) -> Value {
    let mut pre = json!({ "script": script });
    let mut names = Vec::new();
    let data = &source.data;
    let event_text = TEXT_FIELDS
        .iter()
        .find_map(|field| data[*field].as_str())
        .map(str::to_owned);
    for field in FIELDS {
        let Some(new) = changes.get(field).and_then(Value::as_str) else {
            continue;
        };
        match field {
            "title" | "text" | "context" => {
                let old = match field {
                    "title" => data["title"].as_str().map(str::to_owned),
                    "text" => event_text.clone(),
                    _ => None,
                };
                if old.as_deref() == Some(new) {
                    continue;
                }
                let length = |text: Option<&str>| text.map(|text| text.chars().count());
                pre[field] = json!({ "from": length(old.as_deref()), "to": length(Some(new)) });
                request.data[field] = json!(new);
            }
            _ => {
                let old = request.data[field].clone();
                let new = match field {
                    // The source's scope, so a script cannot name another source's task.
                    "key" => {
                        let key = old.as_str().unwrap_or_default();
                        let scope = key.split(':').next().unwrap_or_default();
                        json!(format!("{scope}:{new}"))
                    }
                    _ => json!(new),
                };
                if old == new {
                    continue;
                }
                pre[field] = json!({ "from": old, "to": new });
                request.data[field] = new.clone();
                if field == "key" {
                    request.subject = new.as_str().map(str::to_owned);
                }
            }
        }
        names.push(field);
    }
    if !names.is_empty() {
        pre["changed"] = json!(names);
        pre["seconds"] = json!(seconds);
    }
    pre
}

/// The record of a dropped request: it takes the request's id, so one decision stands per event
/// and trigger.
#[allow(clippy::too_many_arguments)]
fn dropped(
    name: &str,
    script: &str,
    source: &Message,
    request: &Message,
    exit: Option<i32>,
    reason: Option<String>,
    failed: Option<String>,
    seconds: f64,
) -> Message {
    let mut data = json!({
        "trigger": name,
        "source": source.id.to_string(),
        "key": request.data["key"],
        "script": script,
        "exit": exit,
        "reason": reason,
        "seconds": seconds,
    });
    if let Some(failed) = failed {
        data["failed"] = json!(failed);
    }
    let mut message = Message::new(DROPPED, data)
        .id(request.id)
        .time(request.time);
    message.subject = request.subject.clone();
    message
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn nothing_printed_changes_nothing_and_an_object_names_its_fields() {
        assert!(read(b"  \n").unwrap().is_empty());
        let changes = read(br#"{"band": "today", "title": "T"}"#).unwrap();
        assert_eq!(changes.len(), 2);
    }

    #[test]
    fn a_field_a_script_cannot_set_or_a_value_that_is_not_a_string_is_refused() {
        for refused in [
            &br#"{"workflow": "x"}"#[..],
            br#"{"band": 3}"#,
            br#"[1]"#,
            b"not json",
            b"\xff\xfe",
        ] {
            assert!(
                read(refused).is_err(),
                "{}",
                String::from_utf8_lossy(refused)
            );
        }
    }

    proptest! {
        #[test]
        fn the_reader_returns_or_refuses_random_and_mutated_json(
            value in crate::intake::fuzz::json_value(),
            cut in 0usize..64,
            noise in proptest::collection::vec(any::<u8>(), 0..32),
        ) {
            let text = value.to_string();
            let _ = read(text.as_bytes()).and_then(|changes| check(&["today".into()], changes));
            let mut mutated = text.into_bytes();
            mutated.truncate(mutated.len().saturating_sub(cut));
            mutated.extend(noise);
            let _ = read(&mutated).and_then(|changes| check(&["today".into()], changes));
        }

        #[test]
        fn the_reader_returns_or_refuses_objects_of_the_fields_it_knows(
            fields in proptest::collection::vec(
                (proptest::sample::select(FIELDS.to_vec()), "\\PC{0,40}|.{0,300}"),
                0..6,
            ),
        ) {
            let object: Map<String, Value> = fields
                .into_iter()
                .map(|(field, value)| (field.to_owned(), json!(value)))
                .collect();
            let text = Value::Object(object).to_string();
            let _ = read(text.as_bytes()).and_then(|changes| check(&["today".into()], changes));
        }
    }
}
