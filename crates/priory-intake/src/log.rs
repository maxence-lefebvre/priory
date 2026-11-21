//! The event log: what the intake publishes on and the daemon reads, whatever stores it.

use herdr_bus::{Bus, Message, Output, Seq, Start, Stored};

pub type Error = Box<dyn std::error::Error>;
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// A message read as a consumer, to acknowledge once handled.
#[derive(Clone, Debug)]
pub struct Delivery {
    pub output: String,
    pub seq: Seq,
    pub message: Message,
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
}

/// The machine's log: herdr-bus's SQLite file.
pub fn open() -> Result<Box<dyn Log>> {
    Ok(Box::new(Bus::open_default()?))
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
            })
            .collect())
    }

    fn ack(&self, consumer: &str, delivery: &Delivery) -> Result<()> {
        // shortcut: looks the connection up on each ack, a second write; cache it if acks get hot.
        let connection = self.connect(consumer, &delivery.output, Start::Beginning)?;
        Ok(Bus::ack(self, connection, delivery.seq)?)
    }
}
