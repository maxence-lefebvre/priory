use super::{BASELINE, OUTPUT, Watch, dispatch, id, publish};
use crate::log::{self, Log};
use herdr_bus::{Message, Timestamp};
use priory_factory::factory::Trigger;
use serde_json::{Value, json};
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::io::Write;
use std::process::{Command, Stdio};

pub(super) const TOKEN: &str = "SLACK_BOT_TOKEN";
pub(super) const API: &str = "https://slack.com/api/";
pub(super) const POSTED: &str = "slack.message.posted";
pub(super) const REPLIED: &str = "slack.thread.replied";
const FILE_SHARE: &str = "file_share";
const BROADCAST: &str = "thread_broadcast";
/// How many messages a channel's first poll reads.
const FIRST: u64 = 50;
/// How many polls in a row a rate limit may leave a thread unread before the poll reports it.
const BEHIND: u32 = 5;

/// The bot Priory reads Slack as, the Slack triggers, and where each of their channels stands: the
/// `ts` of the newest message seen, or none while the channel is still in its baseline.
pub(super) struct Slack<'a> {
    token: String,
    user: String,
    bot: String,
    triggers: Vec<(&'a str, &'a Trigger)>,
    channels: BTreeMap<String, Option<String>>,
    /// Which watched thread the next poll reads first, modulo their count. It starts at the
    /// minute, so that successive `--once` runs rotate too.
    turn: i64,
    /// The display name of each user id `users.info` answered, for the life of the process.
    names: RefCell<BTreeMap<String, String>>,
    /// Whether a failing `users.info` was reported, so that it is reported once.
    unnamed: Cell<bool>,
    /// How many polls in a row a rate limit left each watched thread unread.
    unread: BTreeMap<(String, String), u32>,
}

impl<'a> Slack<'a> {
    /// Reads the token, asks Slack which bot it belongs to, and reads where each channel stands.
    pub(super) fn start(
        bus: &dyn Log,
        triggers: Vec<(&'a str, &'a Trigger)>,
    ) -> Result<Self, String> {
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
            turn: super::now()?.as_second().div_euclid(60),
            names: RefCell::default(),
            unnamed: Cell::new(false),
            unread: BTreeMap::new(),
        })
    }

    /// Publishes the new messages of each trigger channel and the replies in each watched thread.
    /// A message published again keeps its id, so the bus drops it. A failing channel publishes
    /// nothing and fails the poll once the rest is published.
    pub(super) fn poll(&mut self, bus: &dyn Log, watched: &[Watch]) -> Result<(), String> {
        let mut polled = Vec::new();
        let mut failed = Vec::new();
        for (channel, cursor) in &self.channels {
            match self.channel(channel, cursor.as_deref()) {
                Ok(found) => polled.push(found),
                Err(error) => failed.push(error),
            }
        }
        // A wait and a subscription can name one thread: it is read once, from the earlier time, at
        // the better rank.
        let mut earliest: BTreeMap<(&str, &str), (u8, Timestamp)> = BTreeMap::new();
        for watch in watched {
            let Some((channel, thread)) = watch
                .resource
                .strip_prefix("slack:")
                .and_then(|thread| thread.split_once(':'))
            else {
                continue;
            };
            let read = earliest
                .entry((channel, thread))
                .or_insert((watch.rank, watch.since));
            *read = (read.0.min(watch.rank), read.1.min(watch.since));
        }
        let mut ranked: BTreeMap<u8, Vec<(&str, &str, Timestamp)>> = BTreeMap::new();
        for ((channel, thread), (rank, since)) in earliest {
            ranked
                .entry(rank)
                .or_default()
                .push((channel, thread, since));
        }
        // Slack lets a new app read one thread a minute: open waits come first, then active tasks,
        // then ended ones, and each poll starts every rank at its next thread.
        let turn = self.turn;
        self.turn += 1;
        let threads: Vec<(&str, &str, Timestamp)> = ranked
            .into_values()
            .flat_map(|mut threads| {
                let start = turn.rem_euclid(threads.len() as i64) as usize;
                threads.rotate_left(start);
                threads
            })
            .collect();
        let mut replies = Vec::new();
        let mut limited = Vec::new();
        for (channel, thread, since) in &threads {
            // A deleted thread or a channel the bot left fails the poll once the rest is published. A
            // rate limit only delays the thread, until it has been unread for `BEHIND` polls.
            let key = ((*channel).to_owned(), (*thread).to_owned());
            match self.replies(channel, thread, *since) {
                Ok(found) => {
                    replies.extend(found);
                    limited.push((key, false));
                }
                Err(error) if error.ends_with(": ratelimited") => {
                    eprintln!("{error}");
                    limited.push((key, true));
                }
                Err(error) => {
                    failed.push(format!("thread {channel}:{thread}: {error}"));
                    limited.push((key, false));
                }
            }
        }
        let late = behind(&mut self.unread, &limited);
        if late > 0 {
            failed.push(format!(
                "{late} threads behind: Slack rate-limited their reads for more than {BEHIND} polls"
            ));
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
            // A reply also sent to the channel stays a reply: its thread's poll reads it.
            .filter(|message| {
                after(text(&message["ts"]))
                    && self.by_someone(message)
                    && text(&message["subtype"]) != BROADCAST
            })
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
        let messages = self.pages("conversations.replies", query, true)?;
        // Slack answers a reply's `ts` with that reply alone, so a wait on it would never fire.
        if messages.first().is_some_and(|first| {
            first
                .get("thread_ts")
                .is_some_and(|parent| text(parent) != thread)
        }) {
            return Err("not a thread's first message".into());
        }
        let mut replies = Vec::new();
        for message in messages {
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
                "actor": message["user"].as_str().map(|user| self.name(user)),
                "text": message["text"],
                "files": files(&message),
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

    /// The display name of `user`, asked of `users.info` once per process; the id itself when Slack
    /// does not answer, as without the `users:read` scope.
    fn name(&self, user: &str) -> String {
        if let Some(name) = self.names.borrow().get(user) {
            return name.clone();
        }
        let answer = match call(&self.token, "users.info", &[("user", user.to_owned())]) {
            Ok(answer) => answer,
            Err(error) => {
                if !self.unnamed.replace(true) {
                    eprintln!("{error}: Slack replies name their authors by id");
                }
                return user.to_owned();
            }
        };
        let profile = &answer["user"]["profile"];
        let name = [
            &profile["display_name"],
            &profile["real_name"],
            &answer["user"]["name"],
        ]
        .into_iter()
        .map(text)
        .find(|name| !name.is_empty())
        .unwrap_or(user)
        .to_owned();
        self.names
            .borrow_mut()
            .insert(user.to_owned(), name.clone());
        name
    }

    /// Whether a person wrote `message`: not the bot itself, and not a join or another subtype
    /// than an upload or a reply also sent to the channel.
    fn by_someone(&self, message: &Value) -> bool {
        message
            .get("subtype")
            .is_none_or(|subtype| [FILE_SHARE, BROADCAST].contains(&text(subtype)))
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
            "files": files(message),
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

/// The names of the files uploaded with `message`.
fn files(message: &Value) -> Vec<&str> {
    let files = message["files"].as_array().into_iter().flatten();
    files.map(|file| text(&file["name"])).collect()
}

pub(super) fn text(value: &Value) -> &str {
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
    if !micros.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(invalid());
    }
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
fn resume(bus: &dyn Log, channels: &mut BTreeMap<String, Option<String>>) -> log::Result<()> {
    let mut before = None;
    // shortcut: reads back to each channel's newest marker, once per start; a stored cursor would
    // make this constant once the output is large.
    while channels.values().any(Option::is_none) {
        let page = bus.messages(OUTPUT, before, 500)?;
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

/// One JSON POST to `url`, under `token` when given as its variable's name and value. Everything
/// goes to `curl` on its standard input, so neither the token nor the URL shows in a process list.
pub(super) fn post(token: Option<(&str, &str)>, url: &str, body: &Value) -> Result<Value, String> {
    send(token, url, body).map_err(|(error, _)| error)
}

/// As `post`, with the seconds of Slack's `Retry-After` header beside an error that has one.
pub(super) fn send(
    token: Option<(&str, &str)>,
    url: &str,
    body: &Value,
) -> Result<Value, (String, Option<u64>)> {
    // A raw newline would end the config line, so control characters are refused, not quoted.
    if [url, token.map_or("", |(_, token)| token)]
        .iter()
        .any(|value| value.chars().any(char::is_control))
    {
        let error = "Slack call refused: a control character in its URL or token";
        return Err((error.to_owned(), None));
    }
    let quote = |value: &str| format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""));
    let mut config = String::new();
    if let Some((_, token)) = token {
        config += &format!(
            "header = {}\n",
            quote(&format!("Authorization: Bearer {token}"))
        );
    }
    config += "header = \"Content-Type: application/json; charset=utf-8\"\n";
    config += &format!(
        "data = {}\nurl = {}\n",
        quote(&body.to_string()),
        quote(url)
    );
    let method = url.strip_prefix(API).unwrap_or("response_url");
    let failed = |error: &str| {
        let error = match token {
            Some((name, token)) => error.replace(token, &format!("<{name}>")),
            None => error.to_owned(),
        };
        // A response_url is a credential too.
        let error = error.replace(url, &format!("<{method}>"));
        format!("Slack {method} failed: {error}")
    };
    let mut child = Command::new("curl")
        .args([
            "--disable",
            "--silent",
            "--show-error",
            "--connect-timeout",
            "10",
            "--max-time",
            "30",
            "--max-filesize",
            "1000000",
            "--proto",
            "=https",
            "--fail-with-body",
            "--write-out",
            "\n%header{retry-after}",
            "--config",
            "-",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| (format!("Slack {method}: curl: {error}"), None))?;
    child
        .stdin
        .take()
        .expect("a piped stdin")
        .write_all(config.as_bytes())
        .map_err(|error| (format!("Slack {method}: curl: {error}"), None))?;
    let output = child
        .wait_with_output()
        .map_err(|error| (format!("Slack {method}: curl: {error}"), None))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    // `--write-out` ends the output with a line holding `Retry-After`, empty without the header.
    let (stdout, retry) = stdout.rsplit_once('\n').unwrap_or((&stdout, ""));
    let retry = retry.trim().parse().ok();
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let error = failed(&format!("{} {}", stderr.trim(), stdout.trim()));
        return Err((error, retry));
    }
    // A response_url answers `ok` as text, the Web API answers JSON.
    let answer: Value =
        serde_json::from_str(stdout).unwrap_or(json!({"ok": stdout.trim() == "ok"}));
    if answer["ok"] != true {
        return Err((failed(text(&answer["error"])), retry));
    }
    Ok(answer)
}

/// Counts the polls in a row that a rate limit left each thread of `polled` unread, forgets the
/// threads it no longer names, and returns how many went unread for more than `BEHIND` polls.
fn behind(
    unread: &mut BTreeMap<(String, String), u32>,
    polled: &[((String, String), bool)],
) -> usize {
    let mut counted = BTreeMap::new();
    for (thread, limited) in polled {
        if *limited {
            counted.insert(thread.clone(), unread.get(thread).copied().unwrap_or(0) + 1);
        }
    }
    *unread = counted;
    unread.values().filter(|polls| **polls > BEHIND).count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn a_thread_a_rate_limit_left_unread_for_more_than_five_polls_is_behind_until_it_reads() {
        let thread = ("C0123".to_owned(), "1.0".to_owned());
        let other = ("C0123".to_owned(), "2.0".to_owned());
        let mut unread = BTreeMap::new();
        let poll = [(thread.clone(), true), (other.clone(), false)];
        let counts: Vec<usize> = (0..6).map(|_| behind(&mut unread, &poll)).collect();
        assert_eq!(counts, [0, 0, 0, 0, 0, 1]);
        assert_eq!(behind(&mut unread, &[(thread, false), (other, true)]), 0);
    }

    #[test]
    fn a_ts_with_a_multibyte_character_in_its_fraction_is_refused() {
        assert!(time("1.00000\u{e9}").is_err());
    }

    proptest! {
        #[test]
        fn any_ts_is_read_or_refused_without_panicking(ts in prop_oneof!["\\PC{0,40}", "[0-9-]{0,12}\\.[0-9\u{e9}\u{1f4a5}-]{0,12}"]) {
            let _ = time(&ts);
        }
    }
}
