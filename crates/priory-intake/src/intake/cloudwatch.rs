use super::webhook::opened;
use super::{accepted, id, output, publish_new};
use crate::log::{self, Log};
use herdr_bus::{Message, Timestamp};
use priory_factory::factory::{EventLog, Factory, Trigger};
use priory_runtime::http::{self, Reply, Request};
use serde_json::{Value, json};

pub(super) const CHANGED: &str = "cloudwatch.alarm.state_changed";
/// The header the EventBridge connection's API key authorization sends.
const TOKEN: &str = "x-priory-webhook-token";
const FILTERS: [&str; 3] = ["name", "state", "previous_state"];

/// CloudWatch alarm state changes that an EventBridge API destination delivers to
/// `POST /hooks/cloudwatch`: checked against the connection's API key before anything reads them,
/// then published on `factory.intake`.
pub struct CloudwatchWebhook {
    secret: String,
    /// The `cloudwatch` triggers, which a delivery can match.
    triggers: Vec<(String, Trigger)>,
    /// The log a delivery publishes on, opened again while a worker's open failed.
    log: EventLog,
}

impl CloudwatchWebhook {
    /// Declares `factory.intake`, so a delivery can publish before the intake ever ran.
    pub fn new(secret: String, factory: &Factory) -> Result<Self, String> {
        let bus = log::open(factory.log()).map_err(|error| error.to_string())?;
        bus.declare(&output()).map_err(|error| error.to_string())?;
        let triggers = factory
            .triggers
            .iter()
            .filter(|(_, trigger)| trigger.value.source == "cloudwatch")
            .map(|(name, trigger)| (name.clone(), trigger.value.clone()))
            .collect();
        Ok(CloudwatchWebhook {
            secret,
            triggers,
            log: factory.log().clone(),
        })
    }

    /// Verifies, maps and publishes one delivery. EventBridge waits 5 seconds for the reply, so the
    /// route makes no call to AWS, and every delivery Priory takes or drops on purpose gets `202`.
    pub fn receive(&self, bus: &mut Result<Box<dyn Log>, String>, request: &Request) -> Reply {
        let token = request.header(TOKEN);
        if !token.is_some_and(|token| http::same(token, self.secret.as_bytes())) {
            super::counted("cloudwatch", "webhook", "refused");
            return Reply::error(
                401,
                "X-Priory-Webhook-Token is missing or is not the connection's API key",
            );
        }
        let Ok(event) = serde_json::from_slice::<Value>(request.body) else {
            return Reply::error(400, "the body is not JSON");
        };
        let triggers: Vec<(&str, &Trigger)> = self
            .triggers
            .iter()
            .map(|(name, trigger)| (name.as_str(), trigger))
            .collect();
        let message = match observe(&triggers, &event) {
            Ok(Some(message)) => message,
            Ok(None) => return accepted("dropped"),
            Err(error) => return Reply::error(400, &error),
        };
        let bus = match opened(bus, || log::open(&self.log)) {
            Ok(bus) => bus,
            Err(error) => return Reply::error(500, &error),
        };
        match publish_new(bus, &triggers, &message, "cloudwatch") {
            Ok(true) => accepted("published"),
            Ok(false) => accepted("dropped"),
            Err(error) => Reply::error(500, &error),
        }
    }
}

/// The message an EventBridge event reports, or none for an event other than an alarm state
/// change, or one no trigger matches. Errs when a field the event needs is missing.
fn observe(triggers: &[(&str, &Trigger)], event: &Value) -> Result<Option<Message>, String> {
    if event["source"] != "aws.cloudwatch"
        || event["detail-type"] != "CloudWatch Alarm State Change"
    {
        return Ok(None);
    }
    let detail = &event["detail"];
    let text = |value: &Value| {
        value
            .as_str()
            .filter(|text| !text.is_empty())
            .map(str::to_owned)
    };
    let mut data = json!({
        "alarm": text(&event["resources"][0]),
        "name": text(&detail["alarmName"]),
        "description": text(&detail["configuration"]["description"]),
        "state": text(&detail["state"]["value"]),
        "previous_state": text(&detail["previousState"]["value"]),
        "reason": text(&detail["state"]["reason"]),
        "account": text(&event["account"]),
        "region": text(&event["region"]),
        "delivery": text(&event["id"]),
        "time": text(&event["time"]),
        "baseline": false,
    });
    let required = [
        ("alarm", "resources[0]"),
        ("name", "detail.alarmName"),
        ("state", "detail.state.value"),
        ("delivery", "id"),
        ("time", "time"),
    ];
    if let Some((_, field)) = required.iter().find(|(key, _)| data[*key].is_null()) {
        return Err(format!("the event lacks `{field}`"));
    }
    let raw = data["time"].as_str().unwrap_or_default();
    let time: Timestamp = raw
        .parse()
        .map_err(|error| format!("EventBridge time `{raw}`: {error}"))?;
    let matched = matching(triggers, &data);
    if matched.is_empty() {
        return Ok(None);
    }
    data["triggers"] = json!(matched);
    let field = |key: &str| data[key].as_str().unwrap_or_default().to_owned();
    // EventBridge keeps an event's id across its retries.
    let (name, subject) = (
        format!("cloudwatch:event:{}", field("delivery")),
        field("name"),
    );
    Ok(Some(
        Message::new(CHANGED, data)
            .id(id(&name))
            .subject(subject)
            .time(time),
    ))
}

/// The `cloudwatch` triggers whose `name`, `state` and `previous_state` filters the change meets.
fn matching<'a>(triggers: &[(&'a str, &Trigger)], data: &Value) -> Vec<&'a str> {
    triggers
        .iter()
        .filter(|(_, trigger)| {
            format!("cloudwatch.{}", trigger.on) == CHANGED
                && trigger.filter.iter().flatten().all(|(field, value)| {
                    FILTERS.contains(&field.as_str()) && data[field] == *value
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
            json!({ "source": "cloudwatch", "on": "alarm.state_changed", "workflow": "w" }),
        )
        .unwrap()
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(512))]

        #[test]
        fn an_event_of_any_shape_is_observed_or_refused_without_panicking(
            source in prop_oneof![Just(json!("aws.cloudwatch")), json_value()],
            kind in prop_oneof![Just(json!("CloudWatch Alarm State Change")), json_value()],
            resources in json_value(),
            detail in json_value(),
            time in json_value(),
        ) {
            let trigger = trigger();
            let event = json!({
                "id": "e", "source": source, "detail-type": kind, "resources": resources,
                "detail": detail, "time": time,
            });
            let _ = observe(&[("t", &trigger)], &event);
        }

        #[test]
        fn a_valid_event_with_one_field_replaced_is_observed_or_refused_without_panicking(
            field in prop_oneof![
                Just("id"), Just("time"), Just("resources"), Just("account"), Just("region"),
                Just("detail"),
            ],
            inner in prop_oneof![
                Just("alarmName"), Just("state"), Just("previousState"), Just("configuration"),
            ],
            replacement in json_value(),
            nested in any::<bool>(),
        ) {
            let trigger = trigger();
            let mut event = json!({
                "id": "e", "source": "aws.cloudwatch",
                "detail-type": "CloudWatch Alarm State Change", "time": "2026-01-01T00:00:00Z",
                "account": "1", "region": "us-east-1",
                "resources": ["arn:aws:cloudwatch:us-east-1:1:alarm:a"],
                "detail": {
                    "alarmName": "a", "configuration": { "description": "d" },
                    "state": { "value": "ALARM", "reason": "r" },
                    "previousState": { "value": "OK" },
                },
            });
            if nested {
                event["detail"][inner] = replacement;
            } else {
                event[field] = replacement;
            }
            let _ = observe(&[("t", &trigger)], &event);
        }
    }
}
