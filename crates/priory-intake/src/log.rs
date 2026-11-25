//! The event log: what the intake publishes on and the daemon reads, whatever stores it.

use herdr_bus::{Bus, Message, Output, Seq, Start, Stored, Timestamp};
use priory_factory::factory::{Backend, EventLog};

#[cfg(feature = "postgres")]
pub mod postgres;

pub type Error = Box<dyn std::error::Error>;
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// The log did not answer: its link is down or hangs. A later call may reach it.
#[derive(Debug)]
pub struct Unreachable(pub String);

impl std::fmt::Display for Unreachable {
    fn fmt(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(formatter, "the log is unreachable: {}", self.0)
    }
}

impl std::error::Error for Unreachable {}

/// Whether `error` says the log is unreachable, so that retrying later may work.
pub fn unreachable(error: &(dyn std::error::Error + 'static)) -> bool {
    error.is::<Unreachable>()
}

/// What an `Unreachable` says when a call got no answer within the log's timeout, or was refused
/// because an earlier one still waits.
pub const NO_ANSWER: &str = "no answer within";

/// Whether `error` says the log gave no answer in time, rather than failing.
pub fn timed_out(error: &(dyn std::error::Error + 'static)) -> bool {
    error
        .downcast_ref::<Unreachable>()
        .is_some_and(|Unreachable(detail)| detail.contains(NO_ANSWER))
}

/// A message read as a consumer, to acknowledge once handled.
#[derive(Clone, Debug)]
pub struct Delivery {
    pub output: String,
    pub seq: Seq,
    pub message: Message,
    /// When the log stored the message, if one clock stamps every message, as a Postgres server
    /// does. herdr-bus stamps each with its publisher's clock, so it gives none.
    pub logged: Option<Timestamp>,
}

/// An append-only log of outputs, each a sequence of messages, with a cursor per consumer.
pub trait Log {
    /// Makes `output` ready to publish on. Declaring it again changes nothing.
    fn declare(&self, output: &Output) -> Result<()>;
    /// Appends `message` to `output` and returns its seq, or the seq it already has when the
    /// output holds its id: the log keeps the first message of each id.
    fn publish(&self, output: &str, message: &Message) -> Result<Seq>;
    /// The message at `seq`.
    fn message(&self, seq: Seq) -> Result<Stored>;
    /// At most `limit` messages of `output` before `before`, or the newest, newest first.
    fn messages(&self, output: &str, before: Option<Seq>, limit: usize) -> Result<Vec<Stored>>;
    /// At most `limit` messages of `output` past `consumer`'s cursor, oldest first. A consumer
    /// starts at the beginning. It moves no cursor, so a message comes back until acknowledged.
    /// An output nobody declared yet holds nothing.
    fn read(&self, consumer: &str, output: &str, limit: usize) -> Result<Vec<Delivery>>;
    /// Moves `consumer`'s cursor on the delivery's output to it, unless it is past it already.
    fn ack(&self, consumer: &str, delivery: &Delivery) -> Result<()>;
    /// Each consumer with a cursor on `output`, by the name it reads as, and how many messages of
    /// `output` are past that cursor.
    fn backlog(&self, output: &str) -> Result<Vec<(String, u64)>>;
}

/// The log the factory's `[log]` table names: herdr-bus's SQLite file on this machine unless it
/// names Postgres.
pub fn open(log: &EventLog) -> Result<Box<dyn Log>> {
    priory_runtime::upgrade::barrier()?;
    match log.backend {
        Backend::HerdrBus => Ok(Box::new(Bus::open_default()?)),
        Backend::Postgres => open_postgres(log),
    }
}

#[cfg(feature = "postgres")]
fn open_postgres(log: &EventLog) -> Result<Box<dyn Log>> {
    let url_env = log.url_env();
    let url = std::env::var(url_env).map_err(|_| {
        format!("[log] backend = \"postgres\" reads its connection string from ${url_env}, which is not set")
    })?;
    Ok(Box::new(postgres::Postgres::connect(&url, log.timeout())?))
}

#[cfg(not(feature = "postgres"))]
fn open_postgres(_log: &EventLog) -> Result<Box<dyn Log>> {
    Err("[log] backend = \"postgres\" needs a priory built with `--features postgres`".into())
}

impl Log for Bus {
    fn declare(&self, output: &Output) -> Result<()> {
        Bus::declare(self, output)?;
        Ok(())
    }

    fn publish(&self, output: &str, message: &Message) -> Result<Seq> {
        Ok(Bus::publish(self, output, message)?)
    }

    fn message(&self, seq: Seq) -> Result<Stored> {
        Ok(Bus::message(self, seq)?)
    }

    fn messages(&self, output: &str, before: Option<Seq>, limit: usize) -> Result<Vec<Stored>> {
        Ok(Bus::messages(
            self,
            &herdr_bus::MessageQuery {
                output: Some(output.into()),
                before,
                limit,
                ..herdr_bus::MessageQuery::default()
            },
        )?)
    }

    fn read(&self, consumer: &str, output: &str, limit: usize) -> Result<Vec<Delivery>> {
        let connection = match self.connect(consumer, output, Start::Beginning) {
            Err(herdr_bus::Error::UnknownOutput(_)) => return Ok(Vec::new()),
            connected => connected?,
        };
        // A pull reads every connection of the consumer, such as a tap made with herdr-bus's CLI.
        let deliveries = self.pull(consumer, limit)?;
        let full = deliveries.len() == limit;
        let ours: Vec<Delivery> = deliveries
            .into_iter()
            .filter(|delivery| delivery.connection == connection)
            .map(|delivery| Delivery {
                output: delivery.output,
                seq: delivery.seq,
                message: delivery.message,
                logged: None,
            })
            .collect();
        if !full || !ours.is_empty() || limit == 0 {
            return Ok(ours);
        }
        // The other connections filled the pull: page the output back to this cursor instead.
        let connection = self.connection(connection)?;
        if !connection.enabled {
            return Ok(Vec::new());
        }
        let cursor = connection.cursor;
        let mut past = Vec::new();
        let mut before = None;
        loop {
            let page = Log::messages(self, output, before, 500)?;
            let reached = page.last().is_none_or(|oldest| oldest.seq <= cursor);
            before = page.last().map(|oldest| oldest.seq);
            past.extend(page.into_iter().filter(|stored| stored.seq > cursor));
            // Newest first: keep only the oldest `limit`.
            past.drain(..past.len().saturating_sub(limit));
            if reached {
                break;
            }
        }
        Ok(past
            .into_iter()
            .rev()
            .map(|stored| Delivery {
                output: stored.output,
                seq: stored.seq,
                message: stored.message,
                logged: None,
            })
            .collect())
    }

    fn ack(&self, consumer: &str, delivery: &Delivery) -> Result<()> {
        // shortcut: looks the connection up on each ack, a second write; cache it if acks get hot.
        let connection = self.connect(consumer, &delivery.output, Start::Beginning)?;
        Ok(Bus::ack(self, connection, delivery.seq)?)
    }

    fn backlog(&self, output: &str) -> Result<Vec<(String, u64)>> {
        // This machine's consumers go by their bare name, as they read; another's keeps its id.
        let here = format!("@{}", self.id());
        Ok(self
            .connections()?
            .into_iter()
            .filter(|connection| connection.output == output && connection.enabled)
            .map(|connection| {
                let name = connection.consumer.strip_suffix(&here).map(str::to_owned);
                (name.unwrap_or(connection.consumer), connection.lag)
            })
            .collect())
    }
}
