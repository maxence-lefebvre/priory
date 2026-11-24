//! Submits the prompts the outbox pasted. Enter goes as its own key press, a moment after the
//! paste, so the agent never reads it as part of the paste. When no `UserPromptSubmit` follows,
//! Enter goes once more, and the text is never pasted again.

use crate::state;
use priory_runtime::runtime;
use rusqlite::Connection;
use std::time::{Duration, Instant};

/// How long after a paste Enter goes.
pub(crate) const GAP: Duration = Duration::from_millis(300);
/// How long a pressed Enter waits for its `UserPromptSubmit` before Enter goes once more.
pub(crate) const AGAIN: Duration = Duration::from_secs(5);
/// How long the second Enter waits before the prompt is `uncertain`.
pub(crate) const UNCONFIRMED: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, PartialEq)]
enum Stage {
    Pasted,
    Entered,
    Again,
}

enum Target {
    Run {
        outbox: i64,
        pane: String,
        compact: bool,
    },
    /// A follow-up the factory's `type` command sent to a person's own session, Enter included.
    Outside(i64),
}

struct Pasted {
    target: Target,
    stage: Stage,
    at: Instant,
}

impl Pasted {
    fn wait(&self) -> Duration {
        match self.stage {
            Stage::Pasted => GAP,
            Stage::Entered => AGAIN,
            Stage::Again => UNCONFIRMED,
        }
    }
}

/// The pasted prompts that wait for an Enter or for their `UserPromptSubmit`.
#[derive(Default)]
pub(crate) struct Entering {
    pasted: Vec<Pasted>,
    enters: Vec<i64>,
    unconfirmed: Vec<i64>,
}

impl Entering {
    /// Notes that the text of the attempted prompt `outbox` was pasted into `pane`.
    pub(crate) fn pasted(&mut self, outbox: i64, pane: String, compact: bool) {
        self.pasted.push(Pasted {
            target: Target::Run {
                outbox,
                pane,
                compact,
            },
            stage: Stage::Pasted,
            at: Instant::now(),
        });
    }

    /// Notes that the follow-up of compaction `id` was sent with its first Enter.
    pub(crate) fn sent(&mut self, id: i64) {
        self.pasted.push(Pasted {
            target: Target::Outside(id),
            stage: Stage::Entered,
            at: Instant::now(),
        });
    }

    /// Takes the compactions whose follow-up needs its second Enter.
    pub(crate) fn enters(&mut self) -> Vec<i64> {
        std::mem::take(&mut self.enters)
    }

    /// Takes the compactions whose follow-up got no `UserPromptSubmit`.
    pub(crate) fn unconfirmed(&mut self) -> Vec<i64> {
        std::mem::take(&mut self.unconfirmed)
    }

    /// How long until the next Enter or verdict is due, if any prompt waits.
    pub(crate) fn due(&self) -> Option<Duration> {
        self.pasted
            .iter()
            .map(|pasted| pasted.wait().saturating_sub(pasted.at.elapsed()))
            .min()
    }

    /// Presses Enter on each prompt pasted `GAP` ago, presses it once more on each that got no
    /// `UserPromptSubmit` within `AGAIN`, and marks `uncertain` each still without one
    /// `UNCONFIRMED` later. A `/compact` gets one Enter: its end is a `SessionStart`, which can take
    /// minutes.
    pub(crate) fn tick(&mut self, db: &Connection) -> rusqlite::Result<()> {
        let mut kept = Vec::with_capacity(self.pasted.len());
        for mut pasted in std::mem::take(&mut self.pasted) {
            if pasted.at.elapsed() < pasted.wait() {
                kept.push(pasted);
                continue;
            }
            let (waits, compact) = match &pasted.target {
                Target::Run {
                    outbox, compact, ..
                } => (state::unsubmitted(db, *outbox)?, *compact),
                Target::Outside(id) => (crate::compact::unsubmitted(db, *id)?, false),
            };
            match pasted.stage {
                Stage::Pasted if compact || waits => {
                    if self.press(&pasted) && !compact {
                        pasted.stage = Stage::Entered;
                        pasted.at = Instant::now();
                        kept.push(pasted);
                    }
                }
                Stage::Entered if waits => {
                    if self.press(&pasted) {
                        pasted.stage = Stage::Again;
                        pasted.at = Instant::now();
                        kept.push(pasted);
                    }
                }
                Stage::Again if waits => match pasted.target {
                    Target::Run { outbox, .. } => state::unconfirmed(db, outbox)?,
                    Target::Outside(id) => self.unconfirmed.push(id),
                },
                _ => {}
            }
        }
        self.pasted = kept;
        Ok(())
    }

    /// Presses Enter in the prompt's pane, or leaves it to the `type` command. A pane that is gone
    /// is the health check's to handle.
    fn press(&mut self, pasted: &Pasted) -> bool {
        match &pasted.target {
            Target::Run { outbox, pane, .. } => match runtime::press(pane, "Enter") {
                Ok(()) => true,
                Err(error) => {
                    eprintln!(
                        "priory daemon: pressing Enter in {pane} for prompt {outbox}: {error}"
                    );
                    false
                }
            },
            Target::Outside(id) => {
                self.enters.push(*id);
                true
            }
        }
    }
}
