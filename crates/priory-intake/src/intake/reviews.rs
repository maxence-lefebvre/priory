use super::gh::gh;
use super::{BASELINE, OUTPUT, dispatch, id, publish};
use herdr_bus::{Bus, Message, MessageQuery, Timestamp};
use priory_factory::factory::Trigger;
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub(super) const REQUESTED: &str = "github.pull_request.review_requested";
pub(super) const REMOVED: &str = "github.pull_request.review_request_removed";
const STREAM: &str = "github.review_requests:";

/// The review requests of each reviewer the triggers name, diffed poll to poll as Linear views are:
/// each stream's marker holds the pull requests that asked at the last poll, or none while the
/// stream is still in its baseline.
pub(super) struct Reviews<'a> {
    triggers: Vec<(&'a str, &'a Trigger)>,
    streams: BTreeMap<String, Option<Value>>,
}

impl<'a> Reviews<'a> {
    pub(super) fn start(bus: &Bus, triggers: Vec<(&'a str, &'a Trigger)>) -> Result<Self, String> {
        let mut streams: BTreeMap<String, Option<Value>> = triggers
            .iter()
            .filter_map(|(_, trigger)| trigger.reviewer.as_deref())
            .map(|reviewer| (format!("{STREAM}{reviewer}"), None))
            .collect();
        resume(bus, &mut streams).map_err(|error| error.to_string())?;
        Ok(Reviews { triggers, streams })
    }

    /// Polls every reviewer, and returns the error of each that failed. A failed read changes
    /// nothing for its reviewer: it is not an empty set. The others still publish, and a publish
    /// error ends the poll.
    pub(super) fn poll(&mut self, bus: &Bus, now: Timestamp) -> Result<Vec<String>, String> {
        let mut failed = Vec::new();
        let streams: Vec<String> = self.streams.keys().cloned().collect();
        for stream in streams {
            let (events, marker) = match self.diff(&stream, now) {
                Ok(Some(diff)) => diff,
                Ok(None) => continue,
                Err(error) => {
                    failed.push(error);
                    continue;
                }
            };
            for event in events {
                publish(bus, &event)?;
                dispatch(bus, &self.triggers, &event)?;
            }
            let name = format!("{BASELINE}:{stream}:{}", marker["position"]);
            let message = Message::new(BASELINE, marker.clone())
                .id(id(&name))
                .subject(&stream);
            publish(bus, &message)?;
            self.streams.insert(stream, Some(marker));
        }
        Ok(failed)
    }

    /// The events of the pull requests that entered and left `stream`'s set since its marker, and
    /// its new marker; none when the set is unchanged.
    fn diff(&self, stream: &str, now: Timestamp) -> Result<Option<(Vec<Message>, Value)>, String> {
        let reviewer = stream.trim_start_matches(STREAM);
        let marker = self.streams[stream].as_ref();
        let before: BTreeMap<String, Value> = marker
            .and_then(|marker| marker["members"].as_array())
            .into_iter()
            .flatten()
            .map(|member| (member_key(member), member.clone()))
            .collect();
        let mut members = requested(reviewer)?;
        for (key, member) in &mut members {
            let since = before
                .get(key)
                .map_or(json!(now.to_string()), |old| old["since"].clone());
            member["since"] = since;
        }
        let baseline = marker.is_none();
        if !baseline && members == before {
            return Ok(None);
        }
        let version = marker.map_or(0, |marker| {
            marker["position"].as_u64().unwrap_or_default() + 1
        });
        let mut changes = Vec::new();
        for (key, member) in &members {
            if !before.contains_key(key) {
                changes.push((REQUESTED, key, member.clone()));
            }
        }
        for (key, member) in &before {
            if !members.contains_key(key) {
                let mut data = member.clone();
                data["reviewed"] = json!(reviewed(member, reviewer)?);
                changes.push((REMOVED, key, data));
            }
        }
        // The version keeps a re-request apart from the first request of the same pull request.
        let events = changes
            .into_iter()
            .map(|(kind, key, mut data)| {
                data["reviewer"] = json!(reviewer);
                data["time"] = json!(now.to_string());
                data["baseline"] = json!(baseline);
                data["triggers"] = json!(self.matching(kind, reviewer, baseline));
                let url = data["url"].as_str().unwrap_or_default().to_string();
                Message::new(kind, data)
                    .id(id(&format!("{stream}:{version}:{key}:{kind}")))
                    .subject(key.clone())
                    .time(now)
                    .link("pull_request", &url)
            })
            .collect();
        let marker = json!({
            "stream": stream,
            "position": version,
            "members": members.into_values().collect::<Vec<_>>(),
        });
        Ok(Some((events, marker)))
    }

    fn matching(&self, kind: &str, reviewer: &str, baseline: bool) -> Vec<&'a str> {
        self.triggers
            .iter()
            .filter(|(_, trigger)| {
                format!("github.{}", trigger.on) == kind
                    && trigger.reviewer.as_deref() == Some(reviewer)
                    && (!baseline || trigger.backfill == Some(true))
            })
            .map(|(name, _)| *name)
            .collect()
    }
}

/// `owner/name#42`, the subject of a member's events.
fn member_key(member: &Value) -> String {
    format!(
        "{}#{}",
        member["repo"].as_str().unwrap_or_default(),
        member["pull_request"]
    )
}

/// The open pull requests that ask `reviewer` directly for a review, in every repository.
fn requested(reviewer: &str) -> Result<BTreeMap<String, Value>, String> {
    // `priory check` keeps a reviewer to letters, digits and dashes, so only `:` needs escaping.
    let path = format!(
        "search/issues?q=is%3Apr+is%3Aopen+user-review-requested%3A{reviewer}&per_page=100"
    );
    let answer: Value = gh(&path, &[])?;
    let items = answer["items"]
        .as_array()
        .ok_or_else(|| format!("gh api {path}: no items"))?;
    // shortcut: one page of 100; page through `total_count` if someone ever has more requests.
    if answer["incomplete_results"] != false || answer["total_count"] != json!(items.len()) {
        return Err(format!(
            "gh api {path}: GitHub returned {} of {} results, so the set is unknown",
            items.len(),
            answer["total_count"]
        ));
    }
    items
        .iter()
        .map(|item| {
            let repo = item["repository_url"]
                .as_str()
                .and_then(|url| url.split_once("/repos/"))
                .map(|(_, repo)| repo)
                .filter(|_| item["number"].is_u64())
                .ok_or_else(|| {
                    format!("gh api {path}: a result without its repository or number")
                })?;
            let member = json!({
                "repo": repo,
                "pull_request": item["number"],
                "title": item["title"],
                "url": item["html_url"],
                "author": item["user"]["login"],
            });
            Ok((member_key(&member), member))
        })
        .collect()
}

/// Whether `reviewer` submitted a review of `member` since the intake first found the request.
/// GitHub drops a request once its reviewer reviews, so that is why it left the set.
fn reviewed(member: &Value, reviewer: &str) -> Result<bool, String> {
    let repo = member["repo"].as_str().unwrap_or_default();
    let (owner, name) = repo.split_once('/').unwrap_or_default();
    // Pending reviews have no `submittedAt`: only a submitted one ends the request.
    let query = "query($owner: String!, $name: String!, $number: Int!, $author: String!) { \
        repository(owner: $owner, name: $name) { pullRequest(number: $number) { \
        reviews(author: $author, last: 20) { nodes { submittedAt } } } } }";
    let answer: Result<Value, String> = gh(
        "graphql",
        &[
            "-f",
            &format!("owner={owner}"),
            "-f",
            &format!("name={name}"),
            "-F",
            &format!("number={}", member["pull_request"]),
            "-f",
            &format!("author={reviewer}"),
            "-f",
            &format!("query={query}"),
        ],
    );
    // A pull request or repository the `gh` user can no longer read has no review to find.
    let answer = match answer {
        Err(error) if error.contains("Could not resolve to a") => return Ok(false),
        answer => answer?,
    };
    let reviews = answer
        .pointer("/data/repository/pullRequest/reviews/nodes")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            format!(
                "gh api graphql: no reviews of {repo}#{}",
                member["pull_request"]
            )
        })?;
    let parse = |time: &Value| {
        time.as_str()
            .and_then(|time| time.parse::<Timestamp>().ok())
    };
    let since = parse(&member["since"]);
    Ok(reviews
        .iter()
        .filter_map(|review| parse(&review["submittedAt"]))
        .any(|submitted| since.is_none_or(|since| submitted >= since)))
}

/// Fills each stream with its newest marker, read back newest first.
fn resume(bus: &Bus, streams: &mut BTreeMap<String, Option<Value>>) -> herdr_bus::Result<()> {
    let mut before = None;
    // shortcut: reads back to each stream's newest marker, once per start, as Linear's do.
    while streams.values().any(Option::is_none) {
        let page = bus.messages(&MessageQuery {
            output: Some(OUTPUT.into()),
            before,
            limit: 500,
            ..MessageQuery::default()
        })?;
        let Some(oldest) = page.last() else {
            break;
        };
        before = Some(oldest.seq);
        for stored in page.iter().filter(|stored| stored.message.kind == BASELINE) {
            let stream = stored.message.data["stream"].as_str().unwrap_or_default();
            if let Some(marker @ None) = streams.get_mut(stream) {
                *marker = Some(stored.message.data.clone());
            }
        }
    }
    Ok(())
}
