use super::{BASELINE, OUTPUT, Watch, dispatch, id, publish};
use herdr_bus::{Bus, Message, MessageQuery, Timestamp};
use priory_factory::factory::Trigger;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::io::Write;
use std::process::{Command, Stdio};

pub(super) const TOKEN: &str = "SLACK_BOT_TOKEN";
const API: &str = "https://slack.com/api/";
pub(super) const POSTED: &str = "slack.message.posted";
pub(super) const REPLIED: &str = "slack.thread.replied";
/// How many messages a channel's first poll reads.
const FIRST: u64 = 50;

/// The bot Priory reads Slack as, the Slack triggers, and where each of their channels stands: the
/// `ts` of the newest message seen, or none while the channel is still in its baseline.
pub(super) struct Slack<'a> {
    token: String,
    user: String,
    bot: String,
    triggers: Vec<(&'a str, &'a Trigger)>,
    channels: BTreeMap<String, Option<String>>,
}

impl<'a> Slack<'a> {
    /// Reads the token, asks Slack which bot it belongs to, and reads where each channel stands.
    pub(super) fn start(bus: &Bus, triggers: Vec<(&'a str, &'a Trigger)>) -> Result<Self, String> {
        let token = std::env::var(TOKEN)
            .ok()
            .filter(|token| !token.is_empty())
            .ok_or_else(|| {
                format!("{TOKEN} is not set: the Slack triggers and waits need a bot token")
            })?;
        let mut channels: BTreeMap<String, Option<String>> = triggers
            .iter()
            .filter_map(|(_, trigger)| trigger.channel.clone())
            .map(|channel| (channel, None))
            .collect();
        resume(bus, &mut channels).map_err(|error| error.to_string())?;
        let me = call(&token, "auth.test", &[])?;
        Ok(Slack {
            user: text(&me["user_id"]).to_owned(),
            bot: text(&me["bot_id"]).to_owned(),
            token,
            triggers,
            channels,
        })
    }

    /// Publishes the new messages of each trigger channel and the replies in each watched thread.
    /// A message published again keeps its id, so the bus drops it. A failing channel publishes
    /// nothing and fails the poll once the rest is published.
    pub(super) fn poll(&mut self, bus: &Bus, watched: &[Watch]) -> Result<(), String> {
        let mut polled = Vec::new();
        let mut failed = Vec::new();
        for (channel, cursor) in &self.channels {
            match self.channel(channel, cursor.as_deref()) {
                Ok(found) => polled.push(found),
                Err(error) => failed.push(error),
            }
        }
        let threads: Vec<(&str, &str, Timestamp)> = watched
            .iter()
            .filter_map(|watch| {
                let thread = watch.resource.strip_prefix("slack:")?;
                let (channel, thread) = thread.split_once(':')?;
                Some((channel, thread, watch.since))
            })
            .collect();
        // Slack lets a new app read one thread a minute: each minute starts at the next thread.
        let minute = super::now()?.as_second().div_euclid(60);
        let start = minute.rem_euclid(threads.len().max(1) as i64) as usize;
        let mut replies = Vec::new();
        for (channel, thread, since) in threads[start..].iter().chain(&threads[..start]) {
            // A deleted thread or a channel the bot left fails only its own wait's poll.
            match self.replies(channel, thread, *since) {
                Ok(found) => replies.extend(found),
                Err(error) => eprintln!("{error}"),
            }
        }
        for (channel, events, top) in polled {
            for event in &events {
                publish(bus, event)?;
                dispatch(bus, &self.triggers, event)?;
            }
            let cursor = self.channels.get_mut(&channel).expect("a polled channel");
            if cursor.is_none() || top != *cursor {
                let position = top.clone().unwrap_or_else(|| "0".into());
                publish(bus, &marker(&channel, &position))?;
                *cursor = Some(position);
            }
        }
        for reply in &replies {
            publish(bus, reply)?;
        }
        if failed.is_empty() {
            return Ok(());
        }
        Err(failed.join("\n"))
    }

    /// The messages of `channel` after `cursor`, oldest first, and the newest `ts` seen.
    fn channel(
        &self,
        channel: &str,
        cursor: Option<&str>,
    ) -> Result<(String, Vec<Message>, Option<String>), String> {
        let mut query = vec![("channel", channel.to_owned())];
        match cursor {
            None => query.push(("limit", FIRST.to_string())),
            Some(cursor) => query.extend([("oldest", cursor.to_owned()), ("limit", "200".into())]),
        }
        let messages = self.pages("conversations.history", query, cursor.is_some())?;
        let after = |ts: &str| cursor.is_none_or(|cursor| seconds(ts) > seconds(cursor));
        let top = messages
            .iter()
            .map(|message| text(&message["ts"]))
            .chain(cursor)
            .max_by(|a, b| seconds(a).total_cmp(&seconds(b)))
            .map(str::to_owned);
        let mut events: Vec<Message> = messages
            .iter()
            .filter(|message| after(text(&message["ts"])) && self.by_someone(message))
            .map(|message| self.posted(channel, message, cursor.is_none()))
            .collect::<Result<_, _>>()?;
        events.sort_by_key(|event| event.time);
        Ok((channel.to_owned(), events, top))
    }

    /// The replies in `thread` posted at or after `since`, by anyone but the bot itself.
    fn replies(
        &self,
        channel: &str,
        thread: &str,
        since: Timestamp,
    ) -> Result<Vec<Message>, String> {
        let oldest = format!("{}.{:06}", since.as_second(), since.subsec_microsecond());
        let query = vec![
            ("channel", channel.to_owned()),
            ("ts", thread.to_owned()),
            ("oldest", oldest),
            ("inclusive", "true".into()),
            ("limit", "200".into()),
        ];
        let mut replies = Vec::new();
        for message in self.pages("conversations.replies", query, true)? {
            let ts = text(&message["ts"]);
            let time = time(ts)?;
            if ts == thread || time < since || !self.by_someone(&message) {
                continue;
            }
            let data = json!({
                "channel": channel,
                "thread_ts": thread,
                "ts": ts,
                "user": message["user"],
                "text": message["text"],
                "time": time.to_string(),
                "baseline": false,
                "triggers": [],
            });
            replies.push(
                Message::new(REPLIED, data)
                    .id(id(&format!("slack:{channel}:{ts}")))
                    .subject(format!("{channel}:{thread}"))
                    .time(time),
            );
        }
        Ok(replies)
    }

    /// Whether a person wrote `message`: not the bot itself, and not a join or another subtype.
    fn by_someone(&self, message: &Value) -> bool {
        message.get("subtype").is_none()
            && text(&message["user"]) != self.user
            && message
                .get("bot_id")
                .is_none_or(|bot| text(bot) != self.bot)
    }

    /// A channel message, with the triggers it matches.
    fn posted(&self, channel: &str, message: &Value, baseline: bool) -> Result<Message, String> {
        let ts = text(&message["ts"]);
        let time = time(ts)?;
        let mut data = json!({
            "channel": channel,
            "ts": ts,
            "user": message["user"],
            "text": message["text"],
            "time": time.to_string(),
            "baseline": baseline,
        });
        data["triggers"] = json!(self.matching(channel, text(&message["text"]), baseline));
        Ok(Message::new(POSTED, data)
            .id(id(&format!("slack:{channel}:{ts}")))
            .subject(format!("{channel}:{ts}"))
            .time(time))
    }

    fn matching(&self, channel: &str, words: &str, baseline: bool) -> Vec<&'a str> {
        let words = words.to_lowercase();
        self.triggers
            .iter()
            .filter(|(_, trigger)| {
                trigger.channel.as_deref() == Some(channel)
                    && format!("slack.{}", trigger.on) == POSTED
                    && trigger
                        .filter
                        .iter()
                        .flatten()
                        .all(|(_, text)| words.contains(&text.to_lowercase()))
                    && (!baseline || trigger.backfill == Some(true))
            })
            .map(|(name, _)| *name)
            .collect()
    }

    /// The messages of a Slack list method, every page or only the first.
    fn pages(
        &self,
        method: &str,
        mut query: Vec<(&str, String)>,
        all: bool,
    ) -> Result<Vec<Value>, String> {
        let mut messages = Vec::new();
        loop {
            let answer = call(&self.token, method, &query)?;
            messages.extend(answer["messages"].as_array().into_iter().flatten().cloned());
            let next = text(&answer["response_metadata"]["next_cursor"]);
            if !all || next.is_empty() {
                return Ok(messages);
            }
            query.retain(|(name, _)| *name != "cursor");
            query.push(("cursor", next.to_owned()));
        }
    }
}

fn text(value: &Value) -> &str {
    value.as_str().unwrap_or_default()
}

fn seconds(ts: &str) -> f64 {
    ts.parse().unwrap_or_default()
}

/// The instant a Slack `ts` names: seconds since the epoch, with microseconds after the dot.
fn time(ts: &str) -> Result<Timestamp, String> {
    let invalid = || format!("Slack ts `{ts}` is not seconds.microseconds");
    let (seconds, micros) = ts.split_once('.').ok_or_else(invalid)?;
    let seconds: i64 = seconds.parse().map_err(|_| invalid())?;
    let micros: i32 = format!("{micros:0<6}")[..6]
        .parse()
        .map_err(|_| invalid())?;
    Timestamp::new(seconds, micros * 1000).map_err(|_| invalid())
}

fn marker(channel: &str, position: &str) -> Message {
    let stream = stream(channel);
    Message::new(BASELINE, json!({ "stream": stream, "position": position }))
        .id(id(&format!("{BASELINE}:{stream}:{position}")))
        .subject(stream)
}

fn stream(channel: &str) -> String {
    format!("slack.channel:{channel}")
}

/// Fills each channel with its newest marker's position, read back newest first. Messages a poll
/// published after it are read again and dropped by the bus as duplicates.
fn resume(bus: &Bus, channels: &mut BTreeMap<String, Option<String>>) -> herdr_bus::Result<()> {
    let mut before = None;
    // shortcut: reads back to each channel's newest marker, once per start; a stored cursor would
    // make this constant once the output is large.
    while channels.values().any(Option::is_none) {
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
            let data = &stored.message.data;
            let channel = text(&data["stream"]).strip_prefix("slack.channel:");
            if let Some(cursor @ None) = channel.and_then(|channel| channels.get_mut(channel)) {
                *cursor = Some(text(&data["position"]).to_owned());
            }
        }
    }
    Ok(())
}

/// One call to Slack's Web API. The token goes to `curl` on its standard input, so that it never
/// shows in a process list, and `--disable` keeps a `.curlrc` from tracing it.
fn call(token: &str, method: &str, query: &[(&str, String)]) -> Result<Value, String> {
    let mut child =
        Command::new("curl")
            .args([
                "--disable",
                "--silent",
                "--show-error",
                "--connect-timeout",
                "10",
                "--max-time",
                "60",
                "--fail-with-body",
                "--get",
                "--config",
                "-",
            ])
            .args(query.iter().flat_map(|(name, value)| {
                ["--data-urlencode".to_owned(), format!("{name}={value}")]
            }))
            .arg(format!("{API}{method}"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("Slack {method}: curl: {error}"))?;
    let escaped = token.replace('\\', "\\\\").replace('"', "\\\"");
    child
        .stdin
        .take()
        .expect("a piped stdin")
        .write_all(format!("header = \"Authorization: Bearer {escaped}\"\n").as_bytes())
        .map_err(|error| format!("Slack {method}: curl: {error}"))?;
    let output = child
        .wait_with_output()
        .map_err(|error| format!("Slack {method}: curl: {error}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let failed = |error: &str| {
        format!(
            "Slack {method} failed: {}",
            error.replace(token, "<SLACK_BOT_TOKEN>")
        )
    };
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(failed(&format!("{} {}", stderr.trim(), stdout.trim())));
    }
    let answer: Value =
        serde_json::from_str(&stdout).map_err(|error| format!("Slack {method}: {error}"))?;
    if answer["ok"] != true {
        return Err(failed(text(&answer["error"])));
    }
    Ok(answer)
}
