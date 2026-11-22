use super::webhook::opened;
use super::{id, output, publish_new};
use crate::log::{self, Log};
use herdr_bus::{Message, Timestamp};
use priory_factory::factory::{EventLog, Factory, Trigger};
use priory_runtime::http::{self, Reply, Request};
use serde_json::{Value, json};

pub(super) const COMMENTED: &str = "figma.file.comment_created";

/// Figma v2 webhook deliveries to `POST /hooks/figma`: checked against the webhook's passcode
/// before anything else reads them, then published on `factory.intake`.
pub struct FigmaWebhook {
    passcode: String,
    /// The `figma` triggers, which a delivery can match.
    triggers: Vec<(String, Trigger)>,
    /// The log a delivery publishes on, opened again while a worker's open failed.
    log: EventLog,
}

impl FigmaWebhook {
    /// Declares `factory.intake`, so a delivery can publish before the intake ever ran.
    pub fn new(passcode: String, factory: &Factory) -> Result<Self, String> {
        let bus = log::open(factory.log()).map_err(|error| error.to_string())?;
        bus.declare(&output()).map_err(|error| error.to_string())?;
        let triggers = factory
            .triggers
            .iter()
            .filter(|(_, trigger)| trigger.value.source == "figma")
            .map(|(name, trigger)| (name.clone(), trigger.value.clone()))
            .collect();
        Ok(FigmaWebhook {
            passcode,
            triggers,
            log: factory.log().clone(),
        })
    }

    /// Verifies, maps and publishes one delivery. Figma counts any reply but `200` as a failure and
    /// retries it, so every delivery Priory takes or drops on purpose gets `200`. Figma's docs ask
    /// for `400` on a wrong passcode, which stops the webhook.
    pub fn receive(&self, bus: &mut Result<Box<dyn Log>, String>, request: &Request) -> Reply {
        let payload = serde_json::from_slice::<Value>(request.body).unwrap_or(Value::Null);
        let passcode = payload["passcode"].as_str().map(str::as_bytes);
        if !passcode.is_some_and(|passcode| http::same(passcode, self.passcode.as_bytes())) {
            return Reply::error(
                400,
                "the body is not JSON, or its passcode is missing or is not the webhook's",
            );
        }
        let triggers: Vec<(&str, &Trigger)> = self
            .triggers
            .iter()
            .map(|(name, trigger)| (name.as_str(), trigger))
            .collect();
        let message = match observe(&triggers, &payload) {
            Ok(Some(message)) => message,
            Ok(None) => return ok("dropped"),
            Err(error) => return Reply::error(400, &error),
        };
        let bus = match opened(bus, || log::open(&self.log)) {
            Ok(bus) => bus,
            Err(error) => return Reply::error(500, &error),
        };
        match publish_new(bus, &triggers, &message) {
            Ok(true) => ok("published"),
            Ok(false) => ok("dropped"),
            Err(error) => Reply::error(500, &error),
        }
    }
}

/// The message a delivery reports, or none for `PING`, another event type, or a comment no trigger
/// matches. Errs when a field the event needs is missing.
fn observe(triggers: &[(&str, &Trigger)], payload: &Value) -> Result<Option<Message>, String> {
    if payload["event_type"] != "FILE_COMMENT" {
        return Ok(None);
    }
    let text = |value: &Value| {
        value
            .as_str()
            .filter(|text| !text.is_empty())
            .map(str::to_owned)
    };
    // Figma documents its ids as numbers, and its examples send strings.
    let number = |value: &Value| text(value).or_else(|| value.as_u64().map(|id| id.to_string()));
    let mentions: Vec<&Value> = payload["mentions"]
        .as_array()
        .into_iter()
        .flatten()
        .collect();
    let handle = |id: &Value| {
        mentions
            .iter()
            .find(|mention| mention["id"] == *id)
            .and_then(|mention| text(&mention["handle"]))
            .or_else(|| number(id))
            .unwrap_or_default()
    };
    let words: String = payload["comment"]
        .as_array()
        .into_iter()
        .flatten()
        .map(
            |fragment| match (fragment["text"].as_str(), &fragment["mention"]) {
                (Some(words), _) => words.to_owned(),
                (None, Value::Null) => String::new(),
                (None, mention) => format!("@{} ", handle(mention)),
            },
        )
        .collect();
    let file = text(&payload["file_key"])
        .filter(|key| key.bytes().all(|byte| byte.is_ascii_alphanumeric()));
    let mut data = json!({
        "file": file,
        "file_name": text(&payload["file_name"]),
        "comment": number(&payload["comment_id"]),
        "text": words.trim_end(),
        "parent": number(&payload["parent_id"]),
        "author": text(&payload["triggered_by"]["handle"]),
        "mentions": mentions.iter().filter_map(|mention| text(&mention["handle"])).collect::<Vec<_>>(),
        "url": file.as_ref().map(|key| format!("https://www.figma.com/file/{key}")),
        "webhook": number(&payload["webhook_id"]),
        "time": text(&payload["created_at"]).or_else(|| text(&payload["timestamp"])),
        "baseline": false,
    });
    let required = [
        ("file", "file_key"),
        ("comment", "comment_id"),
        ("time", "created_at"),
    ];
    if let Some((_, field)) = required.iter().find(|(key, _)| data[*key].is_null()) {
        return Err(format!("the payload lacks a valid `{field}`"));
    }
    let raw = data["time"].as_str().unwrap_or_default();
    let time: Timestamp = raw
        .parse()
        .map_err(|error| format!("Figma time `{raw}`: {error}"))?;
    let matched = matching(triggers, &data);
    if matched.is_empty() {
        return Ok(None);
    }
    data["triggers"] = json!(matched);
    let field = |key: &str| data[key].as_str().unwrap_or_default().to_owned();
    // A retry carries the same comment id, whatever else it changes.
    let name = format!("figma:comment:{}", field("comment"));
    let subject = text(&data["file_name"]).unwrap_or_else(|| field("file"));
    let url = field("url");
    Ok(Some(
        Message::new(COMMENTED, data)
            .id(id(&name))
            .subject(subject)
            .time(time)
            .link("file", &url),
    ))
}

/// The `figma` triggers whose `file` filter the comment meets.
fn matching<'a>(triggers: &[(&'a str, &Trigger)], data: &Value) -> Vec<&'a str> {
    triggers
        .iter()
        .filter(|(_, trigger)| {
            format!("figma.{}", trigger.on) == COMMENTED
                && trigger
                    .filter
                    .iter()
                    .flatten()
                    .all(|(field, value)| field == "file" && data["file"] == *value)
        })
        .map(|(name, _)| *name)
        .collect()
}

fn ok(outcome: &str) -> Reply {
    Reply::json(200, json!({ "outcome": outcome }).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intake::fuzz::json_value;
    use proptest::prelude::*;

    fn trigger() -> Trigger {
        serde_json::from_value(
            json!({ "source": "figma", "on": "file.comment_created", "workflow": "w" }),
        )
        .unwrap()
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(512))]

        #[test]
        fn a_delivery_of_any_shape_is_observed_or_refused_without_panicking(
            event_type in prop_oneof![Just(json!("FILE_COMMENT")), Just(json!("PING")), json_value()],
            comment in json_value(),
            mentions in json_value(),
            file_key in json_value(),
            comment_id in json_value(),
        ) {
            let trigger = trigger();
            let payload = json!({
                "event_type": event_type, "comment": comment, "mentions": mentions,
                "file_key": file_key, "comment_id": comment_id,
                "created_at": "2026-01-01T00:00:00Z",
            });
            let _ = observe(&[("t", &trigger)], &payload);
        }

        #[test]
        fn a_valid_comment_with_one_field_replaced_is_observed_or_refused_without_panicking(
            field in prop_oneof![
                Just("file_key"), Just("file_name"), Just("comment"), Just("comment_id"),
                Just("mentions"), Just("parent_id"), Just("created_at"), Just("timestamp"),
                Just("triggered_by"), Just("webhook_id"),
            ],
            replacement in json_value(),
        ) {
            let trigger = trigger();
            let mut payload = json!({
                "event_type": "FILE_COMMENT", "file_key": "k1", "file_name": "f",
                "comment": [{ "text": "a " }, { "mention": "1" }, { "text": "b" }],
                "comment_id": "2", "mentions": [{ "id": "1", "handle": "h" }],
                "parent_id": "", "created_at": "2026-01-01T00:00:00Z",
                "timestamp": "2026-01-01T00:00:00Z", "triggered_by": { "handle": "u" },
                "webhook_id": "3",
            });
            payload[field] = replacement;
            let _ = observe(&[("t", &trigger)], &payload);
        }
    }
}
