//! The event log in a Postgres database that several machines share.

use std::sync::Mutex;

use herdr_bus::{Message, Output, Seq, Stored, Timestamp};
use postgres::{Client, Row};
use tokio_postgres_rustls::MakeRustlsConnect;

use super::{Delivery, Log, Result};

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
    url: String,
    client: Mutex<Client>,
}

impl Postgres {
    /// Connects to `url`, a libpq connection string or URL, over TLS when the server offers it,
    /// and creates the log's tables when they are missing.
    pub fn connect(url: &str) -> Result<Self> {
        Ok(Postgres {
            url: url.to_owned(),
            client: Mutex::new(connected(url)?),
        })
    }

    /// Runs `operation`, again on a new connection when the server dropped this one. Every
    /// operation of the log is idempotent, so running it twice changes nothing.
    fn run<T>(&self, operation: impl Fn(&mut Client) -> Result<T>) -> Result<T> {
        let mut client = self
            .client
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if client.is_closed() {
            *client = connected(&self.url)?;
        }
        match operation(&mut client) {
            Err(_) if client.is_closed() => {
                *client = connected(&self.url)?;
                operation(&mut client)
            }
            done => done,
        }
    }
}

fn connected(url: &str) -> Result<Client> {
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
    let mut client = Client::connect(url, tls)?;
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
        let id = message.id.to_string();
        let message = serde_json::to_string(message)?;
        self.run(|client| {
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
        let row = self.run(|client| {
            Ok(client.query_opt(
                "SELECT * FROM priory_log_messages WHERE seq = $1",
                &[&seq.0],
            )?)
        })?;
        stored(&row.ok_or_else(|| format!("the log holds no message {}", seq.0))?)
    }

    fn messages(&self, output: &str, before: Option<Seq>, limit: usize) -> Result<Vec<Stored>> {
        let limit = i64::try_from(limit)?;
        let rows = self.run(|client| {
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
        let limit = i64::try_from(limit)?;
        let rows = self.run(|client| {
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
        self.run(|client| {
            client.execute(
                "INSERT INTO priory_log_cursors (consumer, output, seq) VALUES ($1, $2, $3)
                 ON CONFLICT (consumer, output)
                 DO UPDATE SET seq = GREATEST(priory_log_cursors.seq, EXCLUDED.seq)",
                &[&consumer, &delivery.output, &delivery.seq.0],
            )?;
            Ok(())
        })
    }
}
