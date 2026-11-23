use super::webhook::opened;
use super::{accepted, id, output, publish_new};
use crate::log::{self, Log};
use herdr_bus::{Message, Timestamp};
use priory_factory::factory::{EventLog, Factory, Trigger};
use priory_runtime::http::{self, Reply, Request};
use serde_json::{Value, json};

pub(super) const FIRED: &str = "honeycomb.trigger.fired";

/// Honeycomb deliveries to `POST /hooks/honeycomb`: checked against the recipient's Shared Secret
/// before anything reads them, then published on `factory.intake`.
pub struct HoneycombWebhook {
    secret: String,
    /// The `honeycomb` triggers, which a delivery can match.
    triggers: Vec<(String, Trigger)>,
    /// The log a delivery publishes on, opened again while a worker's open failed.
    log: EventLog,
}

impl HoneycombWebhook {
    /// Declares `factory.intake`, so a delivery can publish before the intake ever ran.
    pub fn new(secret: String, factory: &Factory) -> Result<Self, String> {
        let bus = log::open(factory.log()).map_err(|error| error.to_string())?;
        bus.declare(&output()).map_err(|error| error.to_string())?;
        let triggers = factory
            .triggers
            .iter()
            .filter(|(_, trigger)| trigger.value.source == "honeycomb")
            .map(|(name, trigger)| (name.clone(), trigger.value.clone()))
            .collect();
        Ok(HoneycombWebhook {
            secret,
            triggers,
            log: factory.log().clone(),
        })
    }

    /// Verifies, maps and publishes one delivery. The route makes no network call, and every
    /// delivery Priory takes or drops on purpose gets `202`.
    pub fn receive(&self, bus: &mut Result<Box<dyn Log>, String>, request: &Request) -> Reply {
        let token = request.header("x-honeycomb-webhook-token");
        if !token.is_some_and(|token| http::same(token, self.secret.as_bytes())) {
            super::counted("honeycomb", "webhook", "refused");
            return Reply::error(
                401,
                "X-Honeycomb-Webhook-Token is missing or is not the Shared Secret",
            );
        }
        let Ok(payload) = serde_json::from_slice::<Value>(request.body) else {
            return Reply::error(400, "the body is not JSON");
        };
        let triggers: Vec<(&str, &Trigger)> = self
            .triggers
            .iter()
            .map(|(name, trigger)| (name.as_str(), trigger))
            .collect();
        let delivery = request
            .header("x-honeycomb-webhook-delivery-id")
            .and_then(|value| std::str::from_utf8(value).ok());
        let message = match observe(&triggers, delivery, &payload) {
            Ok(Some(message)) => message,
            Ok(None) => return accepted("dropped"),
            Err(error) => return Reply::error(400, &error),
        };
        // Honeycomb retries a 503 only with `Retry-After`, which the server adds to every 503.
        let bus = match opened(bus, || log::open(&self.log)) {
            Ok(bus) => bus,
            Err(error) => return Reply::error(503, &error),
        };
        match publish_new(bus, &triggers, &message, "honeycomb") {
            Ok(true) => accepted("published"),
            Ok(false) => accepted("dropped"),
            Err(error) => Reply::error(503, &error),
        }
    }
}

/// The message a delivery reports, or none for an `OK` or test alert, or one no trigger matches.
/// Errs when a field the event needs is missing, as from a webhook without Priory's template.
fn observe(
    triggers: &[(&str, &Trigger)],
    delivery: Option<&str>,
    payload: &Value,
) -> Result<Option<Message>, String> {
    let text = |key: &str| {
        payload[key]
            .as_str()
            .filter(|text| !text.is_empty())
            .map(str::to_owned)
    };
    let mut data = json!({
        "trigger": text("trigger"),
        "name": text("name"),
        "description": text("description"),
        "url": text("url"),
        "environment": text("environment"),
        "summary": text("summary"),
        "instance": text("instance"),
        "result_url": text("result_url"),
        "delivery": delivery,
        "time": text("time"),
        "baseline": false,
    });
    let required = ["trigger", "name", "url", "instance", "time"];
    // `status` and `test` decide whether to publish, so a mistyped one must not pass as a default.
    let lacking = required
        .into_iter()
        .find(|key| data[*key].is_null())
        .or_else(|| (!payload["status"].is_string()).then_some("status"))
        .or_else(|| (!payload["test"].is_boolean()).then_some("test"));
    if let Some(field) = lacking {
        return Err(format!(
            "the payload lacks `{field}`: give the webhook the payload template in docs/intake/honeycomb.md"
        ));
    }
    if payload["status"] != "TRIGGERED" || payload["test"] == true {
        return Ok(None);
    }
    let raw = data["time"].as_str().unwrap_or_default();
    let time: Timestamp = raw
        .parse()
        .map_err(|error| format!("Honeycomb time `{raw}`: {error}"))?;
    let matched = matching(triggers, &data);
    if matched.is_empty() {
        return Ok(None);
    }
    data["triggers"] = json!(matched);
    let field = |key: &str| data[key].as_str().unwrap_or_default().to_owned();
    let name = format!(
        "honeycomb:trigger:{}:fired:{}",
        field("trigger"),
        field("instance")
    );
    let (subject, url) = (field("name"), field("url"));
    Ok(Some(
        Message::new(FIRED, data)
            .id(id(&name))
            .subject(subject)
            .time(time)
            .link("trigger", &url),
    ))
}

/// The `honeycomb` triggers whose `environment` and `trigger` filters the alert meets.
fn matching<'a>(triggers: &[(&'a str, &Trigger)], data: &Value) -> Vec<&'a str> {
    triggers
        .iter()
        .filter(|(_, trigger)| {
            format!("honeycomb.{}", trigger.on) == FIRED
                && trigger
                    .filter
                    .iter()
                    .flatten()
                    .all(|(field, value)| match field.as_str() {
                        "environment" | "trigger" => data[field] == *value,
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
            json!({ "source": "honeycomb", "on": "trigger.fired", "workflow": "w" }),
        )
        .unwrap()
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(512))]

        #[test]
        fn a_delivery_of_any_shape_is_observed_or_refused_without_panicking(
            payload in json_value(),
        ) {
            let trigger = trigger();
            let _ = observe(&[("t", &trigger)], Some("d"), &payload);
        }

        #[test]
        fn a_valid_alert_with_one_field_replaced_is_observed_or_refused_without_panicking(
            field in prop_oneof![
                Just("trigger"), Just("name"), Just("description"), Just("url"),
                Just("environment"), Just("status"), Just("summary"), Just("instance"),
                Just("test"), Just("time"), Just("result_url"),
            ],
            replacement in json_value(),
        ) {
            let trigger = trigger();
            let mut payload = json!({
                "trigger": "t1", "name": "n", "description": "d", "url": "https://h/t1",
                "environment": "e", "status": "TRIGGERED", "summary": "s", "instance": "i",
                "test": false, "time": "2026-01-01T00:00:00Z", "result_url": "https://h/r",
            });
            payload[field] = replacement;
            let _ = observe(&[("t", &trigger)], None, &payload);
        }
    }
}
