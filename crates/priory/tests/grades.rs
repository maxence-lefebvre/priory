#[allow(dead_code)]
mod harness;

#[allow(dead_code)]
mod conform;
mod tmux_dir;

use herdr_bus::{Bus, Message, MessageQuery, Output as BusOutput};
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
    /// or a 404, 422 or 403 when `404-<sha>`, `422-<sha>` or `403-<sha>` exists, a compare from `compare.json`,
    /// workflow 1 for an Actions run unless `workflow-<run>.json` exists, or the 403 of a token without the
    /// permission when `403-run-<run>` exists, and reads `[]` elsewhere.
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
             [ -f \"{dir}/422-$sha\" ] && {{ echo 'gh: No commit found (HTTP 422)' >&2; exit 1; }}\n    \
             [ -f \"{dir}/403-$sha\" ] && {{ echo 'gh: Forbidden (HTTP 403)' >&2; exit 1; }}\n    \
             cat \"{dir}/checks-$sha.json\" 2>/dev/null || \
             echo '{{\"total_count\":0,\"check_runs\":[]}}' ;;\n  repos/owner/repo/compare/*)\n    \
             cat \"{dir}/compare.json\" 2>/dev/null || echo '{{\"status\":\"diverged\"}}' ;;\n  \
             repos/owner/repo/actions/runs/*)\n    \
             [ -f \"{dir}/403-run-${{2##*/}}\" ] && {{ echo 'gh: Resource not accessible by personal access token (HTTP 403)' >&2; exit 1; }}\n    \
             cat \"{dir}/workflow-${{2##*/}}.json\" 2>/dev/null || \
             echo '{{\"workflow_id\":1}}' ;;\n  \
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
        self.checks_completed_at(sha, runs, "2026-11-20T10:05:00Z");
    }

    /// Makes the check runs at `sha` those of `runs`, each completed at `completed`.
    fn checks_completed_at(&self, sha: &str, runs: &[(u64, &str, &str)], completed: &str) {
        let runs: Vec<Value> = runs
            .iter()
            .map(|(id, status, conclusion)| {
                json!({ "id": id, "name": "ci", "status": status, "conclusion": conclusion,
                        "completed_at": completed,
                        "html_url": format!("https://github.com/owner/repo/actions/runs/{id}/job/{id}"),
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
                          "html_url": "https://github.com/owner/repo/actions/runs/500/job/500",
                          "app": { "slug": "github-actions" } });
        let answer = json!({ "total_count": 1, "check_runs": [run] });
        fs::write(self.dir.join("checks-heads-main.json"), answer.to_string()).unwrap();
        let compare = json!({ "status": compare });
        fs::write(self.dir.join("compare.json"), compare.to_string()).unwrap();
    }

    /// The data of every `github.check_run.completed` the intake published, each checked against
    /// its schema.
    fn completed_checks(&self) -> Vec<Value> {
        let query = MessageQuery {
            output: Some(OUTPUT.into()),
            limit: 1000,
            ..MessageQuery::default()
        };
        self.bus()
            .messages(&query)
            .unwrap()
            .into_iter()
            .map(|stored| serde_json::to_value(&stored.message).unwrap())
            .filter(|message| message["type"] == "github.check_run.completed")
            .inspect(conform::conforms)
            .map(|message| message["data"].clone())
            .collect()
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
        succeeded(&self.intake(now));
        succeeded(&self.priory(&["daemon", &self.factory(), "--once"]));
    }

    /// One intake poll at `now`.
    fn intake(&self, now: &str) -> Output {
        self.intake_command(now)
            .args(["intake", "--once", &self.factory()])
            .output()
            .unwrap()
    }

    /// A `priory` at `now` that finds the stub `gh`.
    fn intake_command(&self, now: &str) -> Command {
        let path = format!(
            "{}:{}",
            self.dir.join("bin").display(),
            std::env::var("PATH").unwrap()
        );
        let mut command = self.command();
        command.env("PATH", path).env("PRIORY_NOW", now);
        command
    }

    /// Makes the Actions run `run` one of `workflow`.
    fn workflow(&self, run: u64, workflow: u64) {
        fs::write(
            self.dir.join(format!("workflow-{run}.json")),
            json!({ "workflow_id": workflow }).to_string(),
        )
        .unwrap();
    }

    /// The error of the last GitHub poll.
    fn github_error(&self) -> Option<String> {
        self.db()
            .query_row(
                "SELECT error FROM intake_sources WHERE source = 'github'",
                [],
                |row| row.get(0),
            )
            .unwrap()
    }

    /// How many `gh` calls named `path`.
    fn calls(&self, path: &str) -> usize {
        let log = fs::read_to_string(self.dir.join("gh.log")).unwrap_or_default();
        log.lines().filter(|line| line.contains(path)).count()
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
    let checks = setup.completed_checks();
    assert_eq!(checks.len(), 1);
    assert_eq!(checks[0]["conclusion"], "cancelled");
    assert_eq!(checks[0]["superseded_by"], "m11");
    assert_eq!(checks[0]["failing"], 0);
}

#[test]
fn a_cancelled_run_reads_the_tip_of_a_base_branch_with_a_slash() {
    let setup = Setup::new("cancelled-release-branch");
    setup.polled_repo();
    setup.task(1, "green");
    let merge = on_pull(
        "github.pull_request.closed",
        10,
        1,
        "aaa",
        json!({ "merged": true, "merge_commit": "m10", "actor": "max", "title": "Fix 1",
                "body": "", "github_id": 910, "base_branch": "release/x" }),
    )
    .time("2026-11-20T10:00:00Z".parse().unwrap());
    setup.handle(&[merge]);
    setup.checks_at("m10", &[(1, "completed", "cancelled")]);
    setup.tip_run("ci", "m11", "completed", "success", "ahead");
    // The stub turns each `/` of the path into `-`, so only the encoded branch finds the tip.
    fs::rename(
        setup.dir.join("checks-heads-main.json"),
        setup.dir.join("checks-heads-release%2Fx.json"),
    )
    .unwrap();

    setup.poll();

    assert_eq!(setup.red_after_merge(), 0);
    assert_eq!(setup.completed_checks()[0]["superseded_by"], "m11");
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
            "priory: landed wrong: owner/repo#10 of task 1 is red on main: ci cancelled https://github.com/owner/repo/actions/runs/1/job/1"
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
fn a_cancelled_run_waits_out_the_grace_from_its_cancel_for_a_run_on_the_tip_then_is_red() {
    let setup = Setup::new("cancelled-no-tip-run");
    setup.polled_repo();
    setup.task(1, "green");
    // The deploy ran longer than the grace, then a newer merge cancelled it.
    setup.handle(&[merged_on(10, 1, "aaa", "m10", "2026-11-21T11:00:00Z")]);
    setup.checks_completed_at(
        "m10",
        &[(1, "completed", "cancelled")],
        "2026-11-21T11:58:00Z",
    );

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

/// A merge of task 1 at `m10` whose `ci` run failed, with its alarm on the desk.
fn red_at_merge(name: &str) -> Setup {
    let setup = Setup::new(name);
    setup.polled_repo();
    setup.task(1, "green");
    setup.handle(&[merged_on(10, 1, "aaa", "m10", "2026-11-20T10:00:00Z")]);
    setup.checks_at("m10", &[(1, "completed", "failure")]);
    setup.poll();
    assert_eq!(setup.alarms().len(), 1);
    setup
}

#[test]
fn a_later_run_that_passes_on_a_commit_with_the_merge_clears_the_alarm_and_keeps_the_label() {
    let setup = red_at_merge("red-recovers");
    setup.tip_run("ci", "m11", "completed", "success", "ahead");

    setup.poll();

    assert_eq!(setup.alarms(), Vec::<String>::new());
    assert_eq!(setup.red_after_merge(), 1);
}

#[test]
fn a_run_that_passes_on_a_commit_without_the_merge_keeps_the_alarm() {
    let setup = red_at_merge("red-not-contained");
    for compare in ["diverged", "behind"] {
        setup.tip_run("ci", "m11", "completed", "success", compare);

        setup.poll();

        assert_eq!(setup.alarms().len(), 1, "{compare}");
    }
}

#[test]
fn a_later_run_that_does_not_succeed_keeps_the_alarm() {
    let setup = red_at_merge("red-still");
    for (status, conclusion) in [
        ("in_progress", ""),
        ("completed", "failure"),
        ("completed", "cancelled"),
        ("completed", "skipped"),
    ] {
        setup.tip_run("ci", "m11", status, conclusion, "ahead");

        setup.poll();

        assert_eq!(setup.alarms().len(), 1, "{status} {conclusion}");
    }
}

#[test]
fn a_rerun_that_passes_on_the_merge_commit_clears_the_alarm() {
    let setup = red_at_merge("red-rerun");
    setup.tip_run("ci", "m10", "completed", "success", "identical");

    setup.poll();

    assert_eq!(setup.alarms(), Vec::<String>::new());
}

#[test]
fn a_later_merge_that_passed_clears_the_alarm_while_the_tip_still_runs() {
    let setup = red_at_merge("red-recovers-at-a-merge");
    setup.tip_run("ci", "m12", "in_progress", "", "ahead");
    setup.task(2, "green");
    setup.handle(&[merged_on(11, 2, "bbb", "m11", "2026-11-20T11:00:00Z")]);
    setup.checks_at("m11", &[(2, "completed", "success")]);
    setup.poll();
    assert_eq!(setup.alarms().len(), 1);

    setup.poll();

    assert_eq!(setup.alarms(), Vec::<String>::new());
}

#[test]
fn a_gone_base_branch_keeps_the_alarm_and_the_poll_goes_on() {
    let setup = red_at_merge("red-base-gone");
    for status in ["404", "422"] {
        fs::write(setup.dir.join(format!("{status}-heads-main")), "").unwrap();

        setup.poll();

        assert_eq!(setup.alarms().len(), 1);
        assert_eq!(setup.github_error(), None, "{status}");
        fs::remove_file(setup.dir.join(format!("{status}-heads-main"))).unwrap();
    }
}

#[test]
fn a_failed_read_of_the_base_branch_keeps_the_alarm_and_shows_in_the_intakes_health() {
    let setup = red_at_merge("red-read-fails");
    fs::write(setup.dir.join("403-heads-main"), "").unwrap();

    let output = setup.intake(NOW);

    assert!(!output.status.success());
    assert!(setup.github_error().unwrap().contains("HTTP 403"));
    assert_eq!(setup.alarms().len(), 1);
}

#[test]
fn a_passing_run_of_the_same_name_on_the_merge_commit_keeps_the_alarm() {
    for (name, red, passed) in [("lower", 1, 2), ("higher", 2, 1)] {
        let setup = Setup::new(&format!("red-same-name-{name}"));
        setup.polled_repo();
        setup.task(1, "green");
        setup.handle(&[merged_on(10, 1, "aaa", "m10", "2026-11-20T10:00:00Z")]);
        setup.checks_at(
            "m10",
            &[
                (red, "completed", "failure"),
                (passed, "completed", "success"),
            ],
        );
        setup.poll();
        // The tip is the merge commit, and both runs stand there.
        fs::copy(
            setup.dir.join("checks-m10.json"),
            setup.dir.join("checks-heads-main.json"),
        )
        .unwrap();

        setup.poll();

        assert_eq!(setup.alarms().len(), 1, "the red run's id is {name}");
    }
}

/// A red merge at `m10` whose `ci` run is 10, and a later merge at `m11` with `runs` of `ci`, where
/// every commit but the merge commit descends from it.
fn later_merge(name: &str, runs: &[(u64, &str, &str)]) -> Setup {
    let setup = Setup::new(name);
    setup.polled_repo();
    setup.task(1, "green");
    setup.handle(&[merged_on(10, 1, "aaa", "m10", "2026-11-20T10:00:00Z")]);
    setup.checks_at("m10", &[(10, "completed", "failure")]);
    setup.poll();
    setup.tip_run("ci", "m12", "in_progress", "", "ahead");
    setup.task(2, "green");
    setup.handle(&[merged_on(11, 2, "bbb", "m11", "2026-11-20T11:00:00Z")]);
    setup.checks_at("m11", runs);
    setup.poll();
    setup.poll();
    setup
}

/// Whether `setup` still shows the alarm of the merge at `m10`.
fn still_red(setup: &Setup) -> bool {
    setup
        .alarms()
        .iter()
        .any(|line| line.contains("owner/repo#10 "))
}

#[test]
fn a_later_merge_with_a_failing_run_of_the_same_name_keeps_the_alarm_in_both_id_orders() {
    for (name, red, passed) in [("lower", 11, 12), ("higher", 12, 11)] {
        let setup = later_merge(
            &format!("red-later-{name}"),
            &[
                (red, "completed", "failure"),
                (passed, "completed", "success"),
            ],
        );
        assert!(still_red(&setup), "the red run's id is {name}");
    }
}

#[test]
fn a_later_merge_with_a_run_still_going_keeps_the_alarm() {
    let setup = later_merge(
        "red-later-pending",
        &[(11, "completed", "success"), (12, "in_progress", "")],
    );
    assert!(still_red(&setup));
}

#[test]
fn a_run_before_the_red_one_does_not_keep_the_alarm() {
    let setup = later_merge(
        "red-later-older-run",
        &[(9, "completed", "failure"), (11, "completed", "success")],
    );
    assert!(!still_red(&setup));
}

#[test]
fn a_run_of_another_workflow_keeps_the_alarm_on_the_tip_and_on_a_later_merge() {
    let setup = red_at_merge("red-other-workflow-tip");
    setup.workflow(500, 2);
    setup.tip_run("ci", "m11", "completed", "success", "ahead");
    setup.poll();
    assert_eq!(setup.alarms().len(), 1);

    let setup = Setup::new("red-other-workflow-merge");
    setup.polled_repo();
    setup.workflow(11, 2);
    setup.task(1, "green");
    setup.task(2, "green");
    setup.handle(&[
        merged_on(10, 1, "aaa", "m10", "2026-11-20T10:00:00Z"),
        merged_on(11, 2, "bbb", "m11", "2026-11-20T11:00:00Z"),
    ]);
    setup.checks_at("m10", &[(10, "completed", "failure")]);
    setup.checks_at("m11", &[(11, "completed", "success")]);
    setup.tip_run("ci", "m12", "in_progress", "", "ahead");
    setup.poll();
    setup.poll();
    assert_eq!(setup.alarms().len(), 1);
}

/// A merge at `m10` where the job `ci` of workflow 1, run 10, and of workflow 2, run 11, both failed.
fn red_in_two_workflows(name: &str) -> Setup {
    let setup = Setup::new(name);
    setup.polled_repo();
    for (run, workflow) in [(10, 1), (11, 2), (12, 2), (13, 1)] {
        setup.workflow(run, workflow);
    }
    setup.task(1, "green");
    setup.handle(&[merged_on(10, 1, "aaa", "m10", "2026-11-20T10:00:00Z")]);
    setup.checks_at(
        "m10",
        &[(10, "completed", "failure"), (11, "completed", "failure")],
    );
    setup.poll();
    assert_eq!(setup.alarms().len(), 1);
    setup
}

#[test]
fn a_tip_where_one_of_two_red_workflows_passed_keeps_the_alarm_until_both_did() {
    let setup = red_in_two_workflows("red-two-workflows-tip");
    fs::write(setup.dir.join("compare.json"), r#"{"status":"ahead"}"#).unwrap();
    let tip = |runs: &[u64]| {
        let runs: Vec<Value> = runs
            .iter()
            .map(|id| {
                json!({ "id": id, "name": "ci", "head_sha": "m12", "status": "completed",
                        "conclusion": "success", "completed_at": "2026-11-20T10:30:00Z",
                        "html_url": format!("https://github.com/owner/repo/actions/runs/{id}/job/{id}"),
                        "app": { "slug": "github-actions" } })
            })
            .collect();
        let answer = json!({ "total_count": runs.len(), "check_runs": runs });
        fs::write(setup.dir.join("checks-heads-main.json"), answer.to_string()).unwrap();
    };

    for alone in [12, 13] {
        tip(&[alone]);
        setup.poll();
        assert_eq!(setup.alarms().len(), 1, "only run {alone} passed");
    }

    tip(&[12, 13]);
    setup.poll();
    assert_eq!(setup.alarms(), Vec::<String>::new());
}

#[test]
fn a_later_merge_where_one_of_two_red_workflows_passed_keeps_the_alarm_until_both_did() {
    for (name, runs, red) in [
        ("one", &[(12, "completed", "success")][..], true),
        (
            "both",
            &[(12, "completed", "success"), (13, "completed", "success")][..],
            false,
        ),
    ] {
        let setup = red_in_two_workflows(&format!("red-two-workflows-merge-{name}"));
        setup.tip_run("ci", "m12", "in_progress", "", "ahead");
        setup.task(2, "green");
        setup.handle(&[merged_on(11, 2, "bbb", "m11", "2026-11-20T11:00:00Z")]);
        setup.checks_at("m11", runs);
        setup.poll();
        setup.poll();
        assert_eq!(still_red(&setup), red, "{name}");
    }
}

#[test]
fn a_token_that_cannot_read_actions_runs_keeps_the_alarm_and_the_source_does_not_fail() {
    let setup = red_at_merge("red-no-actions-permission");
    setup.tip_run("ci", "m11", "completed", "success", "ahead");
    fs::write(setup.dir.join("403-run-1"), "").unwrap();

    setup.poll();

    assert_eq!(setup.alarms().len(), 1);
    assert_eq!(setup.github_error(), None);
}

#[test]
fn a_red_merge_older_than_seven_days_is_neither_read_nor_cleared() {
    let setup = red_at_merge("red-too-old");
    setup.tip_run("ci", "m11", "completed", "success", "ahead");

    setup.poll_at("2026-11-28T10:00:00Z");

    assert_eq!(setup.reads_of("heads/main"), 0);
    assert_eq!(setup.alarms().len(), 1);
}

#[test]
fn a_commit_without_the_merge_is_compared_once_per_process() {
    let setup = red_at_merge("red-compared-once");
    let checks = |id: u64, name: &str| {
        on_pull(
            "github.check_run.completed",
            10,
            1,
            "m10",
            json!({ "conclusion": "failure", "name": name, "app": "other", "github_id": id,
                    "pending": 0, "failing": 2 }),
        )
    };
    setup.handle(&[checks(2, "lint")]);
    setup.task(2, "green");
    setup.handle(&[merged_on(11, 2, "bbb", "m11", "2026-11-20T11:00:00Z")]);
    setup.checks_at("m11", &[(3, "completed", "success")]);
    setup.tip_run("ci", "m11", "completed", "success", "diverged");
    setup.poll();
    let before = setup.calls("compare/");

    setup.poll();

    // The tip and the newest later merge are the same commit, so the poll's second look is cached.
    assert_eq!(setup.calls("compare/") - before, 1);
    assert_eq!(setup.alarms().len(), 1);
}

#[test]
fn a_failing_run_without_an_id_keeps_its_alarm_and_another_one_still_clears() {
    let setup = red_at_merge("red-no-id");
    setup.task(2, "green");
    setup.handle(&[
        merged_on(11, 2, "bbb", "m11", "2026-11-20T11:00:00Z"),
        on_pull(
            "github.check_run.completed",
            11,
            2,
            "m11",
            json!({ "conclusion": "failure", "name": "lint", "app": "other", "github_id": null,
                    "pending": 0, "failing": 1 }),
        ),
    ]);
    assert_eq!(setup.alarms().len(), 2);
    setup.tip_run("ci", "m12", "completed", "success", "ahead");

    setup.poll();

    let alarms = setup.alarms();
    assert_eq!(alarms.len(), 1, "{alarms:?}");
    assert!(alarms[0].contains("owner/repo#11 "), "{alarms:?}");
}

#[test]
fn a_poll_that_could_not_read_the_merges_clears_nothing() {
    let setup = red_at_merge("red-merges-unread");
    setup.tip_run("ci", "m11", "completed", "success", "ahead");
    // Only the read of the merges names this table.
    setup
        .db()
        .execute_batch("DELETE FROM intake_sources; ALTER TABLE pull_requests RENAME TO gone;")
        .unwrap();

    let mut intake = setup.intake_command(NOW);
    let mut intake = intake
        .args(["intake", &setup.factory()])
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    let polled = || {
        setup
            .db()
            .query_row("SELECT count(*) FROM intake_sources", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap()
    };
    while polled() == 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = intake.kill();
    let _ = intake.wait();
    assert_eq!(polled(), 1, "the intake recorded its poll");

    setup
        .db()
        .execute_batch("ALTER TABLE gone RENAME TO pull_requests;")
        .unwrap();
    assert_eq!(setup.alarms().len(), 1);
}

#[test]
fn an_alarm_clears_only_once_every_check_that_failed_there_passed() {
    let setup = red_at_merge("red-two-checks");
    setup.handle(&[on_pull(
        "github.check_run.completed",
        10,
        1,
        "m10",
        json!({ "conclusion": "failure", "name": "lint", "app": "other", "github_id": 2,
                "url": "https://github.com/owner/repo/runs/2", "pending": 0, "failing": 2 }),
    )]);
    setup.tip_run("ci", "m11", "completed", "success", "ahead");
    setup.poll();
    assert_eq!(setup.alarms().len(), 1, "only ci passed");

    // A later merge is where `lint` passed.
    setup.task(2, "green");
    setup.handle(&[
        merged_on(11, 2, "bbb", "m11", "2026-11-20T11:00:00Z"),
        on_pull(
            "github.check_run.completed",
            11,
            2,
            "m11",
            json!({ "conclusion": "success", "name": "lint", "app": "other", "github_id": 3,
                    "pending": 0, "failing": 0 }),
        ),
    ]);
    setup.poll();

    assert_eq!(setup.alarms(), Vec::<String>::new());
}

#[test]
fn a_reverts_alarm_stays_when_the_base_branch_passes_again() {
    let setup = red_at_merge("red-reverted");
    setup.handle(&[on_pull(
        "github.pull_request.closed",
        40,
        9,
        "fff",
        json!({ "merged": true, "merge_commit": "m40", "actor": "octocat",
                "title": "Revert \"Fix 1\"", "body": "Reverts owner/repo#10", "github_id": 940 }),
    )
    .time("2026-11-20T11:00:00Z".parse().unwrap())]);
    assert_eq!(setup.alarms().len(), 2);
    setup.tip_run("ci", "m40", "completed", "success", "ahead");

    setup.poll();

    assert_eq!(
        setup.alarms(),
        [
            "priory: landed wrong: owner/repo#10 of task 1 was reverted by #40: https://github.com/owner/repo/pull/40"
        ]
    );
}

#[test]
fn the_upgrade_marks_read_the_alarms_of_merges_whose_only_red_checks_were_cancelled() {
    let setup = Setup::new("red-upgrade");
    let check = |number: u64, conclusion: &str, id: u64| {
        on_pull(
            "github.check_run.completed",
            number,
            number / 10,
            &format!("m{number}"),
            json!({ "conclusion": conclusion, "name": format!("check {id}"), "github_id": id,
                    "url": format!("https://github.com/owner/repo/runs/{id}"),
                    "pending": 0, "failing": 1 }),
        )
    };
    for task in 1..=5 {
        setup.task(task, "green");
    }
    let mut running = check(50, "cancelled", 6);
    running.data["pending"] = json!(1);
    setup.handle(&[
        merged(10, 1, "aaa", "m10"),
        merged(20, 2, "bbb", "m20"),
        merged(30, 3, "ccc", "m30"),
        merged(50, 5, "eee", "m50"),
        check(10, "cancelled", 1),
        check(10, "success", 2),
        check(20, "cancelled", 3),
        check(20, "failure", 4),
        check(30, "cancelled", 5),
        running,
        on_pull(
            "github.pull_request.closed",
            40,
            9,
            "fff",
            json!({ "merged": true, "merge_commit": "m40", "actor": "octocat",
                    "title": "Revert \"Fix 3\"", "body": "Reverts owner/repo#30", "github_id": 940 }),
        ),
    ]);
    assert_eq!(setup.alarms().len(), 5);

    // Back to before migration 116, which marks them read, without what later ones added.
    setup
        .db()
        .execute_batch(
            "DROP TABLE messages; DROP TABLE pairs; DROP TABLE news; ALTER TABLE outbox DROP COLUMN pair; ALTER TABLE tasks DROP COLUMN labels; DROP INDEX inbox_unaccepted;
             ALTER TABLE inbox DROP COLUMN reason; PRAGMA user_version = 115;",
        )
        .unwrap();
    succeeded(&setup.priory(&["upgrade", "probe", &setup.factory()]));

    assert_eq!(
        setup.alarms(),
        [
            "priory: landed wrong: owner/repo#20 of task 2 is red on main: check 3 cancelled https://github.com/owner/repo/runs/3",
            "priory: landed wrong: owner/repo#50 of task 5 is red on main: check 6 cancelled https://github.com/owner/repo/runs/6",
            "priory: landed wrong: owner/repo#30 of task 3 was reverted by #40: https://github.com/owner/repo/pull/40",
        ]
    );
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
