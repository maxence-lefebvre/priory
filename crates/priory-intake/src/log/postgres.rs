//! The event log in a Postgres database that several machines share.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use herdr_bus::{Message, Output, Seq, Stored, Timestamp};
use postgres::{Client, Config, Row};
use tokio_postgres_rustls::MakeRustlsConnect;

use super::sql::SCHEMA;
use super::{Delivery, Log, NO_ANSWER, Result, Unreachable};

/// The Postgres client, for the tests that set up the database a log lives in.
pub use postgres as client;

/// A connection to the database that holds the log, made again when the server drops it.
pub struct Postgres {
    config: Config,
    timeout: Duration,
    /// Connected as a role that may not create tables, so it skips creating them.
    restricted: bool,
    client: Mutex<Option<Client>>,
    /// Set while a call runs, and left set by one that outlived its timeout until it ends.
    busy: Arc<AtomicBool>,
}

/// What a call fails with on its own thread.
type Failure = Box<dyn std::error::Error + Send + Sync>;

impl Postgres {
    /// Connects to `url`, a libpq connection string or URL, over TLS when the server offers it,
    /// and creates the log's tables when they are missing, unless `restricted`. Each call waits on
    /// the database at most `timeout`. A database it cannot reach yet is no error: a later call
    /// connects.
    pub fn connect(url: &str, timeout: Duration, restricted: bool) -> Result<Self> {
        let mut config: Config = url.parse()?;
        let options = format!(
            "{} -c statement_timeout={}",
            config.get_options().unwrap_or_default(),
            timeout.as_millis()
        );
        config
            .options(options.trim_start())
            .connect_timeout(timeout)
            .tcp_user_timeout(timeout)
            .keepalives(true)
            .keepalives_idle(KEEPALIVE)
            .keepalives_interval(KEEPALIVE)
            .keepalives_retries(3);
        let log = Postgres {
            config,
            timeout,
            restricted,
            client: Mutex::new(None),
            busy: Arc::new(AtomicBool::new(false)),
        };
        match log.run(|_| Ok(())) {
            Err(error) if !super::unreachable(&*error) => Err(error),
            _ => Ok(log),
        }
    }

    /// Runs `operation` on a thread of its own, and gives up on it past the timeout: a link that
    /// hangs, such as a stopped ssh tunnel, keeps the socket open and never fails it. Until a call
    /// given up on ends, the next ones give up at once rather than pile up threads. A call runs
    /// again on a new connection when the server dropped this one: every operation of the log is
    /// idempotent, so running it twice changes nothing.
    fn run<T: Send + 'static>(
        &self,
        operation: impl Fn(&mut Client) -> Result<T, Failure> + Send + 'static,
    ) -> Result<T> {
        let waiting = Instant::now();
        let mut slot = self
            .client
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        priory_runtime::ticks::time("priory.log.connection.wait", waiting.elapsed());
        if self.busy.swap(true, Ordering::SeqCst) {
            return Err(Box::new(Unreachable(format!(
                "a call got {NO_ANSWER} {}s and still waits",
                self.timeout.as_secs_f64()
            ))));
        }
        let (client, config, busy) = (slot.take(), self.config.clone(), Arc::clone(&self.busy));
        let restricted = self.restricted;
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let outcome = call(client, &config, restricted, &operation);
            busy.store(false, Ordering::SeqCst);
            let _ = sender.send(outcome);
        });
        let Ok((client, result)) = receiver.recv_timeout(self.timeout) else {
            return Err(Box::new(Unreachable(format!(
                "{NO_ANSWER} {}s",
                self.timeout.as_secs_f64()
            ))));
        };
        *slot = client;
        result.map_err(|error| error as super::Error)
    }
}

/// How long a quiet connection waits before TCP checks the server is there, and between checks.
const KEEPALIVE: Duration = Duration::from_secs(10);

/// Runs `operation` on `client`, or on a new connection when there is none or the server dropped
/// it, and returns the connection to keep.
fn call<T>(
    client: Option<Client>,
    config: &Config,
    restricted: bool,
    operation: &impl Fn(&mut Client) -> Result<T, Failure>,
) -> (Option<Client>, Result<T, Failure>) {
    let open = |client: Option<Client>| match client.filter(|client| !client.is_closed()) {
        Some(client) => Ok(client),
        None => connected(config, restricted),
    };
    let mut client = match open(client) {
        Ok(client) => client,
        Err(error) => return (None, Err(lost(error))),
    };
    let result = match operation(&mut client) {
        Err(_) if client.is_closed() => match open(None) {
            Ok(fresh) => {
                client = fresh;
                operation(&mut client)
            }
            Err(error) => return (None, Err(lost(error))),
        },
        done => done,
    };
    (
        Some(client).filter(|client| !client.is_closed()),
        result.map_err(lost),
    )
}

/// `error` as `Unreachable` when it says the link is down or the server cannot serve now, as
/// opposed to a refused password or a bad statement, which retrying does not fix.
fn lost(error: Failure) -> Failure {
    let Some(postgres) = error.downcast_ref::<postgres::Error>() else {
        return error;
    };
    let gone = match postgres.code().map(postgres::error::SqlState::code) {
        Some(code) => {
            ["08", "53", "57P"]
                .iter()
                .any(|class| code.starts_with(class))
                || code == "57014"
        }
        None => {
            postgres.is_closed()
                || std::error::Error::source(postgres)
                    .is_some_and(|source| source.is::<std::io::Error>())
                || postgres.to_string() == "timeout waiting for server"
        }
    };
    if !gone {
        return error;
    }
    let detail = std::error::Error::source(postgres).map_or_else(
        || postgres.to_string(),
        |source| format!("{postgres}: {source}"),
    );
    Box::new(Unreachable(detail))
}

fn connected(config: &Config, restricted: bool) -> Result<Client, Failure> {
    // No native roots, such as in a bare container: plain connections still work.
    let tls = MakeRustlsConnect::with_native_certs().map_or_else(
        |_| {
            MakeRustlsConnect::new(
                rustls::ClientConfig::builder()
                    .with_root_certificates(rustls::RootCertStore::empty())
                    .with_no_client_auth(),
            )
        },
        |(tls, _)| tls,
    );
    let mut client = config.connect(tls)?;
    if restricted {
        return Ok(client);
    }
    let mut tx = client.transaction()?;
    tx.batch_execute(SCHEMA)?;
    tx.commit()?;
    Ok(client)
}

fn stored(row: &Row) -> Result<Stored> {
    Ok(Stored {
        seq: Seq(row.get("seq")),
        output: row.get("output"),
        published: row.get::<_, Timestamp>("published"),
        message: serde_json::from_str(row.get("message"))?,
        request: None,
    })
}

impl Log for Postgres {
    /// The tables hold every output: declaring one adds nothing.
    fn declare(&self, _output: &Output) -> Result<()> {
        Ok(())
    }

    fn publish(&self, output: &str, message: &Message) -> Result<Seq> {
        let (output, id) = (output.to_owned(), message.id.to_string());
        let message = serde_json::to_string(message)?;
        self.run(move |client| {
            let mut tx = client.transaction()?;
            // One publisher at a time, so seqs commit in order and no reader skips a later commit.
            tx.execute(
                "SELECT pg_advisory_xact_lock(hashtext('priory_log_publish'))",
                &[],
            )?;
            let inserted = tx.query_opt(
                "INSERT INTO priory_log_messages (output, id, published, message)
                 VALUES ($1, $2, now(), $3) ON CONFLICT (output, id) DO NOTHING RETURNING seq",
                &[&output, &id, &message],
            )?;
            let row = match inserted {
                Some(row) => row,
                None => tx.query_one(
                    "SELECT seq FROM priory_log_messages WHERE output = $1 AND id = $2",
                    &[&output, &id],
                )?,
            };
            tx.commit()?;
            Ok(Seq(row.get("seq")))
        })
    }

    fn message(&self, seq: Seq) -> Result<Stored> {
        let row = self.run(move |client| {
            Ok(client.query_opt(
                "SELECT * FROM priory_log_messages WHERE seq = $1",
                &[&seq.0],
            )?)
        })?;
        stored(&row.ok_or_else(|| format!("the log holds no message {}", seq.0))?)
    }

    fn messages(&self, output: &str, before: Option<Seq>, limit: usize) -> Result<Vec<Stored>> {
        let (output, limit) = (output.to_owned(), i64::try_from(limit)?);
        let rows = self.run(move |client| {
            Ok(client.query(
                "SELECT * FROM priory_log_messages
                 WHERE output = $1 AND ($2::BIGINT IS NULL OR seq < $2)
                 ORDER BY seq DESC LIMIT $3",
                &[&output, &before.map(|seq| seq.0), &limit],
            )?)
        })?;
        // `read` reports a row that does not parse; a listing only leaves it out.
        Ok(rows.iter().filter_map(|row| stored(row).ok()).collect())
    }

    /// Skips a row that does not parse as a message, such as one a restricted role wrote with a
    /// duplicate `type`: it reports the row once and moves `consumer`'s cursor past it, so one bad
    /// row never stops a reader.
    fn read(&self, consumer: &str, output: &str, limit: usize) -> Result<Vec<Delivery>> {
        loop {
            let (owned_consumer, owned_output) = (consumer.to_owned(), output.to_owned());
            let query_limit = i64::try_from(limit)?;
            let rows = self.run(move |client| {
                Ok(client.query(
                    "SELECT * FROM priory_log_messages WHERE output = $1 AND seq > COALESCE(
                        (SELECT seq FROM priory_log_cursors WHERE consumer = $2 AND output = $1), 0)
                     ORDER BY seq LIMIT $3",
                    &[&owned_output, &owned_consumer, &query_limit],
                )?)
            })?;
            let mut deliveries = Vec::new();
            let mut skipped = None;
            for row in &rows {
                match stored(row) {
                    Ok(stored) => deliveries.push(Delivery {
                        output: stored.output,
                        seq: stored.seq,
                        message: stored.message,
                        logged: Some(stored.published),
                    }),
                    // The next read meets it first, and skips it then.
                    Err(_) if !deliveries.is_empty() => break,
                    Err(error) => {
                        let seq: i64 = row.get("seq");
                        eprintln!("the log skips message {seq} of {output}: {error}");
                        priory_runtime::ticks::count(super::SKIPPED, &[]);
                        skipped = Some(seq);
                    }
                }
            }
            let Some(seq) = skipped else {
                return Ok(deliveries);
            };
            self.advance(consumer, output, seq)?;
            if !deliveries.is_empty() || rows.len() < limit {
                return Ok(deliveries);
            }
        }
    }

    fn ack(&self, consumer: &str, delivery: &Delivery) -> Result<()> {
        self.advance(consumer, &delivery.output, delivery.seq.0)
    }

    fn backlog(&self, output: &str) -> Result<Vec<(String, u64)>> {
        let output = output.to_owned();
        let rows = self.run(move |client| {
            Ok(client.query(
                "SELECT c.consumer, (SELECT count(*) FROM priory_log_messages m
                   WHERE m.output = c.output AND m.seq > c.seq) AS past
                 FROM priory_log_cursors c WHERE c.output = $1 ORDER BY c.consumer",
                &[&output],
            )?)
        })?;
        Ok(rows
            .iter()
            .map(|row| {
                let past: i64 = row.get("past");
                (row.get("consumer"), u64::try_from(past).unwrap_or_default())
            })
            .collect())
    }
}

/// One consumer's cursor on one output, as `copy_in` writes it.
pub struct Cursor {
    pub consumer: String,
    pub output: String,
    pub seq: i64,
}

impl Postgres {
    /// Writes `messages`, oldest first, with their seqs, and `cursors` into this log in one
    /// transaction, and refuses a log that holds any message or cursor already, so a copy never
    /// mixes two histories. Later publishes get seqs past the last copied. `dry_run` rolls back.
    pub fn copy_in(&self, messages: &[Stored], cursors: &[Cursor], dry_run: bool) -> Result<()> {
        let mut rows = (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
        for stored in messages {
            rows.0.push(stored.seq.0);
            rows.1.push(stored.output.clone());
            rows.2.push(stored.message.id.to_string());
            rows.3.push(stored.published);
            rows.4.push(serde_json::to_string(&stored.message)?);
        }
        let cursors: (Vec<_>, Vec<_>, Vec<_>) = (
            cursors
                .iter()
                .map(|cursor| cursor.consumer.clone())
                .collect(),
            cursors.iter().map(|cursor| cursor.output.clone()).collect(),
            cursors.iter().map(|cursor| cursor.seq).collect(),
        );
        self.run(move |client| {
            let mut tx = client.transaction()?;
            tx.execute(
                "SELECT pg_advisory_xact_lock(hashtext('priory_log_publish'))",
                &[],
            )?;
            let held: bool = tx
                .query_one(
                    "SELECT EXISTS (SELECT 1 FROM priory_log_messages)
                         OR EXISTS (SELECT 1 FROM priory_log_cursors)",
                    &[],
                )?
                .get(0);
            if held {
                return Err("the Postgres log already holds messages or cursors: \
                            priory log copy only fills an empty log"
                    .into());
            }
            tx.execute(
                "INSERT INTO priory_log_messages (seq, output, id, published, message)
                 SELECT * FROM unnest($1::BIGINT[], $2::TEXT[], $3::TEXT[], $4::TIMESTAMPTZ[], $5::TEXT[])",
                &[&rows.0, &rows.1, &rows.2, &rows.3, &rows.4],
            )?;
            tx.execute(
                "SELECT setval(pg_get_serial_sequence('priory_log_messages', 'seq'),
                     GREATEST(max(seq), 1), max(seq) IS NOT NULL) FROM priory_log_messages",
                &[],
            )?;
            tx.execute(
                "INSERT INTO priory_log_cursors (consumer, output, seq)
                 SELECT * FROM unnest($1::TEXT[], $2::TEXT[], $3::BIGINT[])",
                &[&cursors.0, &cursors.1, &cursors.2],
            )?;
            if dry_run {
                tx.rollback()?;
            } else {
                tx.commit()?;
            }
            Ok(())
        })
    }

    /// Moves `consumer`'s cursor on `output` to `seq`, unless it is past it already.
    fn advance(&self, consumer: &str, output: &str, seq: i64) -> Result<()> {
        let (consumer, output) = (consumer.to_owned(), output.to_owned());
        self.run(move |client| {
            client.execute(
                "INSERT INTO priory_log_cursors (consumer, output, seq) VALUES ($1, $2, $3)
                 ON CONFLICT (consumer, output)
                 DO UPDATE SET seq = GREATEST(priory_log_cursors.seq, EXCLUDED.seq)",
                &[&consumer, &output, &seq],
            )?;
            Ok(())
        })
    }
}
