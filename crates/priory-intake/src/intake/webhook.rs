use super::pulls::{comment, thread_comment};
use super::{
    COMMENTED, LABELED, LOOKBACK_SECONDS, OPENED, Observed, PR_CLOSED, PR_LABELED, PR_OPENED,
    REVIEWED, THREAD_COMMENTED, claim, dispatch, now, output, publish_delivered, reported,
};
use crate::log::{self, Log};
use herdr_bus::Timestamp;
use hmac::{Hmac, KeyInit, Mac};
use priory_factory::factory::{EventLog, Factory, Trigger};
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
    /// The log a delivery publishes on, opened again while a worker's open failed.
    log: EventLog,
}

impl Github {
    /// Declares `factory.intake`, so a delivery can publish before the intake ever ran.
    pub fn new(secret: String, factory: &Factory) -> Result<Self, String> {
        let bus = log::open(factory.log()).map_err(|error| error.to_string())?;
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
            log: factory.log().clone(),
        })
    }

    /// Verifies, maps and publishes one delivery. Every delivery Priory takes or drops on purpose
    /// gets 202, so GitHub does not retry it.
    pub fn receive(&self, bus: &mut Result<Box<dyn Log>, String>, request: &Request) -> Reply {
        if !self.signed(request) {
            super::counted("github", "webhook", "refused");
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
        let bus = match opened(bus, || log::open(&self.log)) {
            Ok(bus) => bus,
            Err(error) => return Reply::error(500, &error),
        };
        let triggers: Vec<(&str, &Trigger)> = self
            .triggers
            .iter()
            .map(|(name, trigger)| (name.as_str(), trigger))
            .collect();
        let published = observed
            .message(&triggers, super::gh::viewer)
            .and_then(|mut message| {
                // GitHub redelivers for 3 days; an older body is a replay the change match may miss.
                if message.time.as_second() < now()?.as_second() - LOOKBACK_SECONDS {
                    return Ok(false);
                }
                message.data["delivery"] = json!(delivery);
                let _claim = claim()?;
                if reported(bus, &message)? {
                    super::counted("github", "webhook", "duplicate");
                    return Ok(false);
                }
                publish_delivered(bus, &message, "github")?;
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
            let mut data = json!({
                "repo": repo,
                "pull_request": pull["number"],
                "url": url,
                "head_branch": pull["head"]["ref"],
                "head_sha": pull["head"]["sha"],
                "base_branch": pull["base"]["ref"],
                "actor": actor,
                "time": time,
            });
            head_repo(&mut data, pull);
            data
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
                    PR_CLOSED => {
                        data["merged"] = pull["merged"].clone();
                        // GitHub names a test merge commit on an unmerged pull request too.
                        data["merge_commit"] = match pull["merged"] == true {
                            true => pull["merge_commit_sha"].clone(),
                            false => Value::Null,
                        };
                        data["body"] = json!(pull["body"].as_str().unwrap_or_default());
                    }
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
                data["commit"] = review["commit_id"].clone();
                data["github_id"] = review["id"].clone();
                super::pulls::associate(&mut data, &review["author_association"]);
                data["open"] = json!(pull["state"] == "open");
                author(&mut data, &pull["user"]["login"]);
                if review["user"]["type"] == "Bot" {
                    data["bot"] = json!(true);
                }
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
        let mut branches = json!({
            "head_branch": pull["head"]["ref"],
            "head_sha": pull["head"]["sha"],
            "base_branch": pull["base"]["ref"],
            "open": pull["state"] == "open",
        });
        head_repo(&mut branches, pull);
        author(&mut branches, &pull["user"]["login"]);
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
    let mut data = json!({
        "url": found["html_url"],
        "actor": found["user"]["login"],
        "body": found["body"],
        "time": found["created_at"],
        "github_id": found["id"],
    });
    super::pulls::associate(&mut data, &found["author_association"]);
    // The issue in the payload names no branch: routing finds the task by its pull request.
    let mut branches = json!({ "open": issue["state"] == "open" });
    author(&mut branches, &issue["user"]["login"]);
    Some(comment(
        COMMENTED,
        repo,
        issue["number"].as_u64()?,
        &branches,
        data,
    ))
}

/// Sets `head_repo` from the payload's `pull` when it names the head's repository.
fn head_repo(data: &mut Value, pull: &Value) {
    if let Some(head) = pull["head"]["repo"]["full_name"].as_str() {
        data["head_repo"] = json!(head);
    }
}

/// Sets `author`, the login that opened the pull request, when the payload names one.
fn author(data: &mut Value, login: &Value) {
    if login.is_string() {
        data["author"] = login.clone();
    }
}

/// The log `bus` holds, or the one `open` opens in its place: an open that failed, as against a
/// database down when the worker started, is tried again and kept once it succeeds.
pub(super) fn opened(
    bus: &mut Result<Box<dyn Log>, String>,
    open: impl FnOnce() -> log::Result<Box<dyn Log>>,
) -> Result<&dyn Log, String> {
    if bus.is_err() {
        *bus = open().map_err(|error| error.to_string());
    }
    bus.as_deref().map_err(String::clone)
}

fn accepted(outcome: &str) -> Reply {
    Reply::json(202, json!({ "outcome": outcome }).to_string())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The bytes of a lowercase or uppercase hex string, or none when it is not one.
pub(super) fn unhex(hex: &[u8]) -> Option<Vec<u8>> {
    if !hex.len().is_multiple_of(2) || !hex.iter().all(u8::is_ascii_hexdigit) {
        return None;
    }
    hex.chunks(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intake::fuzz::json_value;
    use proptest::prelude::*;

    fn github() -> Github {
        let trigger = serde_json::from_value(
            json!({ "source": "github", "repo": "o/r", "on": "issues.labeled", "workflow": "w" }),
        )
        .unwrap();
        Github {
            secret: "s".to_owned(),
            triggers: vec![("t".to_owned(), trigger)],
            repos: BTreeSet::from(["o/r".to_owned()]),
            log: EventLog::default(),
        }
    }

    /// A delivery of `event` on pull request 3 of `o/r`, which `max` opened, with `extra` merged in.
    fn on_pull(event: &str, extra: &Value) -> Observed {
        let mut payload = json!({
            "action": if event == "pull_request_review" { "submitted" } else { "created" },
            "repository": { "full_name": "o/r" },
            "pull_request": {
                "number": 3, "state": "open", "user": { "login": "max" },
                "head": { "ref": "h", "sha": "s" }, "base": { "ref": "main" },
            },
            "issue": {
                "number": 3, "state": "open", "user": { "login": "max" },
                "pull_request": { "url": "u" },
            },
        });
        for (key, value) in extra.as_object().unwrap() {
            payload[key] = value.clone();
        }
        let body = payload.to_string();
        github()
            .observe(event, &payload, body.as_bytes())
            .unwrap()
            .unwrap()
    }

    fn comment_by(login: &str, kind: &str) -> Value {
        json!({ "comment": {
            "id": 9, "body": "b", "html_url": "https://github.com/o/r/pull/3#c", "created_at":
            "2026-01-01T00:00:00Z", "path": "p", "line": 1, "author_association": "MEMBER",
            "user": { "login": login, "type": kind },
        } })
    }

    fn feedback_trigger(on: &str, filter: &Value) -> Trigger {
        serde_json::from_value(
            json!({ "source": "github", "repo": "o/r", "on": on, "filter": filter, "workflow": "w" }),
        )
        .unwrap()
    }

    #[test]
    fn a_delivered_review_or_comment_names_the_pull_request_s_author_and_a_bot_reviewer() {
        let review = on_pull(
            "pull_request_review",
            &json!({ "review": {
                "id": 5, "state": "commented", "body": "", "commit_id": "s",
                "html_url": "https://github.com/o/r/pull/3#r", "submitted_at": "2026-01-01T00:00:00Z",
                "author_association": "MEMBER", "user": { "login": "rayon-bot[bot]", "type": "Bot" },
            } }),
        );
        let thread = on_pull("pull_request_review_comment", &comment_by("ana", "User"));
        let conversation = on_pull("issue_comment", &comment_by("ana", "User"));

        assert_eq!(review.data["author"], "max");
        assert_eq!(review.data["bot"], true);
        assert_eq!(thread.data["author"], "max");
        assert_eq!(conversation.data["author"], "max");
        assert!(thread.data.get("bot").is_none());
    }

    #[test]
    fn a_feedback_trigger_skips_a_closed_pull_request_and_a_marked_comment_by_the_login_gh_runs_as()
    {
        let mine = feedback_trigger("pull_request.comment", &json!({ "author": "max" }));
        let threads = feedback_trigger("pull_request_review_comment", &json!({ "author": "max" }));
        let triggers = [("mine", &mine), ("threads", &threads)];
        let matched = |observed: &Observed, viewer: crate::intake::Viewer| {
            observed.message(&triggers, viewer).unwrap().data["triggers"].clone()
        };
        let marked = |login: &str| {
            let mut comment = comment_by(login, "User");
            comment["comment"]["body"] = json!("Fixed in abc.\n\n<!-- agent-comment -->");
            on_pull("issue_comment", &comment)
        };
        let ana = on_pull("issue_comment", &comment_by("ana", "User"));
        let max = on_pull("issue_comment", &comment_by("max", "User"));
        let mut closed = on_pull("pull_request_review_comment", &comment_by("ana", "User"));
        closed.data["open"] = json!(false);

        assert_eq!(matched(&ana, || Some("max")), json!(["mine"]));
        // The person shares the agents' login: only the marker makes a comment an agent's.
        assert_eq!(matched(&max, || Some("max")), json!(["mine"]));
        assert_eq!(matched(&marked("max"), || Some("max")), json!([]));
        assert_eq!(matched(&marked("ana"), || Some("max")), json!(["mine"]));
        assert_eq!(matched(&closed, || None), json!([]));
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(512))]

        #[test]
        fn a_delivery_of_any_shape_is_observed_or_refused_without_panicking(
            event in prop_oneof![
                Just("issues"), Just("pull_request"), Just("pull_request_review"),
                Just("issue_comment"), Just("pull_request_review_comment"), Just("ping"),
            ],
            action in prop_oneof![
                Just("opened"), Just("labeled"), Just("closed"), Just("submitted"),
                Just("created"), Just("deleted"),
            ],
            fields in proptest::collection::vec(json_value(), 8),
            repository in prop_oneof![Just(json!({ "full_name": "o/r" })), json_value()],
        ) {
            let [issue, pull, review, comment, label, sender, user, extra] = <[Value; 8]>::try_from(fields).unwrap();
            let payload = json!({
                "action": action, "repository": repository, "issue": issue, "pull_request": pull,
                "review": review, "comment": comment, "label": label,
                "sender": sender, "user": user, "extra": extra,
            });
            let body = payload.to_string();
            if let Ok(Some(observed)) = github().observe(event, &payload, body.as_bytes()) {
                let _ = observed.message(&[], || None);
            }
        }

        #[test]
        fn a_valid_delivery_with_one_field_replaced_is_observed_or_refused_without_panicking(
            event in prop_oneof![Just("pull_request"), Just("issues"), Just("pull_request_review")],
            field in prop_oneof![
                Just("id"), Just("number"), Just("html_url"), Just("title"), Just("created_at"),
                Just("state"), Just("user"), Just("head"), Just("body"), Just("merged"),
            ],
            replacement in json_value(),
            action in prop_oneof![Just("opened"), Just("closed"), Just("submitted"), Just("labeled")],
        ) {
            let node = json!({
                "id": 7, "number": 3, "html_url": "https://github.com/o/r/pull/3", "title": "t",
                "created_at": "2026-01-01T00:00:00Z", "updated_at": "2026-01-01T00:00:00Z",
                "closed_at": "2026-01-01T00:00:00Z", "submitted_at": "2026-01-01T00:00:00Z",
                "state": "open", "user": { "login": "u" }, "merged": true, "body": "b",
                "head": { "ref": "h", "sha": "s" }, "base": { "ref": "main" },
            });
            let mut changed = node.clone();
            changed[field] = replacement;
            let payload = json!({
                "action": action, "repository": { "full_name": "o/r" }, "sender": { "login": "u" },
                "label": { "name": "l" }, "issue": changed, "pull_request": changed,
                "review": changed, "comment": changed,
            });
            let body = payload.to_string();
            if let Ok(Some(observed)) = github().observe(event, &payload, body.as_bytes()) {
                let _ = observed.message(&[], || None);
            }
        }

        #[test]
        fn any_signature_text_is_unhexed_or_refused_without_panicking(
            text in proptest::collection::vec(any::<u8>(), 0..200),
        ) {
            if let Some(bytes) = unhex(&text) {
                prop_assert_eq!(hex(&bytes).into_bytes(), text.to_ascii_lowercase());
            }
        }
    }

    #[test]
    fn a_log_that_failed_to_open_opens_on_a_later_delivery_and_stays_open() {
        let dir = std::env::temp_dir().join(format!("priory-opened-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("bus.db");
        let mut bus: Result<Box<dyn Log>, String> = Err("the database is down".to_owned());

        let down = opened(&mut bus, || Err("the database is still down".into())).err();
        let up = opened(&mut bus, || Ok(Box::new(herdr_bus::Bus::open(&file)?))).is_ok();
        let kept = opened(&mut bus, || panic!("an open log is opened again")).is_ok();

        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(down.as_deref(), Some("the database is still down"));
        assert!(up && kept);
    }
}
