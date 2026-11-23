use super::webhook::{opened, unhex};
use super::{accepted, id, output, publish_new};
use crate::log::{self, Log};
use herdr_bus::{Message, Timestamp};
use hmac::{Hmac, KeyInit, Mac};
use priory_factory::factory::{EventLog, Factory, Trigger};
use priory_runtime::http::{Reply, Request};
use serde_json::{Value, json};
use sha2::Sha256;

pub(super) const CREATED: &str = "sentry.issue.created";

/// Sentry deliveries to `POST /hooks/sentry`: checked against the integration's Client Secret
/// before anything reads them, then published on `factory.intake`.
pub struct SentryWebhook {
    secret: String,
    /// The `sentry` triggers, which a delivery can match.
    triggers: Vec<(String, Trigger)>,
    /// The log a delivery publishes on, opened again while a worker's open failed.
    log: EventLog,
}

impl SentryWebhook {
    /// Declares `factory.intake`, so a delivery can publish before the intake ever ran.
    pub fn new(secret: String, factory: &Factory) -> Result<Self, String> {
        let bus = log::open(factory.log()).map_err(|error| error.to_string())?;
        bus.declare(&output()).map_err(|error| error.to_string())?;
        let triggers = factory
            .triggers
            .iter()
            .filter(|(_, trigger)| trigger.value.source == "sentry")
            .map(|(name, trigger)| (name.clone(), trigger.value.clone()))
            .collect();
        Ok(SentryWebhook {
            secret,
            triggers,
            log: factory.log().clone(),
        })
    }

    /// Verifies, maps and publishes one delivery. Sentry waits 1 second for the reply, so the
    /// route does no network call, and every delivery Priory takes or drops on purpose gets `202`.
    pub fn receive(&self, bus: &mut Result<Box<dyn Log>, String>, request: &Request) -> Reply {
        if !self.signed(request) {
            super::counted("sentry", "webhook", "refused");
            return Reply::error(
                401,
                "Sentry-Hook-Signature is missing or does not match the body",
            );
        }
        let header = |name| {
            request
                .header(name)
                .and_then(|value| std::str::from_utf8(value).ok())
        };
        let Ok(payload) = serde_json::from_slice::<Value>(request.body) else {
            return Reply::error(400, "the body is not JSON");
        };
        let triggers: Vec<(&str, &Trigger)> = self
            .triggers
            .iter()
            .map(|(name, trigger)| (name.as_str(), trigger))
            .collect();
        let message = match observe(
            &triggers,
            header("sentry-hook-resource"),
            header("request-id"),
            &payload,
        ) {
            Ok(Some(message)) => message,
            Ok(None) => return accepted("dropped"),
            Err(error) => return Reply::error(400, &error),
        };
        let bus = match opened(bus, || log::open(&self.log)) {
            Ok(bus) => bus,
            Err(error) => return Reply::error(500, &error),
        };
        match publish_new(bus, &triggers, &message, "sentry") {
            Ok(true) => accepted("published"),
            Ok(false) => accepted("dropped"),
            Err(error) => Reply::error(500, &error),
        }
    }

    /// Whether `Sentry-Hook-Signature` is the hex HMAC-SHA256 of the body under the secret,
    /// compared in constant time.
    fn signed(&self, request: &Request) -> bool {
        let Some(sent) = request.header("sentry-hook-signature").and_then(unhex) else {
            return false;
        };
        let mut mac =
            Hmac::<Sha256>::new_from_slice(self.secret.as_bytes()).expect("HMAC takes any key");
        mac.update(request.body);
        mac.verify_slice(&sent).is_ok()
    }
}

/// The message a delivery reports, or none for a resource or action Priory does not take, or an
/// issue no trigger matches. Errs when a field the event needs is missing.
fn observe(
    triggers: &[(&str, &Trigger)],
    resource: Option<&str>,
    delivery: Option<&str>,
    payload: &Value,
) -> Result<Option<Message>, String> {
    if resource != Some("issue") || payload["action"] != "created" {
        return Ok(None);
    }
    let issue = &payload["data"]["issue"];
    let text = |value: &Value| {
        value
            .as_str()
            .filter(|text| !text.is_empty())
            .map(str::to_owned)
    };
    let mut data = json!({
        "issue": text(&issue["id"]),
        "short_id": text(&issue["shortId"]),
        "title": text(&issue["title"]),
        "culprit": text(&issue["culprit"]),
        "url": text(&issue["web_url"]),
        "project": text(&issue["project"]["slug"]),
        "level": text(&issue["level"]),
        "category": text(&issue["issueCategory"]),
        "actor": text(&payload["actor"]["name"]),
        "delivery": delivery,
        "time": text(&issue["firstSeen"]),
        "baseline": false,
    });
    let required = [
        ("issue", "id"),
        ("short_id", "shortId"),
        ("title", "title"),
        ("url", "web_url"),
        ("project", "project.slug"),
        ("time", "firstSeen"),
    ];
    if let Some((_, field)) = required.iter().find(|(key, _)| data[*key].is_null()) {
        return Err(format!("the payload lacks the issue's `{field}`"));
    }
    let raw = data["time"].as_str().unwrap_or_default();
    let time: Timestamp = raw
        .parse()
        .map_err(|error| format!("Sentry time `{raw}`: {error}"))?;
    let matched = matching(triggers, &data);
    if matched.is_empty() {
        return Ok(None);
    }
    data["triggers"] = json!(matched);
    let subject = data["short_id"].as_str().unwrap_or_default().to_owned();
    let url = data["url"].as_str().unwrap_or_default().to_owned();
    let name = format!(
        "sentry:issue:{}:created",
        data["issue"].as_str().unwrap_or_default()
    );
    Ok(Some(
        Message::new(CREATED, data)
            .id(id(&name))
            .subject(subject)
            .time(time)
            .link("issue", &url),
    ))
}

/// The `sentry` triggers whose `project` and `level` filters the issue meets.
fn matching<'a>(triggers: &[(&'a str, &Trigger)], data: &Value) -> Vec<&'a str> {
    triggers
        .iter()
        .filter(|(_, trigger)| {
            format!("sentry.{}", trigger.on) == CREATED
                && trigger
                    .filter
                    .iter()
                    .flatten()
                    .all(|(field, value)| match field.as_str() {
                        "project" | "level" => data[field] == *value,
                        _ => false,
                    })
        })
        .map(|(name, _)| *name)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intake::fuzz::json_value;
    use proptest::prelude::*;

    fn trigger() -> Trigger {
        serde_json::from_value(
            json!({ "source": "sentry", "on": "issue.created", "workflow": "w" }),
        )
        .unwrap()
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(512))]

        #[test]
        fn a_delivery_of_any_shape_is_observed_or_refused_without_panicking(
            resource in prop_oneof![Just(Some("issue")), Just(Some("error")), Just(None)],
            action in prop_oneof![Just(json!("created")), json_value()],
            issue in json_value(),
            actor in json_value(),
        ) {
            let trigger = trigger();
            let payload = json!({ "action": action, "data": { "issue": issue }, "actor": actor });
            let _ = observe(&[("t", &trigger)], resource, Some("r"), &payload);
        }

        #[test]
        fn a_valid_issue_with_one_field_replaced_is_observed_or_refused_without_panicking(
            field in prop_oneof![
                Just("id"), Just("shortId"), Just("title"), Just("culprit"), Just("web_url"),
                Just("project"), Just("level"), Just("issueCategory"), Just("firstSeen"),
            ],
            replacement in json_value(),
        ) {
            let trigger = trigger();
            let mut issue = json!({
                "id": "1", "shortId": "P-1", "title": "t", "culprit": "c",
                "web_url": "https://o.sentry.io/issues/1/", "project": { "slug": "p" },
                "level": "error", "issueCategory": "error",
                "firstSeen": "2026-01-01T00:00:00.000000+00:00",
            });
            issue[field] = replacement;
            let payload = json!({ "action": "created", "data": { "issue": issue } });
            let _ = observe(&[("t", &trigger)], Some("issue"), None, &payload);
        }
    }
}
