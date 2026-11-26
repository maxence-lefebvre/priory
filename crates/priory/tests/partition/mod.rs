//! Runners on a Postgres log, each through a proxy a test cuts or freezes, as a lost link does.
//! They run only when `PRIORY_TEST_POSTGRES_URL` names a database.

use super::proxy::{Link, Proxy};
use super::*;
use priory_intake::log::Log;
use priory_intake::log::postgres::{Postgres, client};

const WORK: &str = r#"
start = "work"

[[steps]]
id = "work"
decides = "model"
agent = "implementer"
prompt = "Do the work."
effect = "reversible"
next = { passed = "approve", failed = "stop" }

[[steps]]
id = "approve"
decides = "human"
effect = "none"
options = ["approved", "rejected"]
next = { approved = "done", rejected = "stop" }
"#;

const KEY: &str = "github:owner/repo#1";

/// A workflow whose one code step runs `echo found` and ends.
const POLL: &str = r#"
start = "poll"

[[steps]]
id = "poll"
decides = "code"
action = "run"
run = "echo found"
effect = "reversible"
next = { found = "done", running = "done", ok = "done", failed = "stop" }
"#;

/// A schema of its own in the test database, dropped when dropped.
struct Database {
    url: String,
    schema: String,
}

impl Database {
    /// None when `PRIORY_TEST_POSTGRES_URL` is not set: the test skips.
    fn new(test: &str) -> Option<Database> {
        let Ok(url) = std::env::var("PRIORY_TEST_POSTGRES_URL") else {
            eprintln!("skipped: PRIORY_TEST_POSTGRES_URL is not set");
            return None;
        };
        let schema = format!("priory_partition_{}_{test}", std::process::id());
        let mut sql = client::Client::connect(&url, client::NoTls).unwrap();
        // The log stamps each message with `now()`: a runner whose path finds this one first
        // publishes an hour in the past, as a runner whose last message is an hour old.
        sql.batch_execute(&format!(
            "DROP SCHEMA IF EXISTS {schema} CASCADE; DROP SCHEMA IF EXISTS {schema}_past CASCADE;
             CREATE SCHEMA {schema}; CREATE SCHEMA {schema}_past;
             CREATE FUNCTION {schema}_past.now() RETURNS timestamptz
               AS $$ SELECT pg_catalog.now() - interval '1 hour' $$ LANGUAGE sql STABLE"
        ))
        .unwrap();
        Some(Database { url, schema })
    }

    /// `base`, the test database's URL or one through a proxy, set to this test's schema.
    fn url(&self, base: &str) -> String {
        let separator = if base.contains('?') { '&' } else { '?' };
        format!("{base}{separator}options=-csearch_path%3D{}", self.schema)
    }

    /// `base` set to this test's schema, stamping what it publishes an hour in the past.
    fn past(&self, base: &str) -> String {
        format!("{}%2C{}_past%2Cpg_catalog", self.url(base), self.schema)
    }

    fn log(&self) -> Postgres {
        Postgres::connect(&self.url(&self.url), Duration::from_secs(10), false).unwrap()
    }

    fn publish(&self, message: &Message) {
        self.log().publish(OUTPUT, message).unwrap();
    }

    /// Runs `sql` in this test's schema.
    fn execute(&self, sql: &str) {
        let mut client = client::Client::connect(&self.url(&self.url), client::NoTls).unwrap();
        client.batch_execute(sql).unwrap();
    }

    /// Waits until the log holds a message of `kind` whose data has every field of `fields`.
    fn logged(&self, setup: &Setup, kind: &str, fields: serde_json::Value) {
        let deadline = Instant::now() + Duration::from_secs(60);
        let log = self.log();
        loop {
            let messages = log.messages(OUTPUT, None, 1000).unwrap();
            let found = messages.iter().any(|stored| {
                stored.message.kind == kind
                    && fields
                        .as_object()
                        .unwrap()
                        .iter()
                        .all(|(field, value)| &stored.message.data[field] == value)
            });
            if found {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "no {kind} {fields} in the log\ndaemon: {}",
                fs::read_to_string(setup.dir.join("daemon.log")).unwrap_or_default()
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

impl Drop for Database {
    fn drop(&mut self) {
        if let Ok(mut sql) = client::Client::connect(&self.url, client::NoTls) {
            let schema = &self.schema;
            let _ = sql.batch_execute(&format!(
                "DROP SCHEMA {schema} CASCADE; DROP SCHEMA {schema}_past CASCADE"
            ));
        }
    }
}

/// Runner `host` of a factory with runners `a` and `b`, on `database` through a proxy of its own.
fn runner_on(database: &Database, test: &str, host: &str) -> (Setup, Proxy) {
    let mut setup = runner(test, host, None);
    let proxy = Proxy::new(&database.url);
    let factory = "name = \"demo\"\n[log]\nbackend = \"postgres\"\ntimeout = \"2s\"\n";
    fs::write(setup.dir.join("factory/factory.toml"), factory).unwrap();
    fs::write(setup.dir.join("factory/workflows/work.toml"), WORK).unwrap();
    setup.log_url = Some(database.url(&proxy.url));
    (setup, proxy)
}

fn work(issue: u64) -> Message {
    let mut request = requested(issue);
    request.data["workflow"] = json!("work");
    request
}

fn alive(setup: &mut Setup) -> bool {
    setup
        .daemon
        .as_mut()
        .is_some_and(|daemon| daemon.try_wait().unwrap().is_none())
}

fn stderr(exit: &Exit) -> String {
    String::from_utf8_lossy(&exit.stderr).into_owned()
}

/// Polls the desk's banner on outside links, `intake` in `priory desk --json`, until the log is
/// unreachable or reachable as `unreachable` says.
fn until_unreachable(setup: &Setup, unreachable: bool) {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let desk = setup.priory(&["desk", "--json"]);
        let desk: serde_json::Value = serde_json::from_slice(&desk.stdout).unwrap();
        let banner = desk["intake"].as_str().unwrap_or_default();
        if banner.starts_with("the log is unreachable since ") == unreachable {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "banner: {banner}\ndaemon: {}",
            fs::read_to_string(setup.dir.join("daemon.log")).unwrap_or_default()
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn a_runner_cut_off_from_its_log_finishes_its_turn_and_takes_an_answer_from_elsewhere_once_back() {
    let Some(database) = Database::new("cut") else {
        return;
    };
    let (mut a, link) = runner_on(&database, "cut", "a");
    let (mut b, _b_link) = runner_on(&database, "cut", "b");
    let exe = Path::new(env!("CARGO_BIN_EXE_priory"));
    database.publish(&work(1));
    a.hold("hold-call");
    a.calls(&[result("passed")]);
    a.start_as(exe);
    let kept = json!({ "key": KEY, "runner": "a", "hold": "kept" });
    database.logged(&a, "priory.task.held", kept);
    b.start_as(exe);
    a.written("fake/submitted");

    link.set(Link::Cut);
    until_unreachable(&a, true);
    a.release("hold-call");
    let asks = a.until("asks", |rows| rows.len() == 1);
    assert!(alive(&mut a), "a's daemon rides out the cut");
    let factory = a.dir.join("factory");
    let give = a.priory(&["give", factory.to_str().unwrap(), KEY, "b"]);
    assert!(
        stderr(&give).contains("the log is unreachable since ")
            && stderr(&give).ends_with(": try again when it is back\n"),
        "{}",
        stderr(&give)
    );
    let ask = format!("a:{}", asks[0][0]);
    let factory = b.dir.join("factory");
    let dir = factory.to_str().unwrap();
    let answered = b.priory(&["answer", dir, &ask, "approved", "--revision", "1"]);
    assert!(answered.status.success(), "{}", stderr(&answered));
    std::thread::sleep(Duration::from_secs(3));
    assert_eq!(a.own_tasks()[0][4], "asleep", "the answer waits on the log");

    link.set(Link::Up);
    let tasks = a.until_own_tasks(|rows| rows[0][4] == "done");
    assert_eq!(tasks.len(), 1);
    let ended = json!({ "key": KEY, "runner": "a", "hold": "ended" });
    database.logged(&a, "priory.task.held", ended);
    until_unreachable(&a, false);
    assert_eq!(a.rows("runs").len(), 1, "the task ran once, on a");
    assert!(b.rows("runs").is_empty() && b.own_tasks().is_empty());
}

#[test]
fn a_runner_whose_link_hangs_keeps_answering_its_own_asks_and_publishes_once_it_thaws() {
    let Some(database) = Database::new("frozen") else {
        return;
    };
    let (mut a, link) = runner_on(&database, "frozen", "a");
    let exe = Path::new(env!("CARGO_BIN_EXE_priory"));
    database.publish(&work(1));
    a.calls(&[result("passed")]);
    a.start_as(exe);
    let asks = a.until("asks", |rows| rows.len() == 1);
    let kept = json!({ "key": KEY, "runner": "a", "hold": "kept" });
    database.logged(&a, "priory.task.held", kept);

    link.set(Link::Frozen);
    until_unreachable(&a, true);
    let answered = a.priory(&["answer", &asks[0][0], "approved", "--revision", "1"]);
    assert!(answered.status.success(), "{}", stderr(&answered));
    a.within_own_tasks(10, |rows| rows[0][4] == "done");
    assert!(alive(&mut a));

    link.set(Link::Up);
    let ended = json!({ "key": KEY, "runner": "a", "hold": "ended" });
    database.logged(&a, "priory.task.held", ended);
    until_unreachable(&a, false);
}

#[test]
fn a_task_that_never_started_on_a_cut_off_runner_moves_once_its_lease_ends_and_runs_once() {
    let Some(database) = Database::new("lease") else {
        return;
    };
    let (mut a, link) = runner_on(&database, "lease", "a");
    a.log_url = Some(database.past(&link.url));
    let (mut b, _b_link) = runner_on(&database, "lease", "b");
    let exe = Path::new(env!("CARGO_BIN_EXE_priory"));
    database.publish(&work(1));
    // No room on `a`, so the task waits there.
    a.memory(0);
    a.start_as(exe);
    let waiting = json!({ "key": KEY, "runner": "a", "hold": "waiting" });
    database.logged(&a, "priory.task.held", waiting);

    link.set(Link::Cut);
    until_unreachable(&a, true);
    b.calls(&[result("passed")]);
    b.start_as(exe);
    let takeover = json!({ "key": KEY, "runner": "b", "from": "a" });
    database.logged(&b, "priory.task.claimed", takeover);
    b.until("asks", |rows| rows.len() == 1);
    // Room on `a` again, while it still cannot read the log: it starts nothing, and says why.
    a.memory(60);
    let claiming = "waits for its claim to reach the log";
    a.until_own_tasks(|rows| rows[0][7] == claiming);
    let tasks = a.priory(&["tasks", "--json"]);
    let tasks: serde_json::Value = serde_json::from_slice(&tasks.stdout).unwrap();
    assert_eq!(tasks["tasks"][0]["reason"], claiming, "{tasks}");
    assert!(
        a.rows("runs").is_empty(),
        "a starts no task the log may have moved"
    );
    assert!(alive(&mut a));

    link.set(Link::Up);
    let tasks = a.until_own_tasks(|rows| rows[0][4] == "dropped");
    assert_eq!(tasks.len(), 1);
    assert!(a.rows("runs").is_empty(), "the task ran once, on b");
    assert_eq!(b.rows("runs").len(), 1);
}

#[test]
fn a_code_step_whose_kept_never_comes_back_holds_no_later_tasks_action() {
    let Some(database) = Database::new("uncleared") else {
        return;
    };
    let (mut a, _link) = runner_on(&database, "uncleared", "a");
    fs::write(a.dir.join("factory/workflows/poll.toml"), POLL).unwrap();
    // The log turns the first task's `kept` into `waiting`, as a link lost right after it would
    // leave it unread: that task waits for good, and the later one must not wait behind it.
    database.log();
    database.execute(
        r#"CREATE FUNCTION unkept() RETURNS trigger AS $$ BEGIN
             IF NEW.message LIKE '%"key":"github:owner/repo#1"%' THEN
               NEW.message := replace(NEW.message, '"hold":"kept"', '"hold":"waiting"');
             END IF;
             RETURN NEW;
           END $$ LANGUAGE plpgsql;
           CREATE TRIGGER unkept BEFORE INSERT ON priory_log_messages
             FOR EACH ROW EXECUTE FUNCTION unkept()"#,
    );
    for issue in [1, 2] {
        let mut request = requested(issue);
        request.data["workflow"] = json!("poll");
        database.publish(&request);
    }
    a.start_as(Path::new(env!("CARGO_BIN_EXE_priory")));

    let tasks = a.until_own_tasks(|rows| rows.len() == 2 && rows[1][4] == "done");
    assert_eq!(tasks[0][1], "github:owner/repo#1");
    assert_eq!(tasks[0][4], "queued");
    assert_eq!(tasks[0][7], "waits for its claim to reach the log");
    assert_eq!(
        a.rows("actions").len(),
        1,
        "only the later task's action ran"
    );
}

#[test]
fn a_given_task_s_run_ends_even_when_the_ack_of_the_gift_is_lost() {
    let Some(database) = Database::new("ack") else {
        return;
    };
    let (mut a, _link) = runner_on(&database, "ack", "a");
    let request = work(1);
    database.publish(&request);
    a.hold("hold-call");
    a.calls(&[result("passed")]);
    a.start_as(Path::new(env!("CARGO_BIN_EXE_priory")));
    let kept = json!({ "key": KEY, "runner": "a", "hold": "kept" });
    database.logged(&a, "priory.task.held", kept);
    a.written("fake/submitted");
    let sessions = || {
        let listed = a.tmux(&["list-sessions", "-F", "#{session_name}"]);
        String::from_utf8_lossy(&listed.stdout).into_owned()
    };
    assert!(sessions().contains("priory-1"), "{}", sessions());

    // Acking the gift fails as a lost link does, every time.
    database.execute(
        r#"CREATE FUNCTION lose_ack() RETURNS trigger AS $$ BEGIN
             IF EXISTS (SELECT 1 FROM priory_log_messages
                        WHERE seq = NEW.seq AND message LIKE '%"given":true%') THEN
               RAISE EXCEPTION 'the link dropped' USING ERRCODE = '08006';
             END IF;
             RETURN NEW;
           END $$ LANGUAGE plpgsql;
           CREATE TRIGGER lose_ack BEFORE INSERT OR UPDATE ON priory_log_cursors
             FOR EACH ROW EXECUTE FUNCTION lose_ack()"#,
    );
    let gift = json!({
        "key": KEY, "runner": "b", "request": request.id.to_string(), "from": "a", "given": true,
    });
    database.publish(&Message::new("priory.task.claimed", gift).subject(KEY));

    a.until_own_tasks(|rows| rows[0][4] == "dropped");
    until_unreachable(&a, true);
    let deadline = Instant::now() + Duration::from_secs(10);
    while sessions().contains("priory-1") {
        assert!(Instant::now() < deadline, "the run's session still runs");
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(alive(&mut a), "a's daemon rides out the lost ack");
}

#[test]
fn a_task_reopened_after_its_hold_ended_starts_only_once_the_log_says_its_runner_keeps_it() {
    let Some(database) = Database::new("reopened") else {
        return;
    };
    let (mut a, _a_link) = runner_on(&database, "reopened", "a");
    let (mut b, _b_link) = runner_on(&database, "reopened", "b");
    let exe = Path::new(env!("CARGO_BIN_EXE_priory"));
    a.calls(&[result("passed")]);
    database.publish(&requested(1));
    a.start_as(exe);
    a.until_own_tasks(|rows| rows.first().is_some_and(|row| row[4] == "done"));
    let ended = json!({ "key": KEY, "runner": "a", "hold": "ended" });
    database.logged(&a, "priory.task.held", ended);
    // a's `kept` never reaches the log, as a link lost right after the reopen would leave it.
    database.execute(
        r#"CREATE FUNCTION unsent() RETURNS trigger AS $$ BEGIN
             IF NEW.message LIKE '%"runner":"a"%' AND NEW.message LIKE '%"hold":"kept"%' THEN
               RAISE EXCEPTION 'the link dropped' USING ERRCODE = '08006';
             END IF;
             RETURN NEW;
           END $$ LANGUAGE plpgsql;
           CREATE TRIGGER unsent BEFORE INSERT ON priory_log_messages
             FOR EACH ROW EXECUTE FUNCTION unsent()"#,
    );
    // The comment comes after the task was done, to the second.
    std::thread::sleep(Duration::from_millis(1100));
    database.publish(
        &Message::new(
            "github.issues.comment",
            json!({ "repo": "owner/repo", "issue": 1, "actor": "ana", "body": "Still broken.",
                "url": "u", "github_id": 901 }),
        )
        .subject("owner/repo#1")
        .time(herdr_bus::Timestamp::now()),
    );
    a.until_own_tasks(|rows| rows[0][4] != "done");
    until_unreachable(&a, true);

    // The log still says the key ended, so b claims the next request for it, and wins.
    database.publish(&requested(1));
    b.start_as(exe);
    let tasks = a.until_own_tasks(|rows| rows[0][4] == "dropped");
    assert_eq!(tasks.len(), 1, "{tasks:?}");
    let task = a.priory(&["task", &tasks[0][0], "--json"]);
    let task: serde_json::Value = serde_json::from_slice(&task.stdout).unwrap();
    assert_eq!(task["reason"], "claimed by runner b", "{task}");
    assert_eq!(
        a.rows("runs").len(),
        1,
        "a ran the reopened task before the log said a keeps it"
    );
}

#[test]
fn a_daemon_whose_log_returns_runs_no_task_that_the_rest_of_the_log_withdraws() {
    let Some(database) = Database::new("withdrawn") else {
        return;
    };
    let mut setup = reviews("withdrawn-while-cut");
    let link = Proxy::new(&database.url);
    let factory = setup.dir.join("factory/factory.toml");
    let mut text = fs::read_to_string(&factory).unwrap();
    text.push_str("\n[log]\nbackend = \"postgres\"\ntimeout = \"2s\"\n");
    fs::write(factory, text).unwrap();
    setup.log_url = Some(database.url(&link.url));
    let log = database.log();
    let event = |at: u64| Message::new("t.event", json!({ "at": at }));
    log.publish(OUTPUT, &event(0)).unwrap();
    setup.start_as(Path::new(env!("CARGO_BIN_EXE_priory")));
    setup.until("inbox", |rows| rows.len() == 1);

    link.set(Link::Cut);
    until_unreachable(&setup, true);
    for message in review_request(1) {
        log.publish(OUTPUT, &message).unwrap();
    }
    for at in 1..=150 {
        log.publish(OUTPUT, &event(at)).unwrap();
    }
    let removed = review_event("github.pull_request.review_request_removed", 2);
    log.publish(OUTPUT, &removed).unwrap();
    link.set(Link::Up);

    setup.until("tasks", |rows| rows.len() == 1 && rows[0][4] == "dropped");
    assert!(setup.rows("runs").is_empty());
}

/// Sends the `log` group from runner `setup` to `collector` every 10 seconds.
fn metered(setup: &Setup, collector: &super::metrics::Collector) {
    let factory = setup.dir.join("factory/factory.toml");
    let mut text = fs::read_to_string(&factory).unwrap();
    text.push_str(&format!(
        "\n[telemetry]\notlp = \"{}\"\nmetrics = [\"log\"]\nmetrics_every = \"10s\"\n",
        collector.url
    ));
    fs::write(factory, text).unwrap();
}

/// Whether `request` came from runner `name`.
fn from(request: &serde_json::Value, name: &str) -> bool {
    super::metrics::resource(request)["priory.runner"] == name
}

/// The count of probes of `outcome` in `request`.
fn probes(request: &serde_json::Value, outcome: &str) -> f64 {
    let wanted = json!({ "priory.log.backend": "postgres", "outcome": outcome });
    super::metrics::points(request, "priory.log.probe.duration")
        .into_iter()
        .filter(|(attributes, _)| *attributes == wanted)
        .map(|(_, count)| count)
        .sum()
}

#[test]
fn a_cut_runner_sends_its_failed_probes_and_its_unreachable_age_and_the_other_sees_it_fall_behind()
{
    let Some(database) = Database::new("metrics_cut") else {
        return;
    };
    let collector = super::metrics::Collector::start();
    let (mut a, _a_link) = runner_on(&database, "metrics_cut", "a");
    let (mut b, b_link) = runner_on(&database, "metrics_cut", "b");
    metered(&a, &collector);
    metered(&b, &collector);
    let exe = Path::new(env!("CARGO_BIN_EXE_priory"));
    a.start_as(exe);
    b.start_as(exe);
    collector.until(&b, |requests| {
        requests
            .iter()
            .any(|request| from(request, "b") && probes(request, "ok") >= 1.0)
    });

    b_link.set(Link::Cut);
    until_unreachable(&b, true);
    database.publish(&Message::new("test.happened", json!({ "n": 1 })));

    let cut = |request: &serde_json::Value| {
        let age = super::metrics::points(request, "priory.log.unreachable.age");
        from(request, "b")
            && probes(request, "error") >= 1.0
            && age.first().is_some_and(|age| age.1 >= 1.0)
    };
    let requests = collector.until(&b, |requests| requests.iter().any(cut));
    let request = requests.iter().find(|request| cut(request)).unwrap();
    let waits = super::metrics::histogram(request, "priory.log.connection.wait");
    let waits = &waits["dataPoints"][0];
    assert!(
        waits["count"].as_str().unwrap().parse::<u64>().unwrap() >= 1,
        "{waits}"
    );
    assert_eq!(waits["attributes"], json!([]));
    let queued = super::metrics::points(request, "priory.log.queued");
    assert!(queued.len() == 1 && queued[0].1 >= 0.0, "{queued:?}");
    assert!(
        super::metrics::points(request, "priory.log.backlog").is_empty(),
        "a runner that cannot reach its log sends no backlog: {request}"
    );

    let behind = json!({ "priory.log.output": OUTPUT, "priory.log.consumer": "priory.b" });
    collector.until(&a, |requests| {
        requests.iter().any(|request| {
            from(request, "a")
                && super::metrics::points(request, "priory.log.backlog")
                    .iter()
                    .any(|(attributes, count)| *attributes == behind && *count >= 1.0)
        })
    });
    assert!(alive(&mut a) && alive(&mut b));
}

#[test]
fn a_runner_whose_link_hangs_counts_its_probes_as_timeouts() {
    let Some(database) = Database::new("metrics_frozen") else {
        return;
    };
    let collector = super::metrics::Collector::start();
    let (mut a, link) = runner_on(&database, "metrics_frozen", "a");
    metered(&a, &collector);
    a.start_as(Path::new(env!("CARGO_BIN_EXE_priory")));
    collector.until(&a, |requests| {
        requests.iter().any(|request| probes(request, "ok") >= 1.0)
    });

    link.set(Link::Frozen);
    until_unreachable(&a, true);

    collector.until(&a, |requests| {
        requests
            .iter()
            .any(|request| probes(request, "timeout") >= 1.0)
    });
    assert!(alive(&mut a));
}
