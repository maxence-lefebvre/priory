use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use herdr_bus::{Bus, Message, Output, Seq, Stored, Timestamp};
use priory_intake::log::{Delivery, Log, Result};

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
    for delivery in log.read("daemon", "other.events", 100).unwrap() {
        log.ack("daemon", &delivery).unwrap();
    }
    assert_eq!(numbers(&*log, "daemon"), [2]);
}

/// A tap of the same consumer, made with herdr-bus's own CLI, delivers the output's messages too.
#[test]
fn herdr_acks_through_the_output_despite_a_tap() {
    let store = Herdr::new("tap");
    let bus = Bus::open(&store.0.join("bus.db")).unwrap();
    bus.connect("daemon", herdr_bus::TAP, herdr_bus::Start::Beginning)
        .unwrap();
    let log = declared(&store);
    log.publish(OUTPUT, &event(1)).unwrap();
    for delivery in log.read("daemon", OUTPUT, 100).unwrap() {
        log.ack("daemon", &delivery).unwrap();
    }
    assert_eq!(numbers(&*log, "daemon"), Vec::<u64>::new());
}

/// Runs every case against one adapter, each on a store of its own.
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
                    super::$case(&$store(stringify!($case)));
                }
            )*
        }
    };
}

contract!(herdr, Herdr::new);
contract!(memory, Memory::new);
