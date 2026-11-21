use super::gh::gh;
use super::{BASELINE, OUTPUT, dispatch, id, publish};
use crate::log::{self, Log};
use herdr_bus::{Message, Timestamp};
use priory_factory::factory::Trigger;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub(super) const REQUESTED: &str = "github.pull_request.review_requested";
pub(super) const REMOVED: &str = "github.pull_request.review_request_removed";
const STREAM: &str = "github.review_requests:";
/// A search page's size.
const PAGE: usize = 100;
/// How many pages GitHub search returns at most: 1,000 results.
const PAGES: u64 = 10;

/// The review requests of each reviewer the triggers name, diffed poll to poll as Linear views are:
/// each stream's marker holds the pull requests that asked at the last poll, or none while the
/// stream is still in its baseline.
pub(super) struct Reviews<'a> {
    triggers: Vec<(&'a str, &'a Trigger)>,
    streams: BTreeMap<String, Option<Value>>,
}

impl<'a> Reviews<'a> {
    pub(super) fn start(
        bus: &dyn Log,
        triggers: Vec<(&'a str, &'a Trigger)>,
    ) -> Result<Self, String> {
        let mut streams: BTreeMap<String, Option<Value>> = triggers
            .iter()
            .filter_map(|(_, trigger)| trigger.reviewer.as_deref())
            .map(|reviewer| (format!("{STREAM}{reviewer}"), None))
            .collect();
        let interrupted = resume(bus, &mut streams).map_err(|error| error.to_string())?;
        for (stream, events) in interrupted {
            // A stream still in its baseline polls its first page again.
            let Some(Some(marker)) = streams.get_mut(&stream) else {
                continue;
            };
            // A crash may have stopped between an event and its task requests.
            for event in events.iter().filter(|event| event.data["baseline"] != true) {
                dispatch(bus, &triggers, event)?;
            }
            *marker = recover(marker, &events);
            publish(bus, &marker_message(&stream, marker))?;
        }
        Ok(Reviews { triggers, streams })
    }

    /// Polls every reviewer, and returns the error of each that failed. A failed read changes
    /// nothing for its reviewer: it is not an empty set. The others still publish, and a publish
    /// error ends the poll.
    pub(super) fn poll(&mut self, bus: &dyn Log, now: Timestamp) -> Result<Vec<String>, String> {
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
            publish(bus, &marker_message(&stream, &marker))?;
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
        let teams: BTreeSet<&str> = self
            .triggers
            .iter()
            .filter(|(_, trigger)| trigger.reviewer.as_deref() == Some(reviewer))
            .flat_map(|(_, trigger)| trigger.teams.iter().flatten())
            .map(String::as_str)
            .collect();
        let mut members = requested(reviewer, &teams)?;
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

/// The open pull requests that ask `reviewer` for a review, directly or through one of `teams`, in
/// every repository.
fn requested(reviewer: &str, teams: &BTreeSet<&str>) -> Result<BTreeMap<String, Value>, String> {
    // `priory check` keeps a reviewer and a team to letters, digits, `-`, `_` and one `/`.
    let mut members = search(&format!("user-review-requested%3A{reviewer}"))?;
    for team in teams {
        let team = team.replace('/', "%2F");
        members.extend(search(&format!("team-review-requested%3A{team}"))?);
    }
    Ok(members)
}

/// The open pull requests a search `qualifier` matches, every page of 100 up to GitHub's 1000.
fn search(qualifier: &str) -> Result<BTreeMap<String, Value>, String> {
    let mut members = BTreeMap::new();
    let mut read = 0;
    for page in 1..=PAGES {
        let path =
            format!("search/issues?q=is%3Apr+is%3Aopen+{qualifier}&per_page={PAGE}&page={page}");
        let answer: Value = gh(&path, &[])?;
        let items = answer["items"]
            .as_array()
            .ok_or_else(|| format!("gh api {path}: no items"))?;
        read += items.len();
        members.extend(pulls(&path, items)?);
        let complete = answer["incomplete_results"] == false;
        let total = answer["total_count"].as_u64();
        if complete && total == Some(read as u64) {
            return Ok(members);
        }
        // A set read short, or changed between pages, is unknown, not smaller.
        if !complete || items.len() < PAGE || total.is_none_or(|total| read as u64 > total) {
            return Err(format!(
                "gh api {path}: GitHub returned {read} of {} results, so the set is unknown",
                answer["total_count"]
            ));
        }
    }
    Err(format!(
        "GitHub search {qualifier}: more than {} results, past what search returns",
        PAGES * PAGE as u64
    ))
}

/// The members a search page's `items` name, each `owner/name#n`.
fn pulls(path: &str, items: &[Value]) -> Result<Vec<(String, Value)>, String> {
    items
        .iter()
        // A repository that is not `owner/name` could lead a task's worktree out of Priory's home.
        .filter(|item| {
            item["repository_url"]
                .as_str()
                .and_then(|url| url.split_once("/repos/"))
                .is_none_or(|(_, repo)| priory_runtime::runtime::check_repo(repo).is_ok())
        })
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

fn marker_message(stream: &str, marker: &Value) -> Message {
    let name = format!("{BASELINE}:{stream}:{}", marker["position"]);
    Message::new(BASELINE, marker.clone())
        .id(id(&name))
        .subject(stream)
}

/// The marker a poll stopped by a crash would have published: `events` are what it published,
/// oldest first. Polling on from it never rebuilds those events, nor reuses their ids.
fn recover(marker: &Value, events: &[Message]) -> Value {
    let mut members: BTreeMap<String, Value> = marker["members"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|member| (member_key(member), member.clone()))
        .collect();
    for event in events {
        let mut member = event.data.clone();
        let fields = member.as_object_mut().expect("an object");
        for field in ["reviewer", "reviewed", "time", "baseline", "triggers"] {
            fields.remove(field);
        }
        if event.kind == REQUESTED {
            members.insert(member_key(&member), member);
        } else {
            members.remove(&member_key(&member));
        }
    }
    json!({
        "stream": marker["stream"],
        "position": marker["position"].as_u64().unwrap_or_default() + 1,
        "members": members.into_values().collect::<Vec<_>>(),
    })
}

/// Fills each stream with its newest marker, read back newest first, and returns the events of
/// each stream published after that marker, oldest first: those of a poll a crash stopped.
fn resume(
    bus: &dyn Log,
    streams: &mut BTreeMap<String, Option<Value>>,
) -> log::Result<BTreeMap<String, Vec<Message>>> {
    let mut after: BTreeMap<String, Vec<Message>> = BTreeMap::new();
    let mut before = None;
    // shortcut: reads back to each stream's newest marker, once per start, as Linear's do.
    while streams.values().any(Option::is_none) {
        let page = bus.messages(OUTPUT, before, 500)?;
        let Some(oldest) = page.last() else {
            break;
        };
        before = Some(oldest.seq);
        for stored in &page {
            let message = &stored.message;
            let reviewer = message.data["reviewer"].as_str().unwrap_or_default();
            let stream = match message.kind.as_str() {
                BASELINE => message.data["stream"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
                REQUESTED | REMOVED => format!("{STREAM}{reviewer}"),
                _ => continue,
            };
            let Some(marker @ None) = streams.get_mut(&stream) else {
                continue;
            };
            if message.kind == BASELINE {
                *marker = Some(message.data.clone());
            } else {
                after.entry(stream).or_default().insert(0, message.clone());
            }
        }
    }
    Ok(after)
}
