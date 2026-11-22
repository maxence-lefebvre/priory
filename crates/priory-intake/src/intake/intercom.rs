use super::webhook::{opened, unhex};
use super::{accepted, id, output, publish_new};
use crate::log::{self, Log};
use herdr_bus::{Message, Timestamp};
use hmac::{Hmac, KeyInit, Mac};
use priory_factory::factory::{EventLog, Factory, Trigger};
use priory_runtime::http::{Reply, Request};
use serde_json::{Value, json};
use sha1::Sha1;

pub(super) const CREATED: &str = "intercom.conversation.created";

/// Intercom deliveries to `POST /hooks/intercom`: checked against the app's client secret before
/// anything reads them, then published on `factory.intake` when they come from `workspace`.
pub struct IntercomWebhook {
    secret: String,
    /// The workspace id whose notifications Priory takes; an app installed elsewhere sends others.
    workspace: String,
    /// The `intercom` triggers, which a delivery can match.
    triggers: Vec<(String, Trigger)>,
    /// The log a delivery publishes on, opened again while a worker's open failed.
    log: EventLog,
}

impl IntercomWebhook {
    /// Declares `factory.intake`, so a delivery can publish before the intake ever ran.
    pub fn new(secret: String, workspace: String, factory: &Factory) -> Result<Self, String> {
        let bus = log::open(factory.log()).map_err(|error| error.to_string())?;
        bus.declare(&output()).map_err(|error| error.to_string())?;
        let triggers = factory
            .triggers
            .iter()
            .filter(|(_, trigger)| trigger.value.source == "intercom")
            .map(|(name, trigger)| (name.clone(), trigger.value.clone()))
            .collect();
        Ok(IntercomWebhook {
            secret,
            workspace,
            triggers,
            log: factory.log().clone(),
        })
    }

    /// Verifies, maps and publishes one delivery. Intercom waits 5 seconds and sends a
    /// notification again after any other reply than `2xx`, so the route makes no call to Intercom.
    pub fn receive(&self, bus: &mut Result<Box<dyn Log>, String>, request: &Request) -> Reply {
        if !self.signed(request) {
            return Reply::error(401, "X-Hub-Signature is missing or does not match the body");
        }
        let Ok(payload) = serde_json::from_slice::<Value>(request.body) else {
            return Reply::error(400, "the body is not JSON");
        };
        if payload["app_id"] != self.workspace.as_str() {
            return accepted("dropped");
        }
        let triggers: Vec<(&str, &Trigger)> = self
            .triggers
            .iter()
            .map(|(name, trigger)| (name.as_str(), trigger))
            .collect();
        let message = match observe(&triggers, &payload) {
            Ok(Some(message)) => message,
            Ok(None) => return accepted("dropped"),
            Err(error) => return Reply::error(400, &error),
        };
        let bus = match opened(bus, || log::open(&self.log)) {
            Ok(bus) => bus,
            Err(error) => return Reply::error(500, &error),
        };
        match publish_new(bus, &triggers, &message) {
            Ok(true) => accepted("published"),
            Ok(false) => accepted("dropped"),
            Err(error) => Reply::error(500, &error),
        }
    }

    /// Whether `X-Hub-Signature` is `sha1=` and the hex HMAC-SHA1 of the body under the secret,
    /// compared in constant time.
    fn signed(&self, request: &Request) -> bool {
        let Some(sent) = request
            .header("x-hub-signature")
            .and_then(|value| value.strip_prefix(b"sha1="))
            .and_then(unhex)
        else {
            return false;
        };
        let mut mac =
            Hmac::<Sha1>::new_from_slice(self.secret.as_bytes()).expect("HMAC takes any key");
        mac.update(request.body);
        mac.verify_slice(&sent).is_ok()
    }
}

/// The message a notification reports, or none for a topic Priory does not take, or a
/// conversation no trigger matches. Errs when a field the event needs is missing.
fn observe(triggers: &[(&str, &Trigger)], payload: &Value) -> Result<Option<Message>, String> {
    if payload["topic"] != "conversation.user.created" {
        return Ok(None);
    }
    let item = &payload["data"]["item"];
    let source = &item["source"];
    let text = |value: &Value| {
        value
            .as_str()
            .filter(|text| !text.is_empty())
            .map(str::to_owned)
    };
    let mut data = json!({
        "conversation": text(&item["id"]),
        "workspace": text(&payload["app_id"]),
        "title": text(&item["title"]),
        "subject": text(&source["subject"]),
        "body": text(&source["body"]),
        "author": text(&source["author"]["name"]),
        "author_type": text(&source["author"]["type"]),
        "page": text(&source["url"]),
        "delivery": text(&payload["id"]),
        "baseline": false,
    });
    if data["conversation"].is_null() {
        return Err("the payload lacks the conversation's `id`".into());
    }
    let Some(seconds) = item["created_at"].as_i64() else {
        return Err("the payload lacks the conversation's `created_at`".into());
    };
    let time = Timestamp::from_second(seconds)
        .map_err(|error| format!("Intercom time `{seconds}`: {error}"))?;
    data["time"] = json!(time.to_string());
    let matched: Vec<&str> = triggers
        .iter()
        .filter(|(_, trigger)| format!("intercom.{}", trigger.on) == CREATED)
        .map(|(name, _)| *name)
        .collect();
    if matched.is_empty() {
        return Ok(None);
    }
    data["triggers"] = json!(matched);
    let conversation = data["conversation"].as_str().unwrap_or_default().to_owned();
    let subject = data["title"]
        .as_str()
        .map_or_else(|| format!("conversation {conversation}"), str::to_owned);
    Ok(Some(
        Message::new(CREATED, data)
            .id(id(&format!("intercom:conversation:{conversation}:created")))
            .subject(subject)
            .time(time),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intake::fuzz::json_value;
    use proptest::prelude::*;

    fn trigger() -> Trigger {
        serde_json::from_value(
            json!({ "source": "intercom", "on": "conversation.created", "workflow": "w" }),
        )
        .unwrap()
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(512))]

        #[test]
        fn a_notification_of_any_shape_is_observed_or_refused_without_panicking(
            topic in prop_oneof![Just(json!("conversation.user.created")), json_value()],
            item in json_value(),
        ) {
            let trigger = trigger();
            let payload = json!({ "topic": topic, "app_id": "w", "data": { "item": item } });
            let _ = observe(&[("t", &trigger)], &payload);
        }

        #[test]
        fn a_valid_conversation_with_one_field_replaced_is_observed_or_refused_without_panicking(
            field in prop_oneof![
                Just("id"), Just("title"), Just("created_at"), Just("source"),
            ],
            replacement in json_value(),
        ) {
            let trigger = trigger();
            let mut item = json!({
                "id": "1", "title": "t", "created_at": 1795353698,
                "source": { "subject": "s", "body": "b", "url": "https://x",
                            "author": { "type": "user", "name": "n" } },
            });
            item[field] = replacement;
            let payload = json!({
                "topic": "conversation.user.created", "app_id": "w", "id": "notif_1",
                "data": { "item": item },
            });
            let _ = observe(&[("t", &trigger)], &payload);
        }
    }
}
