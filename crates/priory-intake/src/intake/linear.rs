use super::{BASELINE, OUTPUT, dispatch, id, publish};
use herdr_bus::{Bus, Message, MessageQuery, Timestamp};
use priory_factory::factory::Trigger;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::io::Write;
use std::process::{Command, Stdio};

pub(super) const KEY: &str = "LINEAR_API_KEY";
const ENDPOINT: &str = "https://api.linear.app/graphql";
pub(super) const ENTERED: &str = "linear.issue.entered";
pub(super) const LEFT: &str = "linear.issue.left";
pub(super) const CREATED: &str = "linear.issue.created";
pub(super) const STATE_CHANGED: &str = "linear.issue.state_changed";
pub(super) const COMMENTED: &str = "linear.comment.created";
pub(super) const BLOCKERS: &str = "linear.issue.blockers";
const ISSUES: &str = "linear.issues";
const COMMENTS: &str = "linear.comments";
/// How many issues or comments a stream's first poll reads.
const FIRST: u64 = 50;

const ISSUE: &str = "fragment issue on Issue { id identifier title url createdAt updatedAt \
    team { key } project { name } state { name } labels { nodes { name } } creator { displayName } }";
/// The issues that block an issue, kept small: Linear multiplies a query's cost by page sizes.
const BLOCKING: &str = "inverseRelations(first: 20) { nodes { type issue { identifier state { type } } } \
    pageInfo { hasNextPage } }";
const PAGE: &str = "pageInfo { hasNextPage endCursor }";

/// The Linear streams the triggers need, and where each stands: its last marker's data, or none
/// while it is still in its baseline.
pub(super) struct Linear<'a> {
    key: String,
    triggers: Vec<(&'a str, &'a Trigger)>,
    streams: BTreeMap<String, Option<Value>>,
}

/// What one poll of a stream found: its new events, oldest first, and its new marker data.
pub(super) struct Polled {
    stream: String,
    events: Vec<Message>,
    marker: Value,
}

impl<'a> Linear<'a> {
    /// Reads the API key and where each stream stands. Every Linear trigger polls issues and
    /// comments, so that a task receives its issue's events whatever started it.
    pub(super) fn start(bus: &Bus, triggers: Vec<(&'a str, &'a Trigger)>) -> Result<Self, String> {
        let key = std::env::var(KEY)
            .ok()
            .filter(|key| !key.is_empty())
            .ok_or_else(|| {
                format!("{KEY} is not set: the Linear triggers need a Linear API key")
            })?;
        let mut streams: BTreeMap<String, Option<Value>> = triggers
            .iter()
            .filter_map(|(_, trigger)| trigger.view.as_deref())
            .map(|view| (view_stream(view), None))
            .collect();
        streams.insert(ISSUES.into(), None);
        streams.insert(COMMENTS.into(), None);
        let interrupted = resume(bus, &mut streams).map_err(|error| error.to_string())?;
        for (stream, events) in interrupted {
            // A stream still in its baseline polls its first page again.
            let Some(Some(marker)) = streams.get_mut(&stream) else {
                continue;
            };
            let newest = events
                .last()
                .expect("an interrupted poll published an event");
            // A crash may have stopped between an event and its task requests.
            if newest.data["baseline"] != true {
                dispatch(bus, &triggers, newest)?;
            }
            *marker = recover(marker, &events);
            publish(bus, &marker_message(&stream, marker))?;
        }
        Ok(Linear {
            key,
            triggers,
            streams,
        })
    }

    /// Queries every stream first, so that a failing query publishes nothing.
    pub(super) fn fetch(&self) -> Result<Vec<Polled>, String> {
        let now = Timestamp::now();
        let mut polled = Vec::new();
        for (stream, marker) in &self.streams {
            polled.push(match stream.as_str() {
                ISSUES => self.issues(marker.as_ref())?,
                COMMENTS => self.comments(marker.as_ref())?,
                _ => self.view(stream, marker.as_ref(), now)?,
            });
        }
        Ok(polled)
    }

    pub(super) fn publish_all(&mut self, bus: &Bus, polled: Vec<Polled>) -> Result<(), String> {
        for Polled {
            stream,
            events,
            marker,
        } in polled
        {
            let current = self.streams.get_mut(&stream).expect("a polled stream");
            let seen = current
                .as_ref()
                .and_then(|marker| marker["seen"].as_array())
                .cloned()
                .unwrap_or_default();
            let current = current.as_ref() != Some(&marker);
            for event in events {
                // Read again at the stream's position, and handled by an earlier poll.
                if seen.contains(&json!(event.id.to_string())) {
                    continue;
                }
                publish(bus, &event)?;
                dispatch(bus, &self.triggers, &event)?;
            }
            if current {
                publish(bus, &marker_message(&stream, &marker))?;
                self.streams.insert(stream, Some(marker));
            }
        }
        Ok(())
    }

    /// The issues that entered and left a view since its last marker, which holds its members.
    fn view(&self, stream: &str, marker: Option<&Value>, now: Timestamp) -> Result<Polled, String> {
        let view = stream.trim_start_matches("linear.view:");
        let query = format!(
            "query View($view: String!, $after: String) {{ customView(id: $view) {{ \
             issues(first: 100, after: $after) {{ nodes {{ ...issue }} {PAGE} }} }} }} {ISSUE}"
        );
        let issues = self.nodes(&query, json!({ "view": view }), "/customView/issues", true)?;
        let members: BTreeMap<String, Value> = issues
            .iter()
            .map(|issue| {
                (
                    issue["id"].as_str().unwrap_or_default().to_string(),
                    fields(issue),
                )
            })
            .collect();
        let before: BTreeMap<String, Value> = marker
            .and_then(|marker| marker["members"].as_array())
            .into_iter()
            .flatten()
            .map(|member| {
                (
                    member["linear_id"].as_str().unwrap_or_default().to_string(),
                    member.clone(),
                )
            })
            .collect();
        let baseline = marker.is_none();
        let version = marker.map_or(0, |marker| {
            marker["position"].as_u64().unwrap_or_default() + 1
        });
        let entered = members
            .iter()
            .filter(|(id, _)| !before.contains_key(*id))
            .map(|(_, member)| (ENTERED, member));
        let left = before
            .iter()
            .filter(|(id, _)| !members.contains_key(*id))
            .map(|(_, member)| (LEFT, member));
        let events = entered
            .chain(left)
            .map(|(kind, member)| {
                let mut data = member.clone();
                data["view"] = json!(view);
                data["time"] = json!(now.to_string());
                let name = format!("{stream}:{version}:{}:{kind}", text(&member["linear_id"]));
                self.message(kind, &name, data, baseline)
            })
            .collect::<Result<_, _>>()?;
        let changed = baseline || members != before;
        Ok(Polled {
            stream: stream.into(),
            events,
            marker: json!({
                "stream": stream,
                "position": if changed { version } else { version - 1 },
                "members": members.into_values().collect::<Vec<_>>(),
            }),
        })
    }

    /// Issues created and state changes since the stream's position, the last `updatedAt` seen.
    fn issues(&self, marker: Option<&Value>) -> Result<Polled, String> {
        let since = marker.and_then(|marker| marker["position"].as_str());
        let query = format!(
            "query Issues($filter: IssueFilter, $first: Int!, $after: String) {{ \
             issues(filter: $filter, first: $first, after: $after, orderBy: updatedAt, \
             includeArchived: true) {{ nodes {{ \
             ...issue history(first: 50, orderBy: createdAt) {{ nodes {{ id createdAt \
             fromState {{ name }} toState {{ name }} actor {{ displayName }} }} }} \
             {BLOCKING} relations(first: 20) {{ nodes {{ type relatedIssue {{ id }} }} }} }} {PAGE} }} }} {ISSUE}"
        );
        let issues = self.nodes(&query, page(since, "updatedAt"), "/issues", since.is_some())?;
        let mut events = Vec::new();
        for issue in &issues {
            if after(since, &issue["createdAt"]) {
                let mut data = fields(issue);
                data["actor"] = issue["creator"]["displayName"].clone();
                data["blockers"] = blockers(issue);
                data["time"] = issue["createdAt"].clone();
                let name = format!("linear:issue:{}:created", text(&issue["id"]));
                events.push(self.message(CREATED, &name, data, since.is_none())?);
            }
            let changes = issue["history"]["nodes"].as_array().into_iter().flatten();
            for change in changes.filter(|change| {
                change["fromState"].is_object()
                    && change["toState"].is_object()
                    && after(since, &change["createdAt"])
            }) {
                let mut data = fields(issue);
                data["linear_id"] = change["id"].clone();
                data["from"] = change["fromState"]["name"].clone();
                data["to"] = change["toState"]["name"].clone();
                data["actor"] = change["actor"]["displayName"].clone();
                data["blockers"] = blockers(issue);
                data["time"] = change["createdAt"].clone();
                let name = format!("linear:history:{}", text(&change["id"]));
                events.push(self.message(STATE_CHANGED, &name, data, since.is_none())?);
            }
        }
        // A blocker that closes changes the issues it blocks, which Linear does not update.
        let blocking: Vec<(&Value, &str)> = issues
            .iter()
            .flat_map(|issue| {
                let relations = issue["relations"]["nodes"].as_array().into_iter().flatten();
                relations
                    .filter(|relation| relation["type"] == "blocks")
                    .map(move |relation| (issue, text(&relation["relatedIssue"]["id"])))
            })
            .collect();
        let ids: Vec<&str> = blocking.iter().map(|(_, id)| *id).collect();
        let mut blocked = BTreeMap::new();
        for ids in ids.chunks(50) {
            let query = format!(
                "query Blocked($ids: [ID!], $first: Int!) {{ \
                 issues(filter: {{ id: {{ in: $ids }} }}, first: $first) {{ nodes {{ \
                 ...issue {BLOCKING} }} {PAGE} }} }} {ISSUE}"
            );
            let found = self.nodes(&query, json!({ "ids": ids, "first": 50 }), "/issues", false)?;
            blocked.extend(
                found
                    .into_iter()
                    .map(|issue| (text(&issue["id"]).to_owned(), issue)),
            );
        }
        for (blocker, id) in blocking {
            let Some(issue) = blocked.get(id) else {
                continue;
            };
            let mut data = fields(issue);
            data["blockers"] = blockers(issue);
            data["blocker"] = blocker["identifier"].clone();
            data["time"] = blocker["updatedAt"].clone();
            let name = format!(
                "linear:blockers:{id}:{}:{}",
                text(&blocker["id"]),
                text(&blocker["updatedAt"])
            );
            events.push(self.message(BLOCKERS, &name, data, since.is_none())?);
        }
        Ok(polled(ISSUES, events, &issues, "updatedAt", since))
    }

    /// Comments created since the stream's position, the last `createdAt` seen.
    fn comments(&self, marker: Option<&Value>) -> Result<Polled, String> {
        let since = marker.and_then(|marker| marker["position"].as_str());
        let query = format!(
            "query Comments($filter: CommentFilter, $first: Int!, $after: String) {{ \
             comments(filter: $filter, first: $first, after: $after, orderBy: createdAt) {{ nodes {{ \
             id body url createdAt user {{ displayName }} issue {{ ...issue }} }} {PAGE} }} }} {ISSUE}"
        );
        let comments = self.nodes(
            &query,
            page(since, "createdAt"),
            "/comments",
            since.is_some(),
        )?;
        let mut events = Vec::new();
        // A comment on a project update or a document has no issue.
        for comment in comments
            .iter()
            .filter(|comment| comment["issue"].is_object() && after(since, &comment["createdAt"]))
        {
            let mut data = fields(&comment["issue"]);
            data["linear_id"] = comment["id"].clone();
            data["url"] = comment["url"].clone();
            data["body"] = comment["body"].clone();
            data["actor"] = comment["user"]["displayName"].clone();
            data["time"] = comment["createdAt"].clone();
            let name = format!("linear:comment:{}", text(&comment["id"]));
            events.push(self.message(COMMENTED, &name, data, since.is_none())?);
        }
        Ok(polled(COMMENTS, events, &comments, "createdAt", since))
    }

    /// A Linear event, with the triggers it matches.
    fn message(
        &self,
        kind: &str,
        name: &str,
        mut data: Value,
        baseline: bool,
    ) -> Result<Message, String> {
        let raw = data["time"].as_str().unwrap_or_default();
        let time: Timestamp = raw
            .parse()
            .map_err(|error| format!("Linear time `{raw}`: {error}"))?;
        data["baseline"] = json!(baseline);
        data["triggers"] = json!(self.matching(kind, &data, baseline));
        let subject = data["issue"].as_str().unwrap_or_default().to_string();
        let url = data["url"].as_str().unwrap_or_default().to_string();
        Ok(Message::new(kind, data)
            .id(id(name))
            .subject(subject)
            .time(time)
            .link("issue", &url))
    }

    fn matching(&self, kind: &str, data: &Value, baseline: bool) -> Vec<&'a str> {
        let filtered = |trigger: &Trigger| {
            let state = data.get("to").unwrap_or(&data["state"]);
            trigger
                .filter
                .iter()
                .flatten()
                .all(|(field, value)| match field.as_str() {
                    "team" => data["team"] == *value,
                    "state" => *state == *value,
                    "label" => data["labels"]
                        .as_array()
                        .is_some_and(|labels| labels.iter().any(|label| label == value)),
                    _ => false,
                })
        };
        self.triggers
            .iter()
            .filter(|(_, trigger)| {
                format!("linear.{}", trigger.on) == kind
                    && trigger
                        .view
                        .as_deref()
                        .is_none_or(|view| data["view"] == view)
                    && filtered(trigger)
                    && (!baseline || trigger.backfill == Some(true))
            })
            .map(|(name, _)| *name)
            .collect()
    }

    /// The nodes of the connection at `path`, every page or only the first.
    fn nodes(
        &self,
        query: &str,
        mut variables: Value,
        path: &str,
        all: bool,
    ) -> Result<Vec<Value>, String> {
        let mut nodes = Vec::new();
        loop {
            let data = graphql(&self.key, query, &variables)?;
            let connection = data
                .pointer(path)
                .ok_or_else(|| format!("Linear answered no {path}"))?;
            nodes.extend(
                connection["nodes"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .cloned(),
            );
            let next = &connection["pageInfo"];
            if !all || next["hasNextPage"] != true {
                return Ok(nodes);
            }
            variables["after"] = next["endCursor"].clone();
        }
    }
}

/// The variables of a time stream's query: its first page on a baseline, every node at or after
/// `since` otherwise. Linear compares to the millisecond, so `gte` rereads the last node.
fn page(since: Option<&str>, field: &str) -> Value {
    match since {
        None => json!({ "first": FIRST, "filter": null }),
        Some(since) => json!({ "first": 100, "filter": { field: { "gte": since } } }),
    }
}

fn after(since: Option<&str>, time: &Value) -> bool {
    let parse = |raw: &str| raw.parse::<Timestamp>().ok();
    let time = time.as_str().and_then(parse);
    since
        .and_then(parse)
        .is_none_or(|since| time.is_none_or(|time| time >= since))
}

/// A time stream's poll: its events oldest first, and its position, the latest `field` of
/// `nodes`. A first page with nothing means Linear holds none yet, so the stream starts at 1970.
/// The marker lists the events at that position, which the next poll reads again.
fn polled(
    stream: &str,
    mut events: Vec<Message>,
    nodes: &[Value],
    field: &str,
    since: Option<&str>,
) -> Polled {
    events.sort_by_key(|event| event.time);
    let latest = nodes
        .iter()
        .filter_map(|node| node[field].as_str())
        .chain(since)
        .filter_map(|raw| Some((raw.parse::<Timestamp>().ok()?, raw)))
        .max()
        .map_or_else(
            || Timestamp::UNIX_EPOCH.to_string(),
            |(_, raw)| raw.to_string(),
        );
    let position = latest.parse::<Timestamp>().ok();
    let seen: Vec<String> = events
        .iter()
        .filter(|event| Some(event.time) == position)
        .map(|event| event.id.to_string())
        .collect();
    Polled {
        stream: stream.into(),
        events,
        marker: json!({ "stream": stream, "position": latest, "seen": seen }),
    }
}

fn marker_message(stream: &str, marker: &Value) -> Message {
    let position = &marker["position"];
    let position = position
        .as_str()
        .map_or(position.to_string(), str::to_owned);
    let name = format!("{BASELINE}:{stream}:{position}:{}", marker["seen"]);
    Message::new(BASELINE, marker.clone())
        .id(id(&name))
        .subject(stream)
}

/// The marker a poll stopped by a crash would have published: `events` are what it published,
/// oldest first. Polling on from it never rebuilds those events, nor reuses their view position.
fn recover(marker: &Value, events: &[Message]) -> Value {
    let mut marker = marker.clone();
    if let Some(before) = marker["members"].as_array() {
        let mut members: BTreeMap<String, Value> = before
            .iter()
            .map(|member| (text(&member["linear_id"]).to_string(), member.clone()))
            .collect();
        for event in events {
            let mut member = event.data.clone();
            let fields = member.as_object_mut().expect("an object");
            for field in ["view", "time", "baseline", "triggers"] {
                fields.remove(field);
            }
            let id = text(&member["linear_id"]).to_string();
            if event.kind == ENTERED {
                members.insert(id, member);
            } else {
                members.remove(&id);
            }
        }
        marker["position"] = json!(marker["position"].as_u64().unwrap_or_default() + 1);
        marker["members"] = json!(members.into_values().collect::<Vec<_>>());
        return marker;
    }
    let newest = events.iter().map(|event| event.time).max();
    let at: Vec<&Message> = events
        .iter()
        .filter(|event| Some(event.time) == newest)
        .collect();
    let position = text(&marker["position"]).parse::<Timestamp>().ok();
    // Still at the old instant: the events the old marker lists stay handled.
    let mut seen: Vec<Value> = marker["seen"]
        .as_array()
        .filter(|_| newest == position)
        .cloned()
        .unwrap_or_default();
    seen.extend(at.iter().map(|event| json!(event.id.to_string())));
    marker["position"] = at[0].data["time"].clone();
    marker["seen"] = json!(seen);
    marker
}

fn text(value: &Value) -> &str {
    value.as_str().unwrap_or_default()
}

fn view_stream(view: &str) -> String {
    format!("linear.view:{view}")
}

/// The fields every Linear event carries about its issue.
fn fields(issue: &Value) -> Value {
    let labels: Vec<&Value> = issue["labels"]["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|label| &label["name"])
        .collect();
    json!({
        "issue": issue["identifier"],
        "linear_id": issue["id"],
        "title": issue["title"],
        "url": issue["url"],
        "team": issue["team"]["key"],
        "project": issue["project"]["name"],
        "state": issue["state"]["name"],
        "labels": labels,
    })
}

/// The identifiers of the issues that block `issue` and are neither completed nor canceled. Null
/// when Linear has more relations than one page holds, since one of those could block it.
fn blockers(issue: &Value) -> Value {
    let relations = &issue["inverseRelations"];
    if relations["pageInfo"]["hasNextPage"] != false {
        return Value::Null;
    }
    let open: Vec<&Value> = relations["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|relation| {
            relation["type"] == "blocks"
                && !matches!(
                    relation["issue"]["state"]["type"].as_str(),
                    Some("completed" | "canceled")
                )
        })
        .map(|relation| &relation["issue"]["identifier"])
        .collect();
    json!(open)
}

/// Fills each stream with its newest marker, read back newest first, and returns the events of
/// each stream published after that marker, oldest first: those of a poll a crash stopped.
fn resume(
    bus: &Bus,
    streams: &mut BTreeMap<String, Option<Value>>,
) -> herdr_bus::Result<BTreeMap<String, Vec<Message>>> {
    let mut after: BTreeMap<String, Vec<Message>> = BTreeMap::new();
    let mut before = None;
    // shortcut: reads back to each stream's newest marker, once per start; a stored cursor would
    // make this constant once the output is large.
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
        for stored in &page {
            let message = &stored.message;
            let stream = match message.kind.as_str() {
                BASELINE => text(&message.data["stream"]).to_string(),
                ENTERED | LEFT => view_stream(text(&message.data["view"])),
                CREATED | STATE_CHANGED | BLOCKERS => ISSUES.to_string(),
                COMMENTED => COMMENTS.to_string(),
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

/// One query to Linear's GraphQL API. The key goes to `curl` on its standard input, so that it
/// never shows in a process list, and `--disable` keeps a `.curlrc` from tracing it.
fn graphql(key: &str, query: &str, variables: &Value) -> Result<Value, String> {
    let operation = query.split(['(', ' ']).nth(1).unwrap_or("query");
    let body = json!({ "query": query, "variables": variables }).to_string();
    let mut child = Command::new("curl")
        .args([
            "--disable",
            "--silent",
            "--show-error",
            "--connect-timeout",
            "10",
            "--max-time",
            "60",
            "--fail-with-body",
            "--config",
            "-",
        ])
        .args([
            "--header",
            "Content-Type: application/json",
            "--data-binary",
        ])
        .arg(&body)
        .arg(ENDPOINT)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("Linear {operation}: curl: {error}"))?;
    let escaped = key.replace('\\', "\\\\").replace('"', "\\\"");
    let config = format!("header = \"Authorization: {escaped}\"\n");
    child
        .stdin
        .take()
        .expect("a piped stdin")
        .write_all(config.as_bytes())
        .map_err(|error| format!("Linear {operation}: curl: {error}"))?;
    let output = child
        .wait_with_output()
        .map_err(|error| format!("Linear {operation}: curl: {error}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let error = format!("{} {}", stderr.trim(), stdout.trim());
        return Err(format!(
            "Linear {operation} failed: {}",
            error.replace(key, "<LINEAR_API_KEY>")
        ));
    }
    let answer: Value =
        serde_json::from_str(&stdout).map_err(|error| format!("Linear {operation}: {error}"))?;
    if let Some(errors) = answer["errors"].as_array() {
        let messages: Vec<&str> = errors
            .iter()
            .filter_map(|error| error["message"].as_str())
            .collect();
        return Err(format!(
            "Linear {operation} failed: {}",
            messages.join("; ").replace(key, "<LINEAR_API_KEY>")
        ));
    }
    Ok(answer["data"].clone())
}
