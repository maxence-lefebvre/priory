use super::pulls::{comment, thread_comment};
use super::{
    COMMENTED, LABELED, LOOKBACK_SECONDS, OPENED, Observed, PR_CLOSED, PR_LABELED, PR_OPENED,
    REVIEWED, THREAD_COMMENTED, claim, dispatch, now, output, publish, reported,
};
use herdr_bus::{Bus, Timestamp};
use hmac::{Hmac, KeyInit, Mac};
use priory_factory::factory::{Factory, Trigger};
use priory_runtime::http::{Reply, Request};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

/// GitHub deliveries to `POST /hooks/github`: checked against the secret before anything reads
/// them, then published on `factory.intake` as the polling intake publishes the same events.
pub struct Github {
    secret: String,
    /// The `github` triggers, which a delivery can match.
    triggers: Vec<(String, Trigger)>,
    /// The repositories some trigger names, which polling reads too.
    repos: BTreeSet<String>,
}

impl Github {
    /// Declares `factory.intake`, so a delivery can publish before the intake ever ran.
    pub fn new(secret: String, factory: &Factory) -> Result<Self, String> {
        let bus = Bus::open_default().map_err(|error| error.to_string())?;
        bus.declare(&output()).map_err(|error| error.to_string())?;
        let triggers = factory
            .triggers
            .iter()
            .filter(|(_, trigger)| trigger.value.source == "github")
            .map(|(name, trigger)| (name.clone(), trigger.value.clone()))
            .collect();
        let repos = factory
            .triggers
            .values()
            .filter_map(|trigger| trigger.value.repo.clone())
            .collect();
        Ok(Github {
            secret,
            triggers,
            repos,
        })
    }

    /// Verifies, maps and publishes one delivery. Every delivery Priory takes or drops on purpose
    /// gets 202, so GitHub does not retry it.
    pub fn receive(&self, bus: &Result<Bus, String>, request: &Request) -> Reply {
        if !self.signed(request) {
            return Reply::error(
                401,
                "X-Hub-Signature-256 is missing or does not match the body",
            );
        }
        let header = |name| {
            request
                .header(name)
                .and_then(|value| std::str::from_utf8(value).ok())
        };
        let (Some(event), Some(delivery)) = (header("x-github-event"), header("x-github-delivery"))
        else {
            return Reply::error(400, "send X-GitHub-Event and X-GitHub-Delivery");
        };
        let Ok(payload) = serde_json::from_slice::<Value>(request.body) else {
            return Reply::error(400, "the body is not JSON");
        };
        let observed = match self.observe(event, &payload, request.body) {
            Ok(Some(observed)) => observed,
            Ok(None) => return accepted("dropped"),
            Err(error) => return Reply::error(400, &error),
        };
        let bus = match bus {
            Ok(bus) => bus,
            Err(error) => return Reply::error(500, error),
        };
        let triggers: Vec<(&str, &Trigger)> = self
            .triggers
            .iter()
            .map(|(name, trigger)| (name.as_str(), trigger))
            .collect();
        let published = observed.message(&triggers).and_then(|mut message| {
            // GitHub redelivers for 3 days; an older body is a replay the change match may miss.
            if message.time.as_second() < now()?.as_second() - LOOKBACK_SECONDS {
                return Ok(false);
            }
            message.data["delivery"] = json!(delivery);
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

    /// Whether `X-Hub-Signature-256` is the HMAC-SHA256 of the body under the secret, compared in
    /// constant time.
    fn signed(&self, request: &Request) -> bool {
        let Some(sent) = request
            .header("x-hub-signature-256")
            .and_then(|value| value.strip_prefix(b"sha256="))
            .and_then(unhex)
        else {
            return false;
        };
        let mut mac =
            Hmac::<Sha256>::new_from_slice(self.secret.as_bytes()).expect("HMAC takes any key");
        mac.update(request.body);
        mac.verify_slice(&sent).is_ok()
    }

    /// The event a delivery reports, or none for an event, action or repository Priory does not
    /// take. Errs when a field the event needs is missing.
    fn observe(
        &self,
        event: &str,
        payload: &Value,
        body: &[u8],
    ) -> Result<Option<Observed>, String> {
        let repo = payload["repository"]["full_name"]
            .as_str()
            .unwrap_or_default();
        if !self.repos.contains(repo) {
            return Ok(None);
        }
        let action = payload["action"].as_str().unwrap_or_default();
        // The delivery GUID is not signed, so the id comes from the signed body.
        let signed = || format!("github:webhook:{event}:{}", hex(&Sha256::digest(body)));
        let issue = &payload["issue"];
        let pull = &payload["pull_request"];
        let sender = &payload["sender"]["login"];
        let on_issue = |time: &Value, actor: &Value, github_id: &Value| {
            json!({
                "repo": repo,
                "issue": issue["number"],
                "title": issue["title"],
                "url": issue["html_url"],
                "actor": actor,
                "time": time,
                "github_id": github_id,
            })
        };
        let on_pull = |url: &Value, time: &Value, actor: &Value| {
            json!({
                "repo": repo,
                "pull_request": pull["number"],
                "url": url,
                "head_branch": pull["head"]["ref"],
                "head_sha": pull["head"]["sha"],
                "base_branch": pull["base"]["ref"],
                "actor": actor,
                "time": time,
            })
        };
        let (kind, name, data) = match (event, action) {
            ("issues", "opened") => (
                OPENED,
                format!("github:{repo}:issues:{}:opened", issue["id"]),
                on_issue(&issue["created_at"], &issue["user"]["login"], &issue["id"]),
            ),
            ("issues", "labeled") => {
                let mut data = on_issue(&issue["updated_at"], sender, &Value::Null);
                data["label"] = payload["label"]["name"].clone();
                (LABELED, signed(), data)
            }
            ("pull_request", "opened" | "labeled" | "closed") => {
                let (kind, time, actor) = match action {
                    "opened" => (PR_OPENED, &pull["created_at"], &pull["user"]["login"]),
                    "labeled" => (PR_LABELED, &pull["updated_at"], sender),
                    _ => (PR_CLOSED, &pull["closed_at"], sender),
                };
                let mut data = on_pull(&pull["html_url"], time, actor);
                data["title"] = pull["title"].clone();
                data["github_id"] = Value::Null;
                match kind {
                    PR_LABELED => data["label"] = payload["label"]["name"].clone(),
                    PR_CLOSED => data["merged"] = pull["merged"].clone(),
                    _ => {}
                }
                (kind, signed(), data)
            }
            ("pull_request_review", "submitted") => {
                let review = &payload["review"];
                let mut data = on_pull(
                    &review["html_url"],
                    &review["submitted_at"],
                    &review["user"]["login"],
                );
                let state = review["state"].as_str().unwrap_or_default();
                data["state"] = json!(state.to_lowercase());
                data["body"] = json!(review["body"].as_str().unwrap_or_default());
                data["github_id"] = review["id"].clone();
                data["open"] = json!(pull["state"] == "open");
                let name = format!("github:{repo}:pulls/reviews:{}", review["id"]);
                (REVIEWED, name, data)
            }
            ("issue_comment" | "pull_request_review_comment", "created") => {
                return Ok(feedback(event, repo, payload));
            }
            _ => return Ok(None),
        };
        let number = data
            .get("issue")
            .unwrap_or(&data["pull_request"])
            .as_u64()
            .ok_or("the payload names no issue or pull request number")?;
        let text = |field: &str| data[field].as_str().is_some_and(|value| !value.is_empty());
        let fields: &[&str] = if kind == REVIEWED {
            &["url", "state"]
        } else {
            &["url", "title"]
        };
        if let Some(field) = fields.iter().find(|field| !text(field)) {
            return Err(format!("the payload lacks the event's `{field}`"));
        }
        if matches!(kind, OPENED | REVIEWED) && !data["github_id"].is_u64() {
            return Err("the payload lacks the event's id".to_owned());
        }
        let time = data["time"].as_str().unwrap_or_default();
        if time.parse::<Timestamp>().is_err() {
            return Err(format!("the payload's event time `{time}` is not RFC 3339"));
        }
        let label = data["label"].as_str().map(str::to_owned);
        Ok(Some(Observed {
            kind,
            name,
            repo: repo.to_owned(),
            number,
            label,
            baseline: false,
            data,
        }))
    }
}

/// A comment on a pull request, as the poll publishes it: none for a bot's, or for an issue's.
fn feedback(event: &str, repo: &str, payload: &Value) -> Option<Observed> {
    let found = &payload["comment"];
    if found["user"]["type"] == "Bot" || !found["id"].is_u64() || !found["body"].is_string() {
        return None;
    }
    let pull = &payload["pull_request"];
    if event == "pull_request_review_comment" {
        let branches = json!({
            "head_branch": pull["head"]["ref"],
            "head_sha": pull["head"]["sha"],
            "base_branch": pull["base"]["ref"],
            "open": pull["state"] == "open",
        });
        let number = pull["number"].as_u64()?;
        return Some(comment(
            THREAD_COMMENTED,
            repo,
            number,
            &branches,
            thread_comment(found)?,
        ));
    }
    let issue = &payload["issue"];
    issue.get("pull_request")?;
    let data = json!({
        "url": found["html_url"],
        "actor": found["user"]["login"],
        "body": found["body"],
        "time": found["created_at"],
        "github_id": found["id"],
    });
    // The issue in the payload names no branch: routing finds the task by its pull request.
    let branches = json!({ "open": issue["state"] == "open" });
    Some(comment(
        COMMENTED,
        repo,
        issue["number"].as_u64()?,
        &branches,
        data,
    ))
}

fn accepted(outcome: &str) -> Reply {
    Reply::json(202, json!({ "outcome": outcome }).to_string())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The bytes of a lowercase or uppercase hex string, or none when it is not one.
fn unhex(hex: &[u8]) -> Option<Vec<u8>> {
    if !hex.len().is_multiple_of(2) || !hex.iter().all(u8::is_ascii_hexdigit) {
        return None;
    }
    hex.chunks(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok())
        .collect()
}
