//! The watches each runner posts on the log, which the intake polls for in place of a state file.

use super::{OUTPUT, WATCH_POSTED, Watch};
use crate::log::{self, Delivery, Log};
use herdr_bus::{Message, Timestamp};
use jiff::SignedDuration;
use std::collections::{BTreeMap, BTreeSet};

/// The consumer the intake reads the log as.
pub const CONSUMER: &str = "priory.intake";
/// How long a runner's watches last past its last posting. A running daemon posts daily.
const KEEP: SignedDuration = SignedDuration::from_hours(7 * 24);
const PAGE: usize = 500;

/// The newest posting of each runner, by its time and its watches.
#[derive(Default)]
pub(super) struct Postings {
    newest: BTreeMap<String, (Timestamp, Vec<Watch>)>,
}

impl Postings {
    /// Reads the log back, newest first, to the newest posting of each of `runners`, or of any
    /// runner when the factory has fewer than two, and at most `KEEP` back. Then moves the
    /// consumer past what it read, so each poll reads only what came since.
    pub(super) fn start(
        bus: &dyn Log,
        runners: &BTreeSet<String>,
        now: Timestamp,
    ) -> log::Result<Self> {
        let mut postings = Self::default();
        let mut before = None;
        let mut newest = None;
        loop {
            let page = bus.messages(OUTPUT, before, PAGE)?;
            let Some(oldest) = page.last() else {
                break;
            };
            before = Some(oldest.seq);
            newest = newest.or_else(|| page.first().cloned());
            for stored in &page {
                postings.take(&stored.message, runners);
            }
            let found = match runners.len() {
                0 | 1 => !postings.newest.is_empty(),
                _ => runners
                    .iter()
                    .all(|runner| postings.newest.contains_key(runner)),
            };
            if found || now.duration_since(oldest.message.time) > KEEP {
                break;
            }
        }
        if let Some(stored) = newest {
            let delivery = Delivery {
                output: OUTPUT.to_owned(),
                seq: stored.seq,
                message: stored.message,
                logged: None,
            };
            bus.ack(CONSUMER, &delivery)?;
        }
        Ok(postings)
    }

    /// Reads every message past the consumer's cursor and keeps the postings among them.
    pub(super) fn read(&mut self, bus: &dyn Log, runners: &BTreeSet<String>) -> log::Result<()> {
        loop {
            let deliveries = bus.read(CONSUMER, OUTPUT, PAGE)?;
            for delivery in &deliveries {
                self.take(&delivery.message, runners);
            }
            match deliveries.last() {
                Some(last) => bus.ack(CONSUMER, last)?,
                None => return Ok(()),
            }
            if deliveries.len() < PAGE {
                return Ok(());
            }
        }
    }

    /// Keeps `message` when it is a posting newer than its runner's last. In a factory of several
    /// runners, a runner without a runner file posts nothing the intake watches.
    fn take(&mut self, message: &Message, runners: &BTreeSet<String>) {
        if message.kind != WATCH_POSTED {
            return;
        }
        let Some(runner) = message.data["runner"].as_str() else {
            return;
        };
        if runners.len() > 1 && !runners.contains(runner) {
            return;
        }
        if self
            .newest
            .get(runner)
            .is_some_and(|(time, _)| *time > message.time)
        {
            return;
        }
        let watches = message.data["watches"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|watch| {
                Some(Watch {
                    resource: watch["resource"].as_str()?.to_owned(),
                    since: watch["since"].as_str()?.parse().ok()?,
                    rank: u8::try_from(watch["rank"].as_u64()?).ok()?,
                })
            })
            .collect();
        self.newest
            .insert(runner.to_owned(), (message.time, watches));
    }

    /// The watches of each runner's posting younger than `KEEP`, but `except`'s, or of the newest
    /// posting alone in a factory of fewer than two runners.
    pub(super) fn watches(
        &self,
        runners: &BTreeSet<String>,
        except: Option<&str>,
        now: Timestamp,
    ) -> Vec<Watch> {
        let live = self
            .newest
            .iter()
            .filter(|(runner, (time, _))| {
                Some(runner.as_str()) != except && now.duration_since(*time) <= KEEP
            })
            .map(|(_, posting)| posting);
        match runners.len() {
            0 | 1 => live
                .max_by_key(|(time, _)| *time)
                .map(|(_, watches)| watches.clone())
                .unwrap_or_default(),
            _ => live
                .flat_map(|(_, watches)| watches.iter().cloned())
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn at(text: &str) -> Timestamp {
        text.parse().unwrap()
    }

    fn posting(runner: &str, time: &str, resource: &str) -> Message {
        let watch = json!({ "resource": resource, "since": time, "rank": 0 });
        let data = json!({ "runner": runner, "watches": [watch] });
        Message::new(WATCH_POSTED, data).time(at(time))
    }

    fn resources(watches: Vec<Watch>) -> Vec<String> {
        let mut resources: Vec<String> = watches.into_iter().map(|watch| watch.resource).collect();
        resources.sort();
        resources
    }

    #[test]
    fn the_newest_posting_of_each_runner_wins_whatever_order_it_arrives_in() {
        let runners = BTreeSet::from(["a".to_owned(), "b".to_owned()]);
        let mut postings = Postings::default();
        postings.take(
            &posting("a", "2026-11-26T10:00:00Z", "github:o/n#2"),
            &runners,
        );
        postings.take(
            &posting("a", "2026-11-26T09:00:00Z", "github:o/n#1"),
            &runners,
        );
        postings.take(
            &posting("b", "2026-11-26T09:00:00Z", "github:o/n#3"),
            &runners,
        );

        let now = at("2026-11-26T11:00:00Z");
        assert_eq!(
            resources(postings.watches(&runners, None, now)),
            ["github:o/n#2", "github:o/n#3"]
        );
    }

    #[test]
    fn a_runner_without_a_runner_file_and_the_state_files_own_runner_are_left_out() {
        let runners = BTreeSet::from(["a".to_owned(), "b".to_owned()]);
        let mut postings = Postings::default();
        postings.take(
            &posting("a", "2026-11-26T10:00:00Z", "github:o/n#1"),
            &runners,
        );
        postings.take(
            &posting("b", "2026-11-26T10:00:00Z", "github:o/n#2"),
            &runners,
        );
        postings.take(
            &posting("c", "2026-11-26T10:00:00Z", "github:o/n#3"),
            &runners,
        );

        let now = at("2026-11-26T11:00:00Z");
        let watches = postings.watches(&runners, Some("a"), now);
        assert_eq!(resources(watches), ["github:o/n#2"]);
    }

    #[test]
    fn a_posting_expires_seven_days_after_its_time() {
        let runners = BTreeSet::from(["a".to_owned(), "b".to_owned()]);
        let mut postings = Postings::default();
        postings.take(
            &posting("a", "2026-11-19T10:00:00Z", "github:o/n#1"),
            &runners,
        );
        postings.take(
            &posting("b", "2026-11-20T10:00:00Z", "github:o/n#2"),
            &runners,
        );

        let now = at("2026-11-26T11:00:00Z");
        assert_eq!(
            resources(postings.watches(&runners, None, now)),
            ["github:o/n#2"]
        );
    }

    #[test]
    fn a_factory_of_one_runner_watches_its_newest_posting_whatever_its_host_name() {
        let runners = BTreeSet::new();
        let mut postings = Postings::default();
        postings.take(
            &posting("old-host", "2026-11-26T09:00:00Z", "github:o/n#1"),
            &runners,
        );
        postings.take(
            &posting("new-host", "2026-11-26T10:00:00Z", "github:o/n#2"),
            &runners,
        );

        let now = at("2026-11-26T11:00:00Z");
        assert_eq!(
            resources(postings.watches(&runners, None, now)),
            ["github:o/n#2"]
        );
    }
}
