use super::{Observed, REVIEWED, THREAD_COMMENTED, gh, id, lately, now};
use crate::log::Log;
use herdr_bus::{Message, Timestamp, Uuid};
use serde_json::Value;
use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};

/// The most reviews one poll looks up.
const LOOKUPS: usize = 20;
/// How many failed lookups make a review count as the person's.
const TRIES: u32 = 10;
/// GitHub can report a review before it lists the review's comments.
const YOUNG_SECONDS: i64 = 60;

/// Whether an event is a review that only stands for its thread comments: GitHub wraps each reply in
/// a thread in a COMMENTED review with an empty body.
pub(super) fn wraps(kind: &str, data: &Value) -> bool {
    kind == REVIEWED
        && data["state"] == "commented"
        && data["body"]
            .as_str()
            .is_some_and(|body| body.trim().is_empty())
}

/// What the polls know of the reviews that stand for their threads.
pub(super) struct Threads {
    /// The reviews on the output, by message id.
    stored: HashMap<Uuid, Message>,
    /// The thread comments that were on the output when this poll began.
    said: HashSet<Uuid>,
    /// The comments of each review this poll judges: none while they are unread.
    threads: HashMap<Uuid, Option<Vec<Value>>>,
    /// How many lookups of each review failed.
    tries: HashMap<Uuid, u32>,
    /// Whether this poll has more reviews to look up than `LOOKUPS`.
    backlog: bool,
}

impl Threads {
    pub(super) fn new(stored: HashMap<Uuid, Message>) -> Self {
        Threads {
            stored,
            said: HashSet::new(),
            threads: HashMap::new(),
            tries: HashMap::new(),
            backlog: false,
        }
    }

    /// Whether the review waits for a later poll to read its comments.
    pub(super) fn unread(&self, review: &Uuid) -> bool {
        matches!(self.threads.get(review), Some(None))
    }

    /// Whether any review waits for a later poll to read its comments.
    pub(super) fn any_unread(&self) -> bool {
        self.threads.values().any(Option::is_none)
    }

    /// What intake health says while one poll cannot look up every review.
    pub(super) fn backlog(&self) -> Option<String> {
        // No count: the alarm posts again whenever its text changes.
        self.backlog
            .then(|| format!("over {LOOKUPS} reviews wait for a lookup of their comments"))
    }

    pub(super) fn stored(&self, review: &Uuid) -> Option<&Message> {
        self.stored.get(review)
    }

    pub(super) fn thread(&self, review: &Uuid) -> &[Value] {
        self.threads
            .get(review)
            .and_then(Option::as_deref)
            .unwrap_or_default()
    }

    pub(super) fn said(&self, comment: &Uuid) -> bool {
        self.said.contains(comment)
    }

    /// Reads the comments of at most `LOOKUPS` reviews by `viewer` in `events` that are neither in
    /// `published` nor on the output, newest first within equal tries, and returns the errors. A
    /// rate limit or a timeout ends it.
    pub(super) fn read<'a>(
        &mut self,
        bus: &dyn Log,
        events: impl Iterator<Item = &'a Observed>,
        published: &HashSet<Uuid>,
        viewer: Option<&str>,
    ) -> Result<Vec<String>, String> {
        self.threads.clear();
        self.said.clear();
        self.backlog = false;
        let mut unjudged: Vec<(Uuid, &Observed)> = events
            .filter(|event| event.wraps(viewer))
            .map(|event| (id(&event.name), event))
            .filter(|(id, _)| !published.contains(id) && !self.stored.contains_key(id))
            .collect();
        if unjudged.is_empty() {
            return Ok(Vec::new());
        }
        let wanted = |message: &Message| {
            message.kind == THREAD_COMMENTED || wraps(&message.kind, &message.data)
        };
        for message in lately(bus, wanted)? {
            if message.kind == THREAD_COMMENTED {
                self.said.insert(message.id);
            } else {
                self.stored.insert(message.id, message);
            }
        }
        unjudged.retain(|(id, _)| !self.stored.contains_key(id));
        unjudged.sort_by_key(|(id, event)| {
            let data = &event.data;
            let newest = Reverse((data["time"].as_str(), data["github_id"].as_u64()));
            (self.tries.get(id).copied(), newest)
        });
        self.backlog = unjudged.len() > LOOKUPS;
        self.threads
            .extend(unjudged.iter().map(|(id, _)| (*id, None)));
        let mut failed = Vec::new();
        for (id, event) in unjudged.into_iter().take(LOOKUPS) {
            let error = match event.read_thread() {
                Ok(thread) => {
                    self.threads.insert(id, thread);
                    continue;
                }
                Err(error) => error,
            };
            let limited = limited(&error);
            let stop = limited || error.contains("timed out after");
            if !limited && self.give_up(id) {
                eprintln!("{error}: the review counts as the person's");
                self.threads.insert(id, Some(Vec::new()));
            } else {
                failed.push(error);
            }
            if stop {
                break;
            }
        }
        Ok(failed)
    }

    /// Counts a failed lookup of `review`, and whether it was its last.
    fn give_up(&mut self, review: Uuid) -> bool {
        let tries = self.tries.entry(review).or_default();
        *tries += 1;
        *tries >= TRIES
    }
}

/// Whether `error` says GitHub refuses calls for a while.
fn limited(error: &str) -> bool {
    error.contains("HTTP 429") || error.to_lowercase().contains("rate limit")
}

impl Observed {
    /// Whether this is a review by `viewer` that only stands for its thread comments.
    pub(super) fn wraps(&self, viewer: Option<&str>) -> bool {
        wraps(self.kind, &self.data) && viewer.is_some_and(|viewer| self.data["actor"] == viewer)
    }

    /// The thread comments this review wraps, its first 100: none while it is too young for an empty
    /// list to be final.
    pub(super) fn read_thread(&self) -> Result<Option<Vec<Value>>, String> {
        let thread: Vec<Value> = gh::gh(
            &format!(
                "repos/{}/pulls/{}/reviews/{}/comments?per_page=100",
                self.repo, self.number, self.data["github_id"]
            ),
            &[],
        )?;
        let now = now()?.as_second();
        let time = self.data["time"].as_str().unwrap_or_default();
        let young = time
            .parse::<Timestamp>()
            .is_ok_and(|time| time.as_second() + YOUNG_SECONDS > now);
        Ok((!thread.is_empty() || !young).then_some(thread))
    }

    /// Whether `message` is an agent's review whose `thread` comments are all `said`, each by its
    /// message id.
    pub(super) fn repeats(
        &self,
        message: &Message,
        thread: &[Value],
        said: impl Fn(&Uuid) -> bool,
    ) -> bool {
        !thread.is_empty()
            && message.data["agent"] == true
            && thread.iter().all(|comment| {
                let name = format!("github:{}:pulls/comments:{}", self.repo, comment["id"]);
                said(&id(&name))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tenth_failed_lookup_of_a_review_is_its_last_and_each_review_counts_alone() {
        let mut threads = Threads::new(HashMap::new());
        let (one, other) = (id("one"), id("other"));
        for _ in 1..10 {
            assert!(!threads.give_up(one));
        }
        assert!(!threads.give_up(other));
        assert!(threads.give_up(one));
    }

    #[test]
    fn a_rate_limit_counts_no_try_and_a_404_does() {
        assert!(limited(
            "gh api x failed: gh: API rate limit exceeded for user ID 1. (HTTP 403)"
        ));
        assert!(limited(
            "gh: You have exceeded a secondary rate limit (HTTP 403)"
        ));
        assert!(limited("gh: Too Many Requests (HTTP 429)"));
        assert!(!limited("gh: Not Found (HTTP 404)"));
    }
}
