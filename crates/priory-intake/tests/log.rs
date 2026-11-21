use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use herdr_bus::{Bus, Message, Output, Seq, Stored, Timestamp};
use priory_factory::factory::{Backend, EventLog};
use priory_intake::log::{self, Delivery, Log, Result};

const OUTPUT: &str = "test.events";

/// Opens handles on one log, as each process start does.
trait Store {
    fn open(&self) -> Box<dyn Log>;
}

/// A herdr-bus file in a folder of its own, removed when dropped.
struct Herdr(PathBuf);

impl Herdr {
    fn new(test: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("priory-log-{}-{test}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        Herdr(dir)
    }
}

impl Drop for Herdr {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

impl Store for Herdr {
    fn open(&self) -> Box<dyn Log> {
        Box::new(Bus::open(&self.0.join("bus.db")).unwrap())
    }
}

/// A log in memory, shared by every handle opened on it: proof the contract names no herdr-bus
/// behaviour.
#[derive(Default)]
struct Memory(Arc<Mutex<Logged>>);

#[derive(Default)]
struct Logged {
    messages: Vec<Stored>,
    cursors: BTreeMap<(String, String), Seq>,
}

impl Memory {
    fn new(_test: &str) -> Self {
        Memory::default()
    }
}

impl Store for Memory {
    fn open(&self) -> Box<dyn Log> {
        Box::new(Memory(Arc::clone(&self.0)))
    }
}

impl Log for Memory {
    fn declare(&self, _output: &Output) -> Result<()> {
        Ok(())
    }

    fn publish(&self, output: &str, message: &Message) -> Result<Seq> {
        let mut logged = self.0.lock().unwrap();
        let held = logged
            .messages
            .iter()
            .find(|stored| stored.output == output && stored.message.id == message.id);
        if let Some(stored) = held {
            return Ok(stored.seq);
        }
        let seq = Seq(logged.messages.len() as i64 + 1);
        logged.messages.push(Stored {
            seq,
            output: output.into(),
            published: Timestamp::now(),
            message: message.clone(),
            request: None,
        });
        Ok(seq)
    }

    fn message(&self, seq: Seq) -> Result<Stored> {
        let logged = self.0.lock().unwrap();
        let stored = logged.messages.iter().find(|stored| stored.seq == seq);
        stored
            .cloned()
            .ok_or_else(|| format!("no message {seq:?}").into())
    }

    fn messages(&self, output: &str, before: Option<Seq>, limit: usize) -> Result<Vec<Stored>> {
        let logged = self.0.lock().unwrap();
        Ok(logged
            .messages
            .iter()
            .rev()
            .filter(|stored| stored.output == output && before.is_none_or(|seq| stored.seq < seq))
            .take(limit)
            .cloned()
            .collect())
    }

    fn read(&self, consumer: &str, output: &str, limit: usize) -> Result<Vec<Delivery>> {
        let logged = self.0.lock().unwrap();
        let cursor = logged.cursors.get(&(consumer.into(), output.into()));
        let cursor = cursor.copied().unwrap_or(Seq(0));
        Ok(logged
            .messages
            .iter()
            .filter(|stored| stored.output == output && stored.seq > cursor)
            .take(limit)
            .map(|stored| Delivery {
                output: stored.output.clone(),
                seq: stored.seq,
                message: stored.message.clone(),
                logged: None,
            })
            .collect())
    }

    fn ack(&self, consumer: &str, delivery: &Delivery) -> Result<()> {
        let mut logged = self.0.lock().unwrap();
        let key = (consumer.to_owned(), delivery.output.clone());
        let cursor = logged.cursors.entry(key).or_insert(Seq(0));
        *cursor = (*cursor).max(delivery.seq);
        Ok(())
    }
}

/// A schema of its own in the database `PRIORY_TEST_POSTGRES_URL` names, dropped when dropped.
#[cfg(feature = "postgres")]
struct Postgres {
    url: String,
    schema: String,
}

#[cfg(feature = "postgres")]
impl Postgres {
    /// None when `PRIORY_TEST_POSTGRES_URL` is not set: the case skips.
    fn new(test: &str) -> Option<Self> {
        let Ok(url) = std::env::var("PRIORY_TEST_POSTGRES_URL") else {
            eprintln!("skipped: PRIORY_TEST_POSTGRES_URL is not set");
            return None;
        };
        let schema = format!("priory_log_{}_{test}", std::process::id());
        let mut client = ::postgres::Client::connect(&url, ::postgres::NoTls).unwrap();
        client
            .batch_execute(&format!(
                "DROP SCHEMA IF EXISTS {schema} CASCADE; CREATE SCHEMA {schema}"
            ))
            .unwrap();
        Some(Postgres { url, schema })
    }
}

#[cfg(feature = "postgres")]
impl Drop for Postgres {
    fn drop(&mut self) {
        if let Ok(mut client) = ::postgres::Client::connect(&self.url, ::postgres::NoTls) {
            let _ = client.batch_execute(&format!("DROP SCHEMA {} CASCADE", self.schema));
        }
    }
}

#[cfg(feature = "postgres")]
impl Store for Postgres {
    fn open(&self) -> Box<dyn Log> {
        let separator = if self.url.contains('?') { '&' } else { '?' };
        let url = format!(
            "{}{separator}options=-csearch_path%3D{schema}&application_name={schema}",
            self.url,
            schema = self.schema
        );
        Box::new(priory_intake::log::postgres::Postgres::connect(&url).unwrap())
    }
}

fn declared(store: &dyn Store) -> Box<dyn Log> {
    let log = store.open();
    log.declare(&Output::new(OUTPUT)).unwrap();
    log
}

fn event(n: u32) -> Message {
    Message::new("test.happened", serde_json::json!({ "n": n }))
}

fn numbers(log: &dyn Log, consumer: &str) -> Vec<u64> {
    log.read(consumer, OUTPUT, 100)
        .unwrap()
        .iter()
        .map(|delivery| delivery.message.data["n"].as_u64().unwrap())
        .collect()
}

fn publish_and_read_in_order(store: &dyn Store) {
    let log = declared(store);
    for n in [1, 2, 3] {
        log.publish(OUTPUT, &event(n)).unwrap();
    }
    assert_eq!(numbers(&*log, "daemon"), [1, 2, 3]);
}

fn dedupe_by_event_id(store: &dyn Store) {
    let log = declared(store);
    let first = event(1);
    let seq = log.publish(OUTPUT, &first).unwrap();
    let retry = Message {
        data: serde_json::json!({ "n": 2 }),
        ..first
    };
    assert_eq!(log.publish(OUTPUT, &retry).unwrap(), seq);
    assert_eq!(numbers(&*log, "daemon"), [1]);
    assert_eq!(log.message(seq).unwrap().message.data["n"], 1);
}

fn read_back_newest_first(store: &dyn Store) {
    let log = declared(store);
    let seqs: Vec<Seq> = (1..=3)
        .map(|n| log.publish(OUTPUT, &event(n)).unwrap())
        .collect();
    let page = |before| -> Vec<u64> {
        let stored = log.messages(OUTPUT, before, 2).unwrap();
        stored
            .iter()
            .map(|stored| stored.message.data["n"].as_u64().unwrap())
            .collect()
    };
    assert_eq!(page(None), [3, 2]);
    assert_eq!(page(Some(seqs[1])), [1]);
}

fn ack_and_redelivery(store: &dyn Store) {
    let log = declared(store);
    for n in [1, 2, 3] {
        log.publish(OUTPUT, &event(n)).unwrap();
    }
    let deliveries = log.read("daemon", OUTPUT, 100).unwrap();
    assert_eq!(numbers(&*log, "daemon"), [1, 2, 3]);
    log.ack("daemon", &deliveries[0]).unwrap();
    assert_eq!(numbers(&*log, "daemon"), [2, 3]);
    log.ack("daemon", &deliveries[2]).unwrap();
    log.ack("daemon", &deliveries[1]).unwrap();
    assert_eq!(numbers(&*log, "daemon"), Vec::<u64>::new());
}

fn cursor_survives_a_reopen(store: &dyn Store) {
    let log = declared(store);
    for n in [1, 2] {
        log.publish(OUTPUT, &event(n)).unwrap();
    }
    let first = log.read("daemon", OUTPUT, 1).unwrap().remove(0);
    log.ack("daemon", &first).unwrap();
    drop(log);
    assert_eq!(numbers(&*store.open(), "daemon"), [2]);
}

fn consumers_keep_separate_cursors(store: &dyn Store) {
    let log = declared(store);
    for n in [1, 2] {
        log.publish(OUTPUT, &event(n)).unwrap();
    }
    for delivery in log.read("daemon", OUTPUT, 100).unwrap() {
        log.ack("daemon", &delivery).unwrap();
    }
    assert_eq!(numbers(&*log, "daemon"), Vec::<u64>::new());
    assert_eq!(numbers(&*log, "export"), [1, 2]);
}

fn crash_between_read_and_ack_redelivers_once(store: &dyn Store) {
    let log = declared(store);
    let published = event(1);
    let seq = log.publish(OUTPUT, &published).unwrap();
    assert_eq!(numbers(&*log, "daemon"), [1]);
    drop(log);
    let log = store.open();
    let redelivered = log.read("daemon", OUTPUT, 100).unwrap();
    assert_eq!(redelivered.len(), 1);
    assert_eq!(redelivered[0].seq, seq);
    assert_eq!(redelivered[0].message, published);
    log.ack("daemon", &redelivered[0]).unwrap();
    drop(log);
    assert_eq!(numbers(&*store.open(), "daemon"), Vec::<u64>::new());
}

fn undeclared_output_holds_nothing(store: &dyn Store) {
    assert!(store.open().read("daemon", OUTPUT, 100).unwrap().is_empty());
}

fn outputs_keep_separate_cursors(store: &dyn Store) {
    let log = declared(store);
    log.declare(&Output::new("other.events")).unwrap();
    log.publish("other.events", &event(1)).unwrap();
    log.publish(OUTPUT, &event(2)).unwrap();
    assert_eq!(log.read("daemon", "other.events", 1).unwrap().len(), 1);
    let ours = log.read("daemon", OUTPUT, 1).unwrap();
    assert_eq!(ours.len(), 1);
    assert_eq!(ours[0].message.data["n"], 2);
}

/// A tap of the same consumer, made with herdr-bus's own CLI, delivers the output's messages too.
#[test]
fn herdr_acks_through_the_output_despite_a_tap() {
    let store = Herdr::new("tap");
    let bus = Bus::open(&store.0.join("bus.db")).unwrap();
    bus.connect("daemon", herdr_bus::TAP, herdr_bus::Start::Beginning)
        .unwrap();
    let log = declared(&store);
    for n in [1, 2, 3] {
        log.publish(OUTPUT, &event(n)).unwrap();
    }
    let mut read = Vec::new();
    loop {
        let deliveries = log.read("daemon", OUTPUT, 2).unwrap();
        if deliveries.is_empty() {
            break;
        }
        for delivery in deliveries {
            read.push(delivery.message.data["n"].as_u64().unwrap());
            log.ack("daemon", &delivery).unwrap();
        }
    }
    assert_eq!(read, [1, 2, 3]);
}

#[test]
fn herdr_reads_nothing_through_a_disabled_connection_despite_a_tap() {
    let store = Herdr::new("disabled");
    let bus = Bus::open(&store.0.join("bus.db")).unwrap();
    bus.connect("daemon", herdr_bus::TAP, herdr_bus::Start::Beginning)
        .unwrap();
    let log = declared(&store);
    log.publish(OUTPUT, &event(1)).unwrap();
    let ours = bus
        .connect("daemon", OUTPUT, herdr_bus::Start::Beginning)
        .unwrap();
    bus.set_enabled(ours, false).unwrap();
    assert!(log.read("daemon", OUTPUT, 1).unwrap().is_empty());
}

/// Runs every case against one adapter, each on a store of its own, or skips without one.
macro_rules! contract {
    ($adapter:ident, $store:expr) => {
        contract!(@ $adapter, $store;
            publish_and_read_in_order,
            dedupe_by_event_id,
            read_back_newest_first,
            ack_and_redelivery,
            cursor_survives_a_reopen,
            consumers_keep_separate_cursors,
            crash_between_read_and_ack_redelivers_once,
            undeclared_output_holds_nothing,
            outputs_keep_separate_cursors
        );
    };
    (@ $adapter:ident, $store:expr; $($case:ident),*) => {
        mod $adapter {
            use super::*;
            $(
                #[test]
                fn $case() {
                    if let Some(store) = $store(stringify!($case)) {
                        super::$case(&store);
                    }
                }
            )*
        }
    };
}

contract!(herdr, |test| Some(Herdr::new(test)));
contract!(memory, |test| Some(Memory::new(test)));
#[cfg(feature = "postgres")]
contract!(postgres, Postgres::new);

fn postgres_log(url_env: &str) -> EventLog {
    EventLog {
        backend: Backend::Postgres,
        url_env: Some(url_env.into()),
    }
}

#[cfg(not(feature = "postgres"))]
#[test]
fn a_postgres_log_needs_a_build_with_the_postgres_feature() {
    let error = log::open(&postgres_log("PRIORY_LOG_URL")).err().unwrap();
    assert_eq!(
        error.to_string(),
        "[log] backend = \"postgres\" needs a priory built with `--features postgres`"
    );
}

#[cfg(feature = "postgres")]
#[test]
fn a_postgres_log_names_its_unset_url_variable() {
    let error = log::open(&postgres_log("PRIORY_TEST_UNSET_URL"))
        .err()
        .unwrap();
    assert_eq!(
        error.to_string(),
        "[log] backend = \"postgres\" reads its connection string from $PRIORY_TEST_UNSET_URL, which is not set"
    );
}

#[cfg(feature = "postgres")]
#[test]
fn a_postgres_log_opens_the_database_its_url_variable_names() {
    let Some(store) = Postgres::new("open") else {
        return;
    };
    let output = format!("{OUTPUT}.{}", store.schema);
    let log = log::open(&postgres_log("PRIORY_TEST_POSTGRES_URL")).unwrap();
    let seq = log.publish(&output, &event(1)).unwrap();
    let other = priory_intake::log::postgres::Postgres::connect(&store.url).unwrap();
    let read = other.read("daemon", &output, 100).unwrap();
    let mut client = ::postgres::Client::connect(&store.url, ::postgres::NoTls).unwrap();
    client
        .execute(
            "DELETE FROM priory_log_messages WHERE output = $1",
            &[&output],
        )
        .unwrap();
    assert_eq!(
        read.iter().map(|delivery| delivery.seq).collect::<Vec<_>>(),
        [seq]
    );
}

#[cfg(feature = "postgres")]
#[test]
fn postgres_stamps_each_delivery_with_the_servers_clock_not_the_publishers() {
    let Some(store) = Postgres::new("clock") else {
        return;
    };
    let log = declared(&store);
    let skewed = event(1).time("2000-01-01T00:00:00Z".parse().unwrap());
    log.publish(OUTPUT, &skewed).unwrap();
    let delivery = log.read("daemon", OUTPUT, 1).unwrap().remove(0);
    let logged = delivery.logged.unwrap();
    assert!(
        (logged.as_second() - Timestamp::now().as_second()).abs() < 300,
        "{logged}"
    );
}

#[cfg(feature = "postgres")]
#[test]
fn postgres_reconnects_after_the_server_drops_its_connection() {
    let Some(store) = Postgres::new("reconnect") else {
        return;
    };
    let log = declared(&store);
    log.publish(OUTPUT, &event(1)).unwrap();
    let mut client = ::postgres::Client::connect(&store.url, ::postgres::NoTls).unwrap();
    client
        .execute(
            "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE application_name = $1",
            &[&store.schema],
        )
        .unwrap();
    log.publish(OUTPUT, &event(2)).unwrap();
    assert_eq!(numbers(&*log, "daemon"), [1, 2]);
}

#[cfg(feature = "postgres")]
#[test]
fn postgres_concurrent_publishers_leave_no_message_behind_a_cursor() {
    let Some(store) = Postgres::new("concurrent") else {
        return;
    };
    let (publishers, each) = (4, 50);
    let mut handled = Vec::new();
    std::thread::scope(|scope| {
        for publisher in 0..publishers {
            let store = &store;
            scope.spawn(move || {
                let log = declared(store);
                for n in 0..each {
                    log.publish(OUTPUT, &event(publisher * each + n)).unwrap();
                }
            });
        }
        let log = declared(&store);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while handled.len() < (publishers * each) as usize && std::time::Instant::now() < deadline {
            for delivery in log.read("daemon", OUTPUT, 7).unwrap() {
                handled.push(delivery.message.data["n"].as_u64().unwrap());
                log.ack("daemon", &delivery).unwrap();
            }
        }
    });
    handled.sort_unstable();
    assert_eq!(
        handled,
        (0..u64::from(publishers * each)).collect::<Vec<_>>()
    );
}
