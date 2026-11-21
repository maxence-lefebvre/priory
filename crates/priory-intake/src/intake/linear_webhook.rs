use super::linear::{COMMENTED, CREATED, STATE_CHANGED, event};
use super::webhook::{hex, unhex};
use super::{claim, dispatch, now, output, publish, reported};
use crate::log::{self, Log};
use hmac::{Hmac, KeyInit, Mac};
use priory_factory::factory::{Factory, Trigger};
use priory_runtime::http::{Reply, Request};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// How far `webhookTimestamp` may be from now, in milliseconds: Linear's documented replay window.
const WINDOW_MS: i64 = 60_000;

/// Linear deliveries to `POST /hooks/linear`: checked against the signing secret before anything
/// reads them, then published on `factory.intake` as the polling intake publishes the same events.
pub struct LinearWebhook {
    secret: String,
    /// The `linear` triggers, which a delivery can match. A `linear.view` trigger names a view, which a
    /// delivery cannot tell membership of, so polling alone serves it.
    triggers: Vec<(String, Trigger)>,
}

impl LinearWebhook {
    /// Declares `factory.intake`, so a delivery can publish before the intake ever ran.
    pub fn new(secret: String, factory: &Factory) -> Result<Self, String> {
        let bus = log::open().map_err(|error| error.to_string())?;
        bus.declare(&output()).map_err(|error| error.to_string())?;
        let triggers = factory
            .triggers
            .iter()
            .filter(|(_, trigger)| trigger.value.source == "linear")
            .map(|(name, trigger)| (name.clone(), trigger.value.clone()))
            .collect();
        Ok(LinearWebhook { secret, triggers })
    }

    /// Verifies, maps and publishes one delivery. Linear retries any reply but `200`, so every
    /// delivery Priory takes or drops on purpose gets it.
    pub fn receive(&self, bus: &Result<Box<dyn Log>, String>, request: &Request) -> Reply {
        if !self.signed(request) {
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
            Ok(now) if (now.as_millisecond() - sent).abs() <= WINDOW_MS => {}
            Ok(_) => return Reply::error(401, "`webhookTimestamp` is over a minute from now"),
            Err(error) => return Reply::error(500, &error),
        }
        let (kind, name, data) = match self.observe(&payload, request.body) {
            Ok(Some(observed)) => observed,
            Ok(None) => return accepted("dropped"),
            Err(error) => return Reply::error(400, &error),
        };
        let bus = match bus {
            Ok(bus) => bus.as_ref(),
            Err(error) => return Reply::error(500, error),
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
                return Ok(false);
            }
            publish(bus, &message)?;
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
    fn observe(
        &self,
        payload: &Value,
        body: &[u8],
    ) -> Result<Option<(&'static str, String, Value)>, String> {
        let data = &payload["data"];
        let actor = &payload["actor"]["name"];
        let (kind, name, event_time, mut fields, missing) =
            match (payload["type"].as_str(), payload["action"].as_str()) {
                (Some("Issue"), Some("create")) => (
                    CREATED,
                    format!(
                        "linear:issue:{}:created",
                        data["id"].as_str().unwrap_or_default()
                    ),
                    &data["createdAt"],
                    issue_fields(data, &payload["url"]),
                    project_unknown(data),
                ),
                (Some("Issue"), Some("update"))
                    if payload["updatedFrom"].get("stateId").is_some() =>
                {
                    let mut fields = issue_fields(data, &payload["url"]);
                    fields["from"] = Value::Null;
                    fields["to"] = data["state"]["name"].clone();
                    let name = format!(
                        "linear:webhook:state_changed:{}",
                        hex(&Sha256::digest(body))
                    );
                    (
                        STATE_CHANGED,
                        name,
                        &payload["createdAt"],
                        fields,
                        project_unknown(data),
                    )
                }
                (Some("Comment"), Some("create")) => {
                    let id = data["id"].as_str().unwrap_or_default();
                    let mut fields = comment_fields(payload);
                    fields["body"] = data["body"].clone();
                    let name = format!("linear:comment:{id}");
                    (
                        COMMENTED,
                        name,
                        &data["createdAt"],
                        fields,
                        &["project", "label", "state"][..],
                    )
                }
                _ => return Ok(None),
            };
        if !self.tracked(kind, &fields["team"], missing) {
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

    /// Whether a `linear` trigger of this event type names the team, and the payload holds every
    /// field its filter reads. A trigger whose filter reads a field in `missing` could not be
    /// matched here, and the poll, which reads it, publishes the same id with the whole event.
    fn tracked(&self, kind: &str, team: &Value, missing: &[&str]) -> bool {
        let names_team = |trigger: &Trigger| {
            let filter = trigger.filter.as_ref();
            filter
                .and_then(|filter| filter.get("team"))
                .is_none_or(|key| key == team)
        };
        let blind = |trigger: &Trigger| {
            trigger
                .filter
                .iter()
                .flatten()
                .any(|(field, _)| missing.contains(&field.as_str()))
        };
        let of_kind =
            |trigger: &&Trigger| format!("linear.{}", trigger.on) == kind && names_team(trigger);
        let mut named = self
            .triggers
            .iter()
            .map(|(_, trigger)| trigger)
            .filter(of_kind);
        named.clone().next().is_some() && !named.any(blind)
    }
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

/// The filter fields an `Issue` payload cannot decide: the project's name when it names only an id.
fn project_unknown(data: &Value) -> &'static [&'static str] {
    match data["projectId"].is_string() && !data["project"]["name"].is_string() {
        true => &["project"],
        false => &[],
    }
}

fn accepted(outcome: &str) -> Reply {
    Reply::json(200, json!({ "outcome": outcome }).to_string())
}
