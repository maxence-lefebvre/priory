#[allow(dead_code)]
mod harness;

mod tmux_dir;

use herdr_bus::{Bus, Message, Output as BusOutput};
use serde_json::{Value, json};
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};
use tmux_dir::TmuxDir;

const WORKFLOW: &str = r#"
start = "approve-plan"

[[steps]]
id = "approve-plan"
decides = "human"
effect = "none"
options = ["approved", "rejected", "drop"]
next = { approved = "done", rejected = "stop", drop = "stop" }
"#;

const OUTPUT: &str = "factory.intake";

/// A factory whose tasks wait on a human step, a bus and a state file under the test's own
/// directory.
struct Setup {
    dir: PathBuf,
    tmux: TmuxDir,
}

impl Setup {
    fn new(name: &str) -> Self {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join("grades")
            .join(name);
        priory_runtime::runtime::remove_sealed(&dir);
        let files = [
            ("factory/factory.toml", "name = \"demo\""),
            ("factory/workflows/plan.toml", WORKFLOW),
        ];
        for (path, content) in files {
            let file = dir.join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, content).unwrap();
        }
        let setup = Setup {
            tmux: TmuxDir::new(&dir),
            dir,
        };
        setup.bus().declare(&BusOutput::new(OUTPUT)).unwrap();
        setup
    }

    fn bus(&self) -> Bus {
        Bus::open(&self.dir.join("bus.db")).unwrap()
    }

    fn factory(&self) -> String {
        self.dir.join("factory").to_str().unwrap().to_owned()
    }

    fn command(&self) -> Command {
        let mut command = harness::priory();
        command
            .env_remove("TMUX")
            .env("TMUX_TMPDIR", self.tmux.path())
            .env("HERDR_BUS_DB", self.dir.join("bus.db"))
            .env("PRIORY_DB", self.dir.join("priory.db"))
            .env("PRIORY_HOME", self.dir.join("home"));
        command
    }

    fn priory(&self, args: &[&str]) -> Output {
        self.command().args(args).output().unwrap()
    }

    /// Publishes `messages` and lets one daemon pass handle them.
    fn handle(&self, messages: &[Message]) {
        for message in messages {
            self.bus().publish(OUTPUT, message).unwrap();
        }
        succeeded(&self.priory(&["daemon", &self.factory(), "--once"]));
    }

    fn db(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(self.dir.join("priory.db")).unwrap()
    }

    /// Task `task`, graded `grade`, whose worker pushed `priory/<task>-fix` to `owner/repo`.
    fn task(&self, task: u64, grade: &str) {
        let key = format!("cli:task-{task}");
        self.handle(&[Message::new(
            "factory.task.requested",
            json!({ "workflow": "plan", "key": key, "band": null, "lane": null,
                    "trigger": "cli", "source": "source" }),
        )
        .subject(key)]);
        self.db()
            .execute(
                "UPDATE tasks SET grade = ?2, changed = coalesce(changed, '2026-11-20T09:00:00Z') WHERE id = ?1;",
                rusqlite::params![task as i64, grade],
            )
            .unwrap();
        self.db()
            .execute(
                "INSERT INTO owns (task, resource) VALUES (?1, 'github:owner/repo@' || ?2)",
                rusqlite::params![task as i64, branch(task)],
            )
            .unwrap();
    }

    fn grades(&self, args: &[&str]) -> String {
        let output = self.priory(&[&["grades"][..], args].concat());
        succeeded(&output);
        String::from_utf8(output.stdout).unwrap()
    }

    fn grades_json(&self) -> Value {
        serde_json::from_str(&self.grades(&["--json"])).unwrap()
    }
}

fn succeeded(output: &Output) {
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn branch(task: u64) -> String {
    format!("priory/{task}-fix")
}

/// An event of pull request `number`, whose head is task `task`'s branch at `head`.
fn on_pull(kind: &str, number: u64, task: u64, head: &str, data: Value) -> Message {
    let mut fields = json!({
        "repo": "owner/repo", "pull_request": number, "head_branch": branch(task),
        "head_sha": head, "base_branch": "main", "open": kind != "github.pull_request.closed",
        "url": format!("https://github.com/owner/repo/pull/{number}"),
        "baseline": false, "triggers": [],
    });
    for (field, value) in data.as_object().unwrap() {
        fields[field] = value.clone();
    }
    Message::new(kind, fields).subject(format!("owner/repo#{number}"))
}

/// A review written on `head`, when the pull request's head was still `head`.
fn review(number: u64, task: u64, head: &str, state: &str, id: u64) -> Message {
    on_pull(
        "github.pull_request_review.submitted",
        number,
        task,
        head,
        json!({ "state": state, "actor": "max", "github_id": id, "body": "", "commit": head }),
    )
}

fn merged(number: u64, task: u64, head: &str, merge_commit: &str) -> Message {
    on_pull(
        "github.pull_request.closed",
        number,
        task,
        head,
        json!({ "merged": true, "merge_commit": merge_commit, "actor": "max",
                "title": format!("Fix {task}"), "body": "", "github_id": 900 + number }),
    )
}

#[test]
fn a_green_task_merged_with_no_requested_change_counts_as_one_shot() {
    let setup = Setup::new("one-shot");
    setup.task(1, "green");

    setup.handle(&[
        review(10, 1, "aaa", "commented", 1),
        merged(10, 1, "aaa", "m10"),
    ]);

    let grades = setup.grades_json();
    assert_eq!(
        grades["one_shot"],
        json!({ "rate": 1.0, "clean": 1, "outcomes": 1, "target": 0.9 })
    );
}

/// The labels `grades --json` counts for `grade`, as (label, count) pairs with a count.
fn labels(grades: &Value, grade: &str) -> Vec<(String, i64)> {
    let row = grades["grades"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["grade"] == grade)
        .unwrap();
    row["labels"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(label, counted)| (label.clone(), counted["count"].as_i64().unwrap()))
        .filter(|(_, count)| *count > 0)
        .collect()
}

#[test]
fn a_requested_change_or_a_push_after_review_is_not_one_shot() {
    let setup = Setup::new("not-one-shot");
    setup.task(1, "green");
    setup.task(2, "green");
    setup.task(3, "green");
    setup.task(4, "green");
    setup.task(5, "green");
    setup.task(6, "green");

    setup.handle(&[
        review(10, 1, "aaa", "changes_requested", 1),
        merged(10, 1, "aaa", "m10"),
        review(20, 2, "bbb", "commented", 2),
        merged(20, 2, "ccc", "m20"),
        review(30, 3, "ddd", "approved", 3),
        merged(30, 3, "ddd", "m30"),
        // A poll reads a review after the push, so it names the new head beside the commit reviewed.
        on_pull(
            "github.pull_request_review.submitted",
            40,
            4,
            "ggg",
            json!({ "state": "commented", "actor": "max", "github_id": 4, "body": "",
                    "commit": "fff" }),
        ),
        merged(40, 4, "ggg", "m40"),
        // A review that arrives after the merge still requests a change.
        merged(50, 5, "hhh", "m50"),
        review(50, 5, "hhh", "changes_requested", 5),
        // A comment on an older commit that arrives after the merge still came before a push.
        merged(60, 6, "jjj", "m60"),
        review(60, 6, "iii", "commented", 6),
    ]);

    let grades = setup.grades_json();
    assert_eq!(
        grades["one_shot"],
        json!({ "rate": 1.0 / 6.0, "clean": 1, "outcomes": 6, "target": 0.9 })
    );
    assert_eq!(
        labels(&grades, "green"),
        [
            ("approved-clean".to_owned(), 1),
            ("changes-requested".to_owned(), 5)
        ]
    );
}

#[test]
fn a_rejected_approval_and_a_close_without_merge_get_their_labels() {
    let setup = Setup::new("rejected");
    setup.task(1, "red");
    setup.task(2, "green");
    setup.task(3, "red");
    setup.handle(&[on_pull(
        "github.pull_request.closed",
        20,
        2,
        "bbb",
        json!({ "merged": false, "actor": "max", "title": "Fix 2", "body": "", "github_id": 920 }),
    )]);

    let mut daemon = setup
        .command()
        .args(["daemon", &setup.factory()])
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let answered = loop {
        let output = setup.priory(&["answer", "--revision", "1", "1", "rejected"]);
        let stderr = String::from_utf8_lossy(&output.stderr);
        // The daemon opens the ask on its first tick.
        let early = stderr.contains("no priory daemon listens") || stderr.contains("no ask 1");
        if !early || Instant::now() > deadline {
            break output;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    succeeded(&answered);
    // A gate whose answer stops the task rejects too, whatever its option is called.
    succeeded(&setup.priory(&["answer", "--revision", "1", "3", "drop"]));
    while setup.grades_json()["grades"][0]["outcomes"] != 2 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = daemon.kill();
    let _ = daemon.wait();

    let grades = setup.grades_json();
    assert_eq!(
        labels(&grades, "red"),
        [("approval-rejected".to_owned(), 2)]
    );
    assert_eq!(
        labels(&grades, "green"),
        [("closed-unmerged".to_owned(), 1)]
    );
    assert_eq!(grades["one_shot"]["rate"], 0.0);
}

#[test]
fn a_revert_pr_labels_the_original_and_a_failing_check_at_its_merge_commit_too() {
    let setup = Setup::new("reverted");
    for task in 1..=5 {
        setup.task(task, "green");
    }
    let revert = |number: u64, title: &str, body: &str| {
        on_pull(
            "github.pull_request.closed",
            number,
            9,
            "fff",
            json!({ "merged": true, "merge_commit": format!("m{number}"), "actor": "max",
                    "title": title, "body": body, "github_id": 900 + number }),
        )
    };

    setup.handle(&[
        merged(1, 1, "aaa", "1111111111111111111111111111111111111111"),
        merged(2, 2, "bbb", "m2"),
        merged(3, 3, "ccc", "m3"),
        merged(25, 4, "ddd", "m25"),
        merged(5, 5, "eee", "2222222222222222222222222222222222222222"),
        revert(40, "Revert \"Fix 2\"", "Reverts owner/repo#2"),
        revert(
            41,
            "Undo the first fix",
            "This reverts commit 1111111111111111111111111111111111111111.",
        ),
        revert(
            42,
            "Export layers",
            "This reverts commit 1111111111111111111111111111111111111111. Builds on commit 2222222222222222222222222222222222222222.",
        ),
        on_pull(
            "github.check_run.completed",
            3,
            3,
            "m3",
            json!({ "conclusion": "failure", "name": "ci", "github_id": 77,
                    "pending": 0, "failing": 1 }),
        ),
    ]);

    let grades = setup.grades_json();
    assert_eq!(
        labels(&grades, "green"),
        [
            ("approved-clean".to_owned(), 5),
            ("red-after-merge".to_owned(), 1),
            ("reverted".to_owned(), 2)
        ],
        "#25 is not #2, and building on a commit does not revert it"
    );
}

const NOW: &str = "2026-11-21T12:00:00Z";

impl Setup {
    /// Makes `owner/repo` polled and installs a `gh` that logs its calls to `gh.log`, answers the
    /// check runs of commit `sha` from `checks-<sha>.json` (`heads/main` from `checks-heads-main.json`),
    /// or a 404 when `404-<sha>` exists, a compare from `compare.json`, and reads `[]` elsewhere.
    fn polled_repo(&self) {
        let trigger = "source = \"github\"\nrepo = \"owner/repo\"\non = \"issues.labeled\"\n\
                       filter = { label = \"agent\" }\nworkflow = \"plan\"\nkey = \"issue.number\"\nband = \"today\"\n";
        let file = self.dir.join("factory/triggers/agent.toml");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, trigger).unwrap();
        let script = format!(
            "#!/bin/sh\necho \"$*\" >> '{dir}/gh.log'\ncase \"$2\" in\n  repos/owner/repo/commits/*)\n    \
             sha=${{2#repos/owner/repo/commits/}}\n    sha=$(echo \"${{sha%%/check-runs*}}\" | tr / -)\n    \
             [ -f \"{dir}/404-$sha\" ] && {{ echo 'gh: Not Found (HTTP 404)' >&2; exit 1; }}\n    \
             cat \"{dir}/checks-$sha.json\" 2>/dev/null || \
             echo '{{\"total_count\":0,\"check_runs\":[]}}' ;;\n  repos/owner/repo/compare/*)\n    \
             cat \"{dir}/compare.json\" 2>/dev/null || echo '{{\"status\":\"diverged\"}}' ;;\n  \
             *) echo '[]' ;;\nesac\n",
            dir = self.dir.display()
        );
        let gh = self.dir.join("bin/gh");
        fs::create_dir_all(gh.parent().unwrap()).unwrap();
        fs::write(&gh, script).unwrap();
        fs::set_permissions(&gh, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    }

    /// Makes the default branch's check runs at `sha` those of `runs`.
    fn checks_at(&self, sha: &str, runs: &[(u64, &str, &str)]) {
        let runs: Vec<Value> = runs
            .iter()
            .map(|(id, status, conclusion)| {
                json!({ "id": id, "name": "ci", "status": status, "conclusion": conclusion,
                        "completed_at": "2026-11-20T10:05:00Z",
                        "html_url": format!("https://github.com/owner/repo/runs/{id}"),
                        "app": { "slug": "github-actions" } })
            })
            .collect();
        let answer = json!({ "total_count": runs.len(), "check_runs": runs });
        fs::write(
            self.dir.join(format!("checks-{sha}.json")),
            answer.to_string(),
        )
        .unwrap();
    }

    /// Makes the run of `name` on the tip `sha` of `main` one with `status` and `conclusion`, and
    /// the compare of a merge commit with it say `compare`.
    fn tip_run(&self, name: &str, sha: &str, status: &str, conclusion: &str, compare: &str) {
        let run = json!({ "id": 500, "name": name, "head_sha": sha, "status": status,
                          "conclusion": conclusion, "completed_at": "2026-11-20T10:30:00Z",
                          "html_url": "https://github.com/owner/repo/runs/500" });
        let answer = json!({ "total_count": 1, "check_runs": [run] });
        fs::write(self.dir.join("checks-heads-main.json"), answer.to_string()).unwrap();
        let compare = json!({ "status": compare });
        fs::write(self.dir.join("compare.json"), compare.to_string()).unwrap();
    }

    /// The `landed wrong` lines on the desk.
    fn alarms(&self) -> Vec<String> {
        self.desk()["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|item| item["kind"] == "report")
            .map(|item| item["what"].as_str().unwrap().to_owned())
            .collect()
    }

    /// One intake poll at `NOW`, then the daemon reading what it published.
    fn poll(&self) {
        self.poll_at(NOW);
    }

    /// One intake poll at `now`, then the daemon reading what it published.
    fn poll_at(&self, now: &str) {
        let path = format!(
            "{}:{}",
            self.dir.join("bin").display(),
            std::env::var("PATH").unwrap()
        );
        let output = self
            .command()
            .args(["intake", "--once", &self.factory()])
            .env("PATH", path)
            .env("PRIORY_NOW", now)
            .output()
            .unwrap();
        succeeded(&output);
        succeeded(&self.priory(&["daemon", &self.factory(), "--once"]));
    }

    /// How many times the intake read the check runs at `sha`.
    fn reads_of(&self, sha: &str) -> usize {
        let log = fs::read_to_string(self.dir.join("gh.log")).unwrap_or_default();
        let path = format!("repos/owner/repo/commits/{sha}/check-runs");
        log.lines().filter(|line| line.contains(&path)).count()
    }

    fn red_after_merge(&self) -> i64 {
        let grades = self.grades_json();
        labels(&grades, "green")
            .into_iter()
            .find(|(label, _)| label == "red-after-merge")
            .map_or(0, |(_, count)| count)
    }
}

/// `merged`, at `time`.
fn merged_on(number: u64, task: u64, head: &str, merge_commit: &str, time: &str) -> Message {
    merged(number, task, head, merge_commit).time(time.parse().unwrap())
}

#[test]
fn a_failing_check_on_the_merge_commit_labels_the_task_red_after_merge() {
    let setup = Setup::new("red-after-merge");
    setup.polled_repo();
    setup.task(1, "green");
    setup.handle(&[merged_on(10, 1, "aaa", "m10", "2026-11-20T10:00:00Z")]);
    setup.checks_at(
        "m10",
        &[(1, "completed", "success"), (2, "completed", "failure")],
    );

    setup.poll();

    assert_eq!(setup.red_after_merge(), 1);
}

#[test]
fn a_green_check_on_the_merge_commit_does_not_label_the_task() {
    let setup = Setup::new("green-after-merge");
    setup.polled_repo();
    setup.task(1, "green");
    setup.handle(&[merged_on(10, 1, "aaa", "m10", "2026-11-20T10:00:00Z")]);
    setup.checks_at("m10", &[(1, "completed", "success")]);

    setup.poll();

    assert_eq!(setup.reads_of("m10"), 1);
    assert_eq!(setup.red_after_merge(), 0);
}

#[test]
fn a_merge_commit_is_read_until_every_check_completed_then_not_again() {
    let setup = Setup::new("stops-when-complete");
    setup.polled_repo();
    setup.task(1, "green");
    setup.handle(&[merged_on(10, 1, "aaa", "m10", "2026-11-20T10:00:00Z")]);
    setup.checks_at(
        "m10",
        &[(1, "completed", "success"), (2, "in_progress", "")],
    );
    setup.poll();
    setup.poll();
    assert_eq!(
        setup.reads_of("m10"),
        2,
        "a running check keeps the commit polled"
    );

    setup.checks_at(
        "m10",
        &[(1, "completed", "success"), (2, "completed", "failure")],
    );
    setup.poll();
    setup.poll();

    assert_eq!(
        setup.reads_of("m10"),
        3,
        "the poll that saw them all complete was the last"
    );
    assert_eq!(setup.red_after_merge(), 1);
}

/// A merge of task 1 at `m10` whose `ci` run was cancelled.
fn cancelled_at_merge(name: &str) -> Setup {
    let setup = Setup::new(name);
    setup.polled_repo();
    setup.task(1, "green");
    setup.handle(&[merged_on(10, 1, "aaa", "m10", "2026-11-20T10:00:00Z")]);
    setup.checks_at("m10", &[(1, "completed", "cancelled")]);
    setup
}

#[test]
fn a_cancelled_run_that_a_later_run_on_a_descendant_passed_is_not_red() {
    let setup = cancelled_at_merge("cancelled-superseded");
    setup.tip_run("ci", "m11", "completed", "success", "ahead");

    setup.poll();
    setup.poll();

    assert_eq!(setup.red_after_merge(), 0);
    assert_eq!(setup.alarms(), Vec::<String>::new());
    assert_eq!(setup.reads_of("m10"), 1, "the merge commit is settled");
}

#[test]
fn a_cancelled_run_waits_for_the_later_run_then_counts_as_it_did() {
    let setup = cancelled_at_merge("cancelled-waits");
    setup.tip_run("ci", "m11", "in_progress", "", "ahead");
    setup.poll();
    assert_eq!(setup.red_after_merge(), 0);
    assert_eq!(setup.alarms(), Vec::<String>::new());

    setup.tip_run("ci", "m11", "completed", "failure", "ahead");
    setup.poll();

    assert_eq!(
        setup.reads_of("m10"),
        2,
        "a waiting run keeps the commit polled"
    );
    assert_eq!(setup.red_after_merge(), 1);
    assert_eq!(
        setup.alarms(),
        [
            "priory: landed wrong: owner/repo#10 of task 1 is red on main: ci cancelled https://github.com/owner/repo/runs/1"
        ]
    );
}

#[test]
fn a_cancelled_run_whose_base_branch_is_gone_is_red_and_the_poll_goes_on() {
    let setup = cancelled_at_merge("cancelled-base-gone");
    fs::write(setup.dir.join("404-heads-main"), "").unwrap();

    setup.poll();

    assert_eq!(setup.red_after_merge(), 1);
    assert_eq!(setup.alarms().len(), 1);
}

#[test]
fn a_cancelled_run_waits_out_the_grace_for_a_run_on_the_tip_then_is_red() {
    let setup = Setup::new("cancelled-no-tip-run");
    setup.polled_repo();
    setup.task(1, "green");
    setup.handle(&[merged_on(10, 1, "aaa", "m10", "2026-11-21T11:58:00Z")]);
    setup.checks_at("m10", &[(1, "completed", "cancelled")]);

    setup.poll();
    assert_eq!(setup.red_after_merge(), 0);
    assert_eq!(setup.alarms(), Vec::<String>::new());

    setup.poll_at("2026-11-21T12:04:00Z");

    assert_eq!(setup.red_after_merge(), 1);
    assert_eq!(setup.alarms().len(), 1);
}

#[test]
fn a_cancelled_run_with_no_later_run_on_a_descendant_is_red() {
    let setup = cancelled_at_merge("cancelled-diverged");
    setup.tip_run("ci", "m11", "completed", "success", "diverged");

    setup.poll();

    assert_eq!(setup.red_after_merge(), 1);
    assert_eq!(setup.alarms().len(), 1);
}

#[test]
fn a_merge_older_than_seven_days_is_not_read() {
    let setup = Setup::new("merge-too-old");
    setup.polled_repo();
    setup.task(1, "green");
    setup.task(2, "green");
    setup.handle(&[
        merged_on(10, 1, "aaa", "m10", "2026-11-10T10:00:00Z"),
        merged_on(11, 2, "bbb", "m11", "2026-11-20T10:00:00Z"),
    ]);
    setup.checks_at("m10", &[(1, "completed", "failure")]);
    setup.checks_at("m11", &[(2, "completed", "success")]);

    setup.poll();

    assert_eq!(setup.reads_of("m11"), 1);
    assert_eq!(setup.reads_of("m10"), 0);
    assert_eq!(setup.red_after_merge(), 0);
}

#[test]
fn a_merge_commit_whose_failing_event_was_not_stored_is_read_again() {
    let setup = Setup::new("partial-batch");
    setup.polled_repo();
    setup.task(1, "green");
    setup.handle(&[
        merged_on(10, 1, "aaa", "m10", "2026-11-20T10:00:00Z"),
        on_pull(
            "github.check_run.completed",
            10,
            1,
            "m10",
            json!({ "conclusion": "success", "name": "ci", "github_id": 1, "pending": 0, "failing": 1 }),
        ),
    ]);
    setup.checks_at(
        "m10",
        &[(1, "completed", "success"), (2, "completed", "failure")],
    );

    setup.poll();

    assert_eq!(setup.red_after_merge(), 1);
}

const TOKEN: &str = "s3cret-token";

/// `GET <path>` on the server at `address`, as its status and body.
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
fn the_headline_is_greens_one_shot_rate_and_the_grades_endpoint_answers_grades_json() {
    let setup = Setup::new("endpoint");
    fs::write(
        setup.dir.join("factory/factory.toml"),
        "name = \"demo\"\n\n[serve]\nlisten = \"127.0.0.1:0\"\ntoken_env = \"TEST_TOKEN\"\n",
    )
    .unwrap();
    setup.task(1, "green");
    setup.task(2, "green");
    setup.task(3, "yellow");
    setup.handle(&[
        merged(10, 1, "aaa", "m10"),
        review(20, 2, "bbb", "changes_requested", 2),
        merged(30, 3, "ccc", "m30"),
    ]);

    let text = setup.grades(&[]);
    assert_eq!(
        text.lines().take(5).collect::<Vec<_>>(),
        [
            "green one-shot: 50.0% (1 of 2), target 90%",
            "",
            "grade\ttasks\toutcomes\tapproved-clean\tchanges-requested\tapproval-rejected\tclosed-unmerged\tred-after-merge\treverted",
            "red\t0\t0\t0 (-)\t0 (-)\t0 (-)\t0 (-)\t0 (-)\t0 (-)",
            "yellow\t1\t1\t1 (100.0%)\t0 (0.0%)\t0 (0.0%)\t0 (0.0%)\t0 (0.0%)\t0 (0.0%)",
        ]
    );

    let mut server = setup
        .command()
        .env("TEST_TOKEN", TOKEN)
        .args(["serve", &setup.factory()])
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stderr = BufReader::new(server.stderr.take().unwrap());
    let address = loop {
        let mut line = String::new();
        assert!(stderr.read_line(&mut line).unwrap() > 0, "serve exited");
        if let Some(address) = line.trim().strip_prefix("priory: serving on http://") {
            break address.to_owned();
        }
    };
    let served = [
        get(&address, "/api/grades"),
        get(&address, "/api/grades?since=2000-01-01T00%3A00%3A00Z"),
        get(&address, "/api/grades?since=never"),
    ];
    let _ = server.kill();
    let _ = server.wait();

    let json =
        |(status, body): &(u16, String)| (*status, serde_json::from_str::<Value>(body).unwrap());
    assert_eq!(json(&served[0]), (200, setup.grades_json()));
    assert_eq!(
        json(&served[1]),
        (
            200,
            serde_json::from_str(&setup.grades(&["--since", "2000-01-01T00:00:00Z", "--json"]))
                .unwrap()
        )
    );
    assert_eq!(served[2].0, 400);
}

#[test]
fn a_stack_is_one_shot_once_every_pull_request_merged_with_no_push_after_review() {
    let setup = Setup::new("stack");
    setup.task(1, "green");
    setup.task(2, "green");
    for (task, number) in [(1, 10), (1, 11), (2, 20), (2, 21)] {
        setup
            .db()
            .execute(
                "INSERT INTO pull_requests (task, resource, branch, base)
                 VALUES (?1, 'github:owner/repo#' || ?2, 'priory/' || ?2, 'main')",
                [task, number],
            )
            .unwrap();
    }

    setup.handle(&[
        merged(10, 1, "aaa", "m10"),
        review(20, 2, "bbb", "commented", 1),
        on_pull(
            "github.pull_request.check_runs",
            20,
            2,
            "ccc",
            json!({ "runs": 1, "updated_at": "2026-11-17T10:00:00Z" }),
        ),
        merged(20, 2, "ccc", "m20"),
    ]);
    assert_eq!(
        setup.grades_json()["one_shot"]["outcomes"],
        0,
        "#11 and #21 are open"
    );

    setup.handle(&[merged(11, 1, "ddd", "m11"), merged(21, 2, "eee", "m21")]);

    let grades = setup.grades_json();
    assert_eq!(
        labels(&grades, "green"),
        [
            ("approved-clean".to_owned(), 1),
            ("changes-requested".to_owned(), 1)
        ]
    );
}

#[test]
fn each_review_that_requests_changes_is_its_own_record_with_when_it_happened() {
    let setup = Setup::new("per-event");
    setup.task(1, "green");
    let on = |message: Message, time: &str| message.time(time.parse().unwrap());

    setup.handle(&[
        on(
            review(10, 1, "aaa", "changes_requested", 1),
            "2026-10-01T10:00:00Z",
        ),
        on(
            review(10, 1, "bbb", "changes_requested", 2),
            "2026-10-05T10:00:00Z",
        ),
        merged_on(10, 1, "bbb", "m10", "2026-10-06T10:00:00Z"),
    ]);
    // The same event stored twice is still one source event.
    setup.handle(&[on(
        review(10, 1, "bbb", "changes_requested", 2),
        "2026-10-05T10:00:00Z",
    )]);

    let happened: Vec<(String, String)> = setup
        .db()
        .prepare(
            "SELECT json_extract(event, '$.data.label'), json_extract(event, '$.data.happened')
             FROM trace WHERE type = 'factory.step.labeled' ORDER BY seq",
        )
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        happened,
        [
            (
                "changes-requested".to_owned(),
                "2026-10-01T10:00:00Z".to_owned()
            ),
            (
                "changes-requested".to_owned(),
                "2026-10-05T10:00:00Z".to_owned()
            ),
        ]
    );
    assert_eq!(
        labels(&setup.grades_json(), "green"),
        [("changes-requested".to_owned(), 1)],
        "grades counts a task once per label"
    );
}

impl Setup {
    /// The desk as `priory desk --json` prints it.
    fn desk(&self) -> Value {
        let output = self.priory(&["desk", "--json"]);
        succeeded(&output);
        serde_json::from_slice(&output.stdout).unwrap()
    }
}

#[test]
fn a_red_merge_commit_and_a_revert_each_raise_one_alarm_on_the_desk() {
    let setup = Setup::new("landed-wrong-alarm");
    setup.task(1, "green");
    setup.task(2, "green");
    let failing = |id: u64| {
        on_pull(
            "github.check_run.completed",
            10,
            1,
            "m10",
            json!({ "conclusion": "failure", "name": format!("test {id}"), "github_id": id,
                    "url": format!("https://github.com/owner/repo/runs/{id}"),
                    "pending": 0, "failing": 2 }),
        )
    };
    setup.handle(&[
        merged(10, 1, "aaa", "m10"),
        merged(20, 2, "bbb", "m20"),
        failing(71),
        failing(72),
        on_pull(
            "github.pull_request.closed",
            40,
            9,
            "fff",
            json!({ "merged": true, "merge_commit": "m40", "actor": "max",
                    "title": "Revert \"Fix 2\"", "body": "Reverts owner/repo#20", "github_id": 940 }),
        ),
    ]);

    let desk = setup.desk();
    let alarms: Vec<&str> = desk["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["kind"] == "report")
        .map(|item| item["what"].as_str().unwrap())
        .collect();
    assert_eq!(desk["next"]["title"], "a merge landed wrong", "{desk}");
    assert!(
        desk["next"]["why"]
            .as_str()
            .unwrap()
            .starts_with("work stopped"),
        "{desk}"
    );
    assert_eq!(
        alarms,
        [
            "priory: landed wrong: owner/repo#10 of task 1 is red on main: test 71 failure https://github.com/owner/repo/runs/71",
            "priory: landed wrong: owner/repo#20 of task 2 was reverted by #40: https://github.com/owner/repo/pull/40",
        ],
        "one alarm per merge commit, however many checks fail there"
    );
}

#[test]
fn a_check_run_nobody_hears_shows_its_name_and_conclusion_as_its_unrouted_line() {
    let setup = Setup::new("unrouted-check-line");
    setup.task(1, "green");
    setup.handle(&[merged(10, 1, "aaa", "m10")]);
    setup
        .db()
        .execute_batch(
            "UPDATE tasks SET state = 'done' WHERE id = 1;
             INSERT INTO owns (task, resource) VALUES (1, 'github:owner/repo#10');",
        )
        .unwrap();
    setup.handle(&[on_pull(
        "github.check_run.completed",
        10,
        1,
        "m10",
        json!({ "conclusion": "failure", "name": "test (ubuntu)", "github_id": 71,
                "url": "https://github.com/owner/repo/runs/71", "pending": 0, "failing": 1 }),
    )]);

    let output = setup.priory(&["inbox", "--unrouted", "--json"]);
    succeeded(&output);
    let unrouted: Value = serde_json::from_slice(&output.stdout).unwrap();
    let lines: Vec<&Value> = unrouted["unrouted"]["owned"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|group| group["events"].as_array().unwrap())
        .filter(|event| event["type"] == "github.check_run.completed")
        .map(|event| &event["line"])
        .collect();
    assert_eq!(lines, [&json!("test (ubuntu) failure")], "{unrouted}");
}

#[test]
fn by_lane_splits_each_grade_by_the_tasks_lane() {
    let setup = Setup::new("by-lane");
    setup.task(1, "green");
    setup.task(2, "green");
    setup.task(3, "green");
    setup
        .db()
        .execute("UPDATE tasks SET lane = 'web' WHERE id IN (1, 2)", [])
        .unwrap();
    setup.handle(&[
        merged(10, 1, "aaa", "m10"),
        review(20, 2, "bbb", "changes_requested", 2),
        merged(30, 3, "ccc", "m30"),
    ]);

    let grades: Value = serde_json::from_str(&setup.grades(&["--by", "lane", "--json"])).unwrap();
    assert_eq!(grades["by"], "lane");
    let green = |lane: &str| {
        grades["grades"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["grade"] == "green" && row["lane"] == lane)
            .unwrap()
            .clone()
    };
    assert_eq!(green("web")["tasks"], 2);
    assert_eq!(green("web")["labels"]["changes-requested"]["count"], 1);
    assert_eq!(green("none/misc")["tasks"], 1);
    assert_eq!(green("none/misc")["labels"]["approved-clean"]["count"], 1);
    assert_eq!(
        grades["one_shot"],
        json!({ "rate": 2.0 / 3.0, "clean": 2, "outcomes": 3, "target": 0.9 })
    );

    let text = setup.grades(&["--by", "lane"]);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[0], "green one-shot: 66.7% (2 of 3), target 90%");
    assert!(
        lines[2].starts_with("lane\tgrade\ttasks\toutcomes\t"),
        "{text}"
    );
    assert!(
        lines.contains(
            &"web\tgreen\t2\t2\t1 (50.0%)\t1 (50.0%)\t0 (0.0%)\t0 (0.0%)\t0 (0.0%)\t0 (0.0%)"
        ),
        "{text}"
    );
    assert!(
        lines.contains(
            &"none/misc\tgreen\t1\t1\t1 (100.0%)\t0 (0.0%)\t0 (0.0%)\t0 (0.0%)\t0 (0.0%)\t0 (0.0%)"
        ),
        "{text}"
    );
    assert_eq!(setup.grades_json()["by"], Value::Null);
}
