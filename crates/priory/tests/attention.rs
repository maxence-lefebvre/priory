mod tmux_dir;

use serde_json::{Value, json};
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::Duration;
use tmux_dir::TmuxDir;

const SERVE: &str =
    "name = \"demo\"\n\n[serve]\nlisten = \"127.0.0.1:0\"\ntoken_env = \"TEST_TOKEN\"\n";

const TOKEN: &str = "s3cret-token";

const NOW: &str = "2026-11-17T11:00:00Z";

/// A factory, a state file and Priory home under the test's own directory, on a clock fixed at
/// `NOW`. Dropping it kills the server it started.
struct Setup {
    dir: PathBuf,
    server: Option<Child>,
    tmux: TmuxDir,
}

impl Setup {
    fn new(name: &str) -> Self {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join("attention")
            .join(name);
        priory_runtime::runtime::remove_sealed(&dir);
        fs::create_dir_all(dir.join("factory")).unwrap();
        fs::write(dir.join("factory/factory.toml"), SERVE).unwrap();
        let setup = Setup {
            tmux: TmuxDir::new(&dir),
            dir,
            server: None,
        };
        succeeded(&setup.priory(&["tasks"]));
        setup
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_priory"));
        command
            .env_remove("TMUX")
            .env("TMUX_TMPDIR", self.tmux.path())
            .env("HERDR_BUS_DB", self.dir.join("bus.db"))
            .env("PRIORY_DB", self.dir.join("priory.db"))
            .env("PRIORY_HOME", self.dir.join("home"))
            .env("TEST_TOKEN", TOKEN)
            .env("TZ", "UTC")
            .env("PRIORY_NOW", NOW);
        command
    }

    fn priory(&self, args: &[&str]) -> Output {
        self.command().args(args).output().unwrap()
    }

    /// `priory report attention <args>`, which must succeed, as its standard output.
    fn attention(&self, args: &[&str]) -> String {
        let output = self.priory(&[&["report", "attention"][..], args].concat());
        succeeded(&output);
        String::from_utf8(output.stdout).unwrap()
    }

    fn attention_json(&self, args: &[&str]) -> Value {
        serde_json::from_str(&self.attention(&[args, &["--json"]].concat())).unwrap()
    }

    fn db(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(self.dir.join("priory.db")).unwrap()
    }

    /// Adds task `id` in `workflow` and `lane`, created at `created`.
    fn task(&self, id: i64, workflow: &str, lane: &str, created: &str) {
        let db = self.db();
        db.execute(
            "INSERT INTO tasks (id, key, workflow, step, state, lane, trigger, source)
             VALUES (?1, 'issue-' || ?1, ?2, 'work', 'asleep', ?3, 'cli', 'cli-' || ?1)",
            rusqlite::params![id, workflow, lane],
        )
        .unwrap();
        db.execute(
            "UPDATE tasks SET created = ?2 WHERE id = ?1",
            rusqlite::params![id, created],
        )
        .unwrap();
    }

    /// Adds ask `id` of `task`, asked at `created`, and its answer at `answered`, if any.
    fn ask(&self, id: i64, task: i64, created: &str, answered: Option<&str>) {
        let db = self.db();
        db.execute(
            "INSERT INTO asks (id, task, step, kind, question, options, created)
             VALUES (?1, ?2, 'work', 'agent', 'Which?', '[]', ?3)",
            rusqlite::params![id, task, created],
        )
        .unwrap();
        db.execute(
            "UPDATE ask_revisions SET created = ?2 WHERE ask = ?1",
            rusqlite::params![id, created],
        )
        .unwrap();
        if let Some(answered) = answered {
            db.execute(
                "UPDATE asks SET answer = 'yes', answered = ?2 WHERE id = ?1",
                rusqlite::params![id, answered],
            )
            .unwrap();
        }
    }

    /// Appends a person's snooze of ask `ask` of `task` at `time`.
    fn snooze(&self, task: i64, ask: i64, time: &str) {
        let data = json!({
            "task": task.to_string(), "item": { "kind": "ask", "id": ask.to_string() },
            "until": "2026-11-20T00:00:00Z",
        });
        self.db()
            .execute(
                "INSERT INTO trace (id, type, time, event) VALUES
                 (lower(hex(randomblob(16))), 'factory.task.snoozed', ?1,
                  json_object('data', json(?2)))",
                [time, &data.to_string()],
            )
            .unwrap();
    }

    /// Starts `priory serve` and returns the address it prints once it listens.
    fn serve(&mut self) -> String {
        let mut child = self
            .command()
            .args(["serve", self.dir.join("factory").to_str().unwrap()])
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut stderr = BufReader::new(child.stderr.take().unwrap());
        self.server = Some(child);
        loop {
            let mut line = String::new();
            assert!(stderr.read_line(&mut line).unwrap() > 0, "serve exited");
            if let Some(address) = line.trim().strip_prefix("priory: serving on http://") {
                return address.to_owned();
            }
        }
    }
}

impl Drop for Setup {
    fn drop(&mut self) {
        if let Some(mut server) = self.server.take() {
            let _ = server.kill();
            let _ = server.wait();
        }
    }
}

fn succeeded(output: &Output) {
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn ask_waits_give_median_and_p90_with_and_without_the_persons_away_time() {
    let setup = Setup::new("waits");
    setup.task(1, "implement", "lane-a", "2026-11-16T08:00:00Z");
    // Waits of 10, 25, 300 and 20 minutes. The 300 sits in a 5.5-hour gap between the person's acts,
    // so it counts as away; the snooze ends the fourth wait. Ask 5 is still open.
    setup.ask(1, 1, "2026-11-16T09:00:00Z", Some("2026-11-16T09:10:00Z"));
    setup.ask(2, 1, "2026-11-16T09:05:00Z", Some("2026-11-16T09:30:00Z"));
    setup.ask(3, 1, "2026-11-16T10:00:00Z", Some("2026-11-16T15:00:00Z"));
    setup.ask(4, 1, "2026-11-16T15:00:00Z", None);
    setup.snooze(1, 4, "2026-11-16T15:20:00Z");
    setup.ask(5, 1, "2026-11-16T16:00:00Z", None);

    let attention = setup.attention_json(&["--since", "2026-11-16T00:00:00Z"]);

    assert_eq!(
        attention["waits"],
        json!({ "acted": 4, "open": 1, "median_minutes": 22.5, "p90_minutes": 300.0,
                "median_present_minutes": 15.0, "p90_present_minutes": 25.0 })
    );
    let text = setup.attention(&["--since", "2026-11-16T00:00:00Z"]);
    assert!(
        text.contains("asks\tacted\topen\tmedian\tp90\tmedian present\tp90 present\nall\t4\t1\t22.5\t300.0\t15.0\t25.0\n"),
        "{text}"
    );
}

#[test]
fn a_tasks_wall_time_splits_into_working_waiting_on_the_person_and_on_the_world() {
    let setup = Setup::new("split");
    setup.task(1, "implement", "lane-a", "2026-11-16T08:00:00Z");
    // Working 08:00 to 09:00, the person 09:00 to 09:30, working to 10:00, checks to 11:30,
    // working to 11:50, then queued until the task ends at 12:00.
    setup.ask(1, 1, "2026-11-16T09:00:00Z", Some("2026-11-16T09:30:00Z"));
    setup
        .db()
        .execute_batch(
            "INSERT INTO runs (id, task, step, tmux, session, started, ended) VALUES
               (1, 1, 'work', 't', 's', '2026-11-16T08:00:00Z', '2026-11-16T09:00:00Z'),
               (2, 1, 'work', 't', 's', '2026-11-16T09:30:00Z', '2026-11-16T10:00:00Z'),
               (3, 1, 'work', 't', 's', '2026-11-16T11:30:00Z', '2026-11-16T11:50:00Z');
             INSERT INTO waits (task, run, kind, created, fired) VALUES
               (1, 2, 'checks', '2026-11-16T10:00:00Z', '2026-11-16T11:30:00Z');
             UPDATE tasks SET state = 'done' WHERE id = 1;
             UPDATE task_ends SET ended = '2026-11-16T12:00:00.000Z' WHERE task = 1;",
        )
        .unwrap();

    let attention = setup.attention_json(&["--since", "2026-11-16T00:00:00Z"]);

    assert_eq!(
        attention["tasks"],
        json!([{ "id": 1, "key": "issue-1", "workflow": "implement", "lane": "lane-a",
                 "state": "done", "since": "2026-11-16T08:00:00Z",
                 "until": "2026-11-16T12:00:00.000Z", "wall_minutes": 240.0,
                 "working_minutes": 110.0, "person_wait_minutes": 30.0,
                 "world_wait_minutes": 90.0, "queued_minutes": 10.0 }])
    );
    let text = setup.attention(&["--since", "2026-11-16T00:00:00Z"]);
    assert!(
        text.contains("task\tkey\tworkflow\tlane\tstate\twall\tworking\twaiting on person\twaiting on world\tqueued\n1\tissue-1\timplement\tlane-a\tdone\t240.0\t110.0\t30.0\t90.0\t10.0\n"),
        "{text}"
    );
}

#[test]
fn person_minutes_per_merged_pull_request_count_each_act_up_to_five_minutes() {
    let setup = Setup::new("per-merge");
    setup.task(1, "implement", "lane-a", "2026-11-16T08:00:00Z");
    setup.task(2, "review", "lane-b", "2026-11-16T08:00:00Z");
    // Acts at 10:00, 10:01, 10:31 and 13:31 cost 5 (the first), 1, 5 and 5 minutes. The last is a
    // dismissal after three hours away.
    setup.ask(1, 1, "2026-11-16T09:00:00Z", Some("2026-11-16T10:00:00Z"));
    setup.ask(2, 1, "2026-11-16T09:00:00Z", Some("2026-11-16T10:01:00Z"));
    setup.ask(3, 2, "2026-11-16T10:00:00Z", Some("2026-11-16T10:31:00Z"));
    setup.ask(4, 2, "2026-11-16T13:00:00Z", None);
    setup
        .db()
        .execute_batch(
            "UPDATE asks SET withdrawn = 1, withdrawn_reason = 'dismissed' WHERE id = 4;
             UPDATE ask_withdrawals SET created = '2026-11-16T13:31:00.000Z' WHERE ask = 4;
             INSERT INTO owns (task, resource) VALUES (1, 'github:o/r#1'), (2, 'github:o/r#2');
             INSERT INTO inbox (id, type, subject, time, data, links, outcome) VALUES
               ('m1', 'github.pull_request.closed', 'o/r#1', '2026-11-16T11:00:00Z',
                '{\"merged\":true}', '[]', 'stored'),
               ('m2', 'github.pull_request.closed', 'o/r#2', '2026-11-16T14:00:00Z',
                '{\"merged\":true}', '[]', 'stored'),
               ('m3', 'github.pull_request.closed', 'o/r#3', '2026-11-16T14:00:00Z',
                '{\"merged\":true}', '[]', 'stored');",
        )
        .unwrap();

    let attention = setup.attention_json(&["--since", "2026-11-16T00:00:00Z"]);

    let headline = [
        "since",
        "until",
        "by",
        "acts",
        "person_minutes",
        "merges",
        "person_minutes_per_merge",
    ]
    .map(|field| (field.to_owned(), attention[field].clone()));
    assert_eq!(
        Value::Object(headline.into_iter().collect()),
        json!({ "since": "2026-11-16T00:00:00.000Z", "until": "2026-11-17T11:00:00.000Z",
                "by": "workflow", "acts": 4, "person_minutes": 16.0, "merges": 2,
                "person_minutes_per_merge": 8.0 })
    );
    assert_eq!(
        attention["split"],
        json!([
            { "workflow": "implement", "acts": 2, "person_minutes": 6.0,
              "waits": { "acted": 2, "open": 0, "median_minutes": 60.5, "p90_minutes": 61.0,
                         "median_present_minutes": 60.5, "p90_present_minutes": 61.0 } },
            { "workflow": "review", "acts": 2, "person_minutes": 10.0,
              "waits": { "acted": 2, "open": 0, "median_minutes": 31.0, "p90_minutes": 31.0,
                         "median_present_minutes": 15.5, "p90_present_minutes": 31.0 } },
        ])
    );
    let lanes = setup.attention_json(&["--since", "2026-11-16T00:00:00Z", "--by", "lane"]);
    assert_eq!(lanes["split"][1]["lane"], "lane-b");
    let text = setup.attention(&["--since", "2026-11-16T00:00:00Z"]);
    assert!(
        text.contains("since\tuntil\tacts\tperson minutes\tmerges\tper merge\n2026-11-16T00:00:00.000Z\t2026-11-17T11:00:00.000Z\t4\t16.0\t2\t8.0\n"),
        "{text}"
    );
    assert!(
        text.contains("workflow\tacts\tperson minutes\tacted\topen\tmedian\tp90\tmedian present\tp90 present\nimplement\t2\t6.0\t2\t0\t60.5\t61.0\t60.5\t61.0\nreview\t2\t10.0\t2\t0\t31.0\t31.0\t15.5\t31.0\n"),
        "{text}"
    );
}

#[test]
fn revisions_withdrawn_waits_and_tasks_ended_before_task_ends_keep_their_times() {
    let setup = Setup::new("history");
    setup.task(1, "implement", "lane-a", "2026-11-16T08:00:00Z");
    setup.task(2, "implement", "lane-a", "2026-11-16T08:00:00Z");
    // Task 1 runs 09:00 to 11:00 through an answered ask and a wait its run withdrew.
    setup.ask(1, 1, "2026-11-16T09:30:00Z", Some("2026-11-16T10:00:00Z"));
    // Task 2's ask is answered at 09:10 and asked again in the same second, then answered at 09:20.
    setup.ask(2, 2, "2026-11-16T09:00:00Z", Some("2026-11-16T09:10:00Z"));
    setup
        .db()
        .execute_batch(
            "INSERT INTO runs (id, task, step, tmux, session, started, ended) VALUES
               (1, 1, 'work', 't', 's', '2026-11-16T09:00:00Z', '2026-11-16T11:00:00Z');
             INSERT INTO waits (task, run, kind, created, withdrawn, withdrawn_reason) VALUES
               (1, 1, 'checks', '2026-11-16T09:30:00Z', 1, 'moved-on');
             UPDATE tasks SET state = 'done' WHERE id = 1;
             UPDATE task_ends SET ended = '2026-11-16T12:00:00.000Z' WHERE task = 1;
             UPDATE asks SET revision = 2, answer = NULL, answered = NULL WHERE id = 2;
             UPDATE ask_revisions SET created = '2026-11-16T09:10:00Z' WHERE ask = 2 AND revision = 2;
             UPDATE asks SET answer = 'yes', answered = '2026-11-16T09:20:00Z' WHERE id = 2;
             UPDATE tasks SET state = 'done' WHERE id = 2;
             DELETE FROM task_ends WHERE task = 2;
             UPDATE tasks SET changed = '2026-11-16T12:00:00Z' WHERE id = 2;",
        )
        .unwrap();

    let attention = setup.attention_json(&["--since", "2026-11-16T00:00:00Z"]);

    let split = |task: &Value| {
        [
            "wall_minutes",
            "working_minutes",
            "person_wait_minutes",
            "world_wait_minutes",
            "queued_minutes",
        ]
        .map(|field| task[field].as_f64().unwrap())
    };
    assert_eq!(
        split(&attention["tasks"][0]),
        [240.0, 90.0, 30.0, 0.0, 120.0]
    );
    assert_eq!(
        split(&attention["tasks"][1]),
        [240.0, 0.0, 20.0, 0.0, 220.0]
    );
    assert_eq!(attention["waits"]["acted"], 3);
    assert_eq!(attention["waits"]["median_minutes"], 10.0);
}

/// `GET path` with the token, as the status code and body.
fn get(address: &str, path: &str) -> (u16, String) {
    let mut stream = TcpStream::connect(address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer {TOKEN}\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    let status = response[9..12].parse().unwrap();
    let body = response.split_once("\r\n\r\n").map_or("", |(_, body)| body);
    (status, body.to_owned())
}

#[test]
fn the_attention_endpoint_answers_what_report_attention_json_prints() {
    let mut setup = Setup::new("endpoint");
    setup.task(1, "implement", "lane-a", "2026-11-16T08:00:00Z");
    setup.ask(1, 1, "2026-11-16T09:00:00Z", Some("2026-11-16T10:00:00Z"));
    let address = setup.serve();

    for (path, args) in [
        ("/api/attention", vec![]),
        (
            "/api/attention?since=3%20days&by=kind",
            vec!["--since", "3 days", "--by", "kind"],
        ),
    ] {
        let (status, body) = get(&address, path);
        assert_eq!(status, 200, "{path}: {body}");
        let served: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(served, setup.attention_json(&args), "{path}");
    }
    for path in [
        "/api/attention?by=repo",
        "/api/attention?since=soon",
        "/api/attention?who=me",
        "/api/attention?since=%zz",
    ] {
        assert_eq!(get(&address, path).0, 400, "{path}");
    }
}

#[test]
fn a_dismissal_ends_its_asks_wait_when_it_happens() {
    let setup = Setup::new("dismissal");
    let asked = "strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-30 minutes')";
    setup.task(1, "implement", "lane-a", "2026-11-16T08:00:00Z");
    setup
        .db()
        .execute_batch(&format!(
            "INSERT INTO asks (id, task, step, kind, question, options, created)
               VALUES (1, 1, 'work', 'agent', 'Which?', '[]', {asked});
             UPDATE ask_revisions SET created = {asked};
             UPDATE asks SET withdrawn = 1, withdrawn_reason = 'dismissed' WHERE id = 1;
             UPDATE asks SET revision = 2, withdrawn = NULL, withdrawn_reason = NULL WHERE id = 1;"
        ))
        .unwrap();

    let output = setup
        .command()
        .env_remove("PRIORY_NOW")
        .args(["report", "attention", "--since", "1 day", "--json"])
        .output()
        .unwrap();

    succeeded(&output);
    let attention: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(attention["acts"], 1);
    let wait = attention["waits"]["median_minutes"].as_f64().unwrap();
    assert!((29.9..30.5).contains(&wait), "{wait}");
}

#[test]
fn the_report_shows_the_share_of_ended_tasks_that_were_no_ops_and_the_attention_on_those_that_should_have_been()
 {
    let setup = Setup::new("noop");
    for id in 1..=4 {
        setup.task(id, "implement", "lane-a", "2026-11-16T08:00:00Z");
    }
    // Task 2 asks and is answered; an intent scorer found a no-op case for it.
    setup.ask(1, 2, "2026-11-16T09:00:00Z", Some("2026-11-16T09:20:00Z"));
    let scored = json!({
        "task": "2", "kind": "intent", "verdict": "fail", "evaluated": true,
        "noop_case": "The issue is a question, not a bug.",
    });
    setup
        .db()
        .execute(
            "INSERT INTO trace (id, type, time, event) VALUES
             (lower(hex(randomblob(16))), 'factory.run.scored', '2026-11-16T13:00:00Z',
              json_object('data', json(?1)))",
            [scored.to_string()],
        )
        .unwrap();
    // Task 1 ends with nothing, task 3 ends with a commit, task 4 is still asleep.
    setup
        .db()
        .execute_batch(
            "UPDATE tasks SET trigger = 'agent-label' WHERE id IN (1, 2, 3, 4);
             INSERT INTO runs (id, task, step, tmux, session, started, ended) VALUES
               (1, 3, 'work', 't', 's', '2026-11-16T09:00:00Z', '2026-11-16T10:00:00Z');
             INSERT INTO results (run, task, step, outcome, summary, commit_sha)
               VALUES (1, 3, 'work', 'passed', 'Fixed.', 'abc1234');
             UPDATE tasks SET state = 'done' WHERE id IN (1, 2, 3);
             UPDATE task_ends SET ended = '2026-11-16T12:00:00.000Z';",
        )
        .unwrap();

    let attention = setup.attention_json(&["--since", "2026-11-16T00:00:00Z"]);
    let text = setup.attention(&["--since", "2026-11-16T00:00:00Z"]);

    assert_eq!(
        attention["noop"],
        json!([{
            "trigger": "agent-label", "ended": 3, "noop": 1, "share": 1.0 / 3.0,
            "should_have": 1, "should_have_person_minutes": 5.0,
        }])
    );
    assert!(
        text.ends_with(
            "trigger\tended\tno-op\tshare\tshould have\tperson minutes\nagent-label\t3\t1\t33%\t1\t5.0\n"
        ),
        "{text}"
    );
}
