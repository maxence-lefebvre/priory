//! Which runner owns each task: claims, holds and leases read from the shared log.

use jiff::{SignedDuration, Timestamp};

/// How long a runner's lease lasts past its last alive message.
pub(crate) const LEASE: SignedDuration = SignedDuration::from_mins(30);

/// What another runner may do with a task, as its owner last published it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Hold {
    /// Queued or ready, and no run of it ever started: it moves once its runner's lease ends.
    Waiting,
    /// Started or asleep: it stays with its owner until a person gives it away.
    Kept,
    /// Done, failed or dropped: a new request for its key claims it again.
    Ended,
}

/// The hold of a task in `state`, which `started` a run or not.
pub(crate) fn hold(state: &str, started: bool) -> Hold {
    match state {
        "done" | "failed" | "dropped" => Hold::Ended,
        "queued" | "ready" if !started => Hold::Waiting,
        _ => Hold::Kept,
    }
}

/// A key's owner, as the log says at a claim's place in it.
pub(crate) struct Owner<'a> {
    pub(crate) runner: &'a str,
    pub(crate) hold: Hold,
    /// The owner's last alive message, if any.
    pub(crate) alive: Option<Timestamp>,
}

/// A claim of a key, read from the log.
pub(crate) struct Claim<'a> {
    /// The owner it takes the key from, for a takeover or a gift.
    pub(crate) from: Option<&'a str>,
    /// A person gave the key, so the hold and the lease do not matter.
    pub(crate) given: bool,
    pub(crate) time: Timestamp,
}

/// Whether `claim` takes its key from `owner`.
pub(crate) fn wins(claim: &Claim, owner: Option<&Owner>) -> bool {
    let Some(from) = claim.from else {
        return owner.is_none_or(|owner| owner.hold == Hold::Ended);
    };
    let Some(owner) = owner.filter(|owner| owner.runner == from) else {
        return false;
    };
    match owner.hold {
        Hold::Ended => false,
        _ if claim.given => true,
        Hold::Waiting => lapsed(owner.alive, claim.time),
        Hold::Kept => false,
    }
}

/// Whether a lease last renewed at `alive` is over at `at`.
pub(crate) fn lapsed(alive: Option<Timestamp>, at: Timestamp) -> bool {
    alive.is_none_or(|alive| at.duration_since(alive) >= LEASE)
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: &str = "2026-11-21T10:00:00Z";

    fn at(minutes: i64) -> Timestamp {
        T0.parse::<Timestamp>().unwrap() + SignedDuration::from_mins(minutes)
    }

    fn owner(runner: &str, hold: Hold, alive: i64) -> Owner<'_> {
        Owner {
            runner,
            hold,
            alive: Some(at(alive)),
        }
    }

    fn claim(from: Option<&str>, given: bool, minutes: i64) -> Claim<'_> {
        Claim {
            from,
            given,
            time: at(minutes),
        }
    }

    #[test]
    fn a_first_claim_takes_an_unowned_key_or_one_whose_task_ended() {
        assert!(wins(&claim(None, false, 0), None));
        assert!(wins(
            &claim(None, false, 0),
            Some(&owner("a", Hold::Ended, 0))
        ));
    }

    #[test]
    fn a_second_claim_of_an_owned_key_loses_whatever_its_hold() {
        for hold in [Hold::Waiting, Hold::Kept] {
            assert!(!wins(&claim(None, false, 90), Some(&owner("a", hold, 0))));
        }
    }

    #[test]
    fn a_takeover_wins_a_waiting_key_only_once_its_owners_lease_is_over() {
        let waiting = owner("a", Hold::Waiting, 0);
        assert!(!wins(&claim(Some("a"), false, 29), Some(&waiting)));
        assert!(wins(&claim(Some("a"), false, 30), Some(&waiting)));
    }

    #[test]
    fn a_takeover_never_wins_a_kept_or_ended_key() {
        for hold in [Hold::Kept, Hold::Ended] {
            assert!(!wins(
                &claim(Some("a"), false, 600),
                Some(&owner("a", hold, 0))
            ));
        }
    }

    #[test]
    fn a_takeover_or_gift_from_a_runner_that_no_longer_owns_the_key_loses() {
        let other = owner("b", Hold::Waiting, 0);
        assert!(!wins(&claim(Some("a"), false, 600), Some(&other)));
        assert!(!wins(&claim(Some("a"), true, 600), Some(&other)));
        assert!(!wins(&claim(Some("a"), true, 0), None));
    }

    #[test]
    fn a_gift_takes_a_kept_key_from_a_live_runner() {
        assert!(wins(
            &claim(Some("a"), true, 1),
            Some(&owner("a", Hold::Kept, 0))
        ));
    }

    #[test]
    fn a_runner_never_seen_alive_holds_no_lease() {
        assert!(lapsed(None, at(0)));
        assert!(!lapsed(Some(at(0)), at(29)));
        assert!(lapsed(Some(at(0)), at(30)));
    }

    #[test]
    fn only_a_task_that_never_started_a_run_waits() {
        assert_eq!(hold("queued", false), Hold::Waiting);
        assert_eq!(hold("ready", false), Hold::Waiting);
        assert_eq!(hold("ready", true), Hold::Kept);
        for state in ["running", "blocked", "held", "asleep"] {
            assert_eq!(hold(state, false), Hold::Kept, "{state}");
        }
        for state in ["done", "failed", "dropped"] {
            assert_eq!(hold(state, true), Hold::Ended, "{state}");
        }
    }
}
