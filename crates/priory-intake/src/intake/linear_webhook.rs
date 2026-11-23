use super::linear::{COMMENTED, CREATED, STATE_CHANGED, event};
use super::webhook::{opened, unhex};
use super::{claim, dispatch, now, output, publish_delivered, reported};
use crate::log::{self, Log};
use hmac::{Hmac, KeyInit, Mac};
use priory_factory::factory::{EventLog, Factory, Trigger};
use priory_runtime::http::{Reply, Request};
use serde_json::{Value, json};
use sha2::Sha256;

/// Whether Linear's webhook reports `message`, a poll's event: a created issue or a state change
/// outside a view, or a comment a trigger requests. Its first poll's events are no deliveries.
pub(super) fn reports(message: &herdr_bus::Message) -> bool {
    let requested = message.data["triggers"]
        .as_array()
        .is_some_and(|triggers| !triggers.is_empty());
    message.data["baseline"] != true
        && match message.kind.as_str() {
            CREATED | STATE_CHANGED => true,
            COMMENTED => requested,
            _ => false,
        }
}

/// How far `webhookTimestamp` may be from now, in milliseconds: Linear's documented replay window.
const WINDOW_MS: u64 = 60_000;

/// Linear deliveries to `POST /hooks/linear`: checked against the signing secret before anything
/// reads them, then published on `factory.intake` as the polling intake publishes the same events.
pub struct LinearWebhook {
    secret: String,
    /// The `linear` triggers, which a delivery can match. A `linear.view` trigger names a view, which a
    /// delivery cannot tell membership of, so polling alone serves it.
    triggers: Vec<(String, Trigger)>,
    /// The log a delivery publishes on, opened again while a worker's open failed.
    log: EventLog,
}

impl LinearWebhook {
    /// Declares `factory.intake`, so a delivery can publish before the intake ever ran.
    pub fn new(secret: String, factory: &Factory) -> Result<Self, String> {
        let bus = log::open(factory.log()).map_err(|error| error.to_string())?;
        bus.declare(&output()).map_err(|error| error.to_string())?;
        let triggers = factory
            .triggers
            .iter()
            .filter(|(_, trigger)| trigger.value.source == "linear")
            .map(|(name, trigger)| (name.clone(), trigger.value.clone()))
            .collect();
        Ok(LinearWebhook {
            secret,
            triggers,
            log: factory.log().clone(),
        })
    }

    /// Verifies, maps and publishes one delivery. Linear retries any reply but `200`, so every
    /// delivery Priory takes or drops on purpose gets it.
    pub fn receive(&self, bus: &mut Result<Box<dyn Log>, String>, request: &Request) -> Reply {
        if !self.signed(request) {
            super::counted("linear", "webhook", "refused");
            return Reply::error(
                401,
                "Linear-Signature is missing or does not match the body",
            );
        }
        let Ok(payload) = serde_json::from_slice::<Value>(request.body) else {
            return Reply::error(400, "the body is not JSON");
        };
        let Some(sent) = payload["webhookTimestamp"].as_i64() else {
            return Reply::error(400, "the payload lacks `webhookTimestamp`");
        };
        match now() {
            Ok(now) if now.as_millisecond().abs_diff(sent) <= WINDOW_MS => {}
            Ok(_) => {
                super::counted("linear", "webhook", "refused");
                return Reply::error(401, "`webhookTimestamp` is over a minute from now");
            }
            Err(error) => return Reply::error(500, &error),
        }
        let (kind, name, data) = match self.observe(&payload) {
            Ok(Some(observed)) => observed,
            Ok(None) => return accepted("dropped"),
            Err(error) => return Reply::error(400, &error),
        };
        let bus = match opened(bus, || log::open(&self.log)) {
            Ok(bus) => bus,
            Err(error) => return Reply::error(500, &error),
        };
        let triggers: Vec<(&str, &Trigger)> = self
            .triggers
            .iter()
            .map(|(name, trigger)| (name.as_str(), trigger))
            .collect();
        let published = event(&triggers, kind, &name, data, false).and_then(|mut message| {
            let delivery = request.header("linear-delivery");
            message.data["delivery"] = json!(delivery.and_then(|v| std::str::from_utf8(v).ok()));
            let _claim = claim()?;
            if reported(bus, &message)? {
                super::counted("linear", "webhook", "duplicate");
                return Ok(false);
            }
            publish_delivered(bus, &message, "linear")?;
            dispatch(bus, &triggers, &message)?;
            Ok(true)
        });
        match published {
            Ok(true) => accepted("published"),
            Ok(false) => accepted("dropped"),
            Err(error) => Reply::error(500, &error),
        }
    }

    /// Whether `Linear-Signature` is the hex HMAC-SHA256 of the body under the secret, compared in
    /// constant time.
    fn signed(&self, request: &Request) -> bool {
        let Some(sent) = request.header("linear-signature").and_then(unhex) else {
            return false;
        };
        let mut mac =
            Hmac::<Sha256>::new_from_slice(self.secret.as_bytes()).expect("HMAC takes any key");
        mac.update(request.body);
        mac.verify_slice(&sent).is_ok()
    }

    /// The type, id name and data of the event a delivery reports, or none for a type, action or
    /// team Priory does not take, or an event whose triggers the payload cannot decide. Errs when a
    /// field the event needs is missing.
    fn observe(&self, payload: &Value) -> Result<Option<(&'static str, String, Value)>, String> {
        let data = &payload["data"];
        let actor = &payload["actor"]["name"];
        let (kind, name, event_time, mut fields) =
            match (payload["type"].as_str(), payload["action"].as_str()) {
                (Some("Issue"), Some("create")) => (
                    CREATED,
                    format!(
                        "linear:issue:{}:created",
                        data["id"].as_str().unwrap_or_default()
                    ),
                    &data["createdAt"],
                    issue_fields(data, &payload["url"]),
                ),
                (Some("Issue"), Some("update"))
                    if payload["updatedFrom"].get("stateId").is_some() =>
                {
                    let mut fields = issue_fields(data, &payload["url"]);
                    fields["from"] = Value::Null;
                    fields["to"] = data["state"]["name"].clone();
                    // Signed fields that a redelivery keeps, unlike the body's `webhookTimestamp`.
                    let name = format!(
                        "linear:webhook:state_changed:{}:{}:{}",
                        data["id"].as_str().unwrap_or_default(),
                        data["state"]["id"].as_str().unwrap_or_default(),
                        payload["createdAt"].as_str().unwrap_or_default(),
                    );
                    (STATE_CHANGED, name, &payload["createdAt"], fields)
                }
                (Some("Comment"), Some("create")) => {
                    let id = data["id"].as_str().unwrap_or_default();
                    let mut fields = comment_fields(payload);
                    fields["body"] = data["body"].clone();
                    (
                        COMMENTED,
                        format!("linear:comment:{id}"),
                        &data["createdAt"],
                        fields,
                    )
                }
                _ => return Ok(None),
            };
        if !self.names_team(&fields["team"]) {
            return Ok(None);
        }
        // The poll reads what the payload lacks, and a task request cannot be repaired after the
        // message that carries it is published: the first writer of an id wins.
        let incomplete = match kind {
            COMMENTED => self.requests(kind, &fields["team"]),
            _ => data["projectId"].is_string() && !data["project"]["name"].is_string(),
        };
        if incomplete {
            return Ok(None);
        }
        fields["actor"] = actor.clone();
        fields["time"] = event_time.clone();
        if kind != COMMENTED {
            fields["blockers"] = Value::Null;
        }
        let required: &[&str] = match kind {
            COMMENTED => &["issue", "linear_id", "url", "team", "time", "body"],
            _ => &[
                "issue",
                "linear_id",
                "title",
                "url",
                "team",
                "state",
                "time",
            ],
        };
        let lacks = |field: &&&str| fields[**field].as_str().is_none_or(str::is_empty);
        if let Some(field) = required.iter().find(lacks) {
            return Err(format!("the payload lacks the event's `{field}`"));
        }
        Ok(Some((kind, name, fields)))
    }

    /// Whether a `linear` trigger of type `kind` names the team.
    fn requests(&self, kind: &str, team: &Value) -> bool {
        self.triggers
            .iter()
            .any(|(_, trigger)| format!("linear.{}", trigger.on) == kind && names(trigger, team))
    }

    /// Whether any `linear` trigger names the team. The poll reads every event of the workspace for
    /// them, so a delivery for one publishes even when no trigger of its type does.
    fn names_team(&self, team: &Value) -> bool {
        self.triggers
            .iter()
            .any(|(_, trigger)| names(trigger, team))
    }
}

/// Whether the trigger's `team` filter equals the team key, or it has none.
fn names(trigger: &Trigger, team: &Value) -> bool {
    let key = trigger
        .filter
        .as_ref()
        .and_then(|filter| filter.get("team"));
    key.is_none_or(|key| key == team)
}

/// The fields of an `Issue` payload's data, as a poll reads them.
fn issue_fields(data: &Value, url: &Value) -> Value {
    let identifier = data["identifier"].clone();
    let labels: Vec<&Value> = data["labels"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|label| &label["name"])
        .collect();
    json!({
        "issue": identifier,
        "linear_id": data["id"],
        "title": data["title"],
        "url": url,
        "team": data["team"]["key"],
        "project": data["project"]["name"],
        "project_id": data["projectId"],
        "state": data["state"]["name"],
        "labels": labels,
    })
}

/// A `Comment` payload names the issue by its page: `.../issue/ENG-7/slug#comment-id`. Its title,
/// state and labels are not in it, so they stay empty.
fn comment_fields(payload: &Value) -> Value {
    let data = &payload["data"];
    let url = payload["url"].as_str().unwrap_or_default();
    let identifier = url
        .split("/issue/")
        .nth(1)
        .and_then(|rest| rest.split('/').next());
    let team = identifier
        .and_then(|identifier| identifier.rsplit_once('-'))
        .map(|(team, _)| team);
    json!({
        "issue": identifier,
        "linear_id": data["id"],
        "title": data["issue"]["title"].as_str().unwrap_or_default(),
        "url": url,
        "team": team,
        "project": null,
        "project_id": null,
        "state": "",
        "labels": [],
    })
}

fn accepted(outcome: &str) -> Reply {
    Reply::json(200, json!({ "outcome": outcome }).to_string())
}
