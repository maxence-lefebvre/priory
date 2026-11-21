//! The event log in a Postgres database that several machines share.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use herdr_bus::{Message, Output, Seq, Stored, Timestamp};
use postgres::{Client, Config, Row};
use tokio_postgres_rustls::MakeRustlsConnect;

use super::{Delivery, Log, Result, Unreachable};

/// The Postgres client, for the tests that set up the database a log lives in.
pub use postgres as client;

/// Creates the tables once per database, whichever process connects first.
const SCHEMA: &str = "
    SELECT pg_advisory_xact_lock(hashtext('priory_log_schema'));
    CREATE TABLE IF NOT EXISTS priory_log_messages (
        seq BIGSERIAL PRIMARY KEY,
        output TEXT NOT NULL,
        id TEXT NOT NULL,
        published TIMESTAMPTZ NOT NULL,
        message TEXT NOT NULL,
        UNIQUE (output, id)
    );
    CREATE TABLE IF NOT EXISTS priory_log_cursors (
        consumer TEXT NOT NULL,
        output TEXT NOT NULL,
        seq BIGINT NOT NULL,
        PRIMARY KEY (consumer, output)
    );
";

/// A connection to the database that holds the log, made again when the server drops it.
pub struct Postgres {
    config: Config,
    timeout: Duration,
    client: Mutex<Option<Client>>,
    /// Set while a call runs, and left set by one that outlived its timeout until it ends.
    busy: Arc<AtomicBool>,
}

/// What a call fails with on its own thread.
type Failure = Box<dyn std::error::Error + Send + Sync>;

impl Postgres {
    /// Connects to `url`, a libpq connection string or URL, over TLS when the server offers it,
    /// and creates the log's tables when they are missing. Each call waits on the database at most
    /// `timeout`. A database it cannot reach yet is no error: a later call connects.
    pub fn connect(url: &str, timeout: Duration) -> Result<Self> {
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
        let mut slot = self
            .client
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.busy.swap(true, Ordering::SeqCst) {
            return Err(Box::new(Unreachable(
                "an earlier call still waits on it".into(),
            )));
        }
        let (client, config, busy) = (slot.take(), self.config.clone(), Arc::clone(&self.busy));
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let outcome = call(client, &config, &operation);
            busy.store(false, Ordering::SeqCst);
            let _ = sender.send(outcome);
        });
        let Ok((client, result)) = receiver.recv_timeout(self.timeout) else {
            return Err(Box::new(Unreachable(format!(
                "no answer within {}s",
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
    operation: &impl Fn(&mut Client) -> Result<T, Failure>,
) -> (Option<Client>, Result<T, Failure>) {
    let open = |client: Option<Client>| match client.filter(|client| !client.is_closed()) {
        Some(client) => Ok(client),
        None => connected(config),
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

fn connected(config: &Config) -> Result<Client, Failure> {
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
        rows.iter().map(stored).collect()
    }

    fn read(&self, consumer: &str, output: &str, limit: usize) -> Result<Vec<Delivery>> {
        let (consumer, output) = (consumer.to_owned(), output.to_owned());
        let limit = i64::try_from(limit)?;
        let rows = self.run(move |client| {
            Ok(client.query(
                "SELECT * FROM priory_log_messages WHERE output = $1 AND seq > COALESCE(
                    (SELECT seq FROM priory_log_cursors WHERE consumer = $2 AND output = $1), 0)
                 ORDER BY seq LIMIT $3",
                &[&output, &consumer, &limit],
            )?)
        })?;
        rows.iter()
            .map(|row| {
                let stored = stored(row)?;
                Ok(Delivery {
                    output: stored.output,
                    seq: stored.seq,
                    message: stored.message,
                    logged: Some(stored.published),
                })
            })
            .collect()
    }

    fn ack(&self, consumer: &str, delivery: &Delivery) -> Result<()> {
        let (consumer, output, seq) =
            (consumer.to_owned(), delivery.output.clone(), delivery.seq.0);
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
