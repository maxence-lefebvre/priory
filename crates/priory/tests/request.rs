mod conform;

use herdr_bus::{Bus, MessageQuery, Stored};
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const WORKFLOW: &str = r#"
start = "implement"

[[steps]]
id = "implement"
decides = "model"
agent = "implementer"
prompt = "Implement the issue."
effect = "reversible"
next = { passed = "done", failed = "stop" }
"#;

const AGENT: &str = "+++\ndescription = \"Implements one task.\"\nharness = \"claude-code\"\n+++\n";

struct Setup {
    dir: PathBuf,
}

impl Setup {
    fn new(name: &str) -> Self {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join("request")
            .join(name);
        let _ = fs::remove_dir_all(&dir);
        let files = [
            ("factory/factory.toml", "name = \"demo\""),
            ("factory/agents/implementer/agent.md", AGENT),
            ("factory/workflows/implement.toml", WORKFLOW),
        ];
        for (path, content) in files {
            let file = dir.join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, content).unwrap();
        }
        Setup { dir }
    }

    fn priory(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_priory"))
            .args(args)
            .env("HERDR_BUS_DB", self.dir.join("bus.db"))
            .env("PRIORY_DB", self.dir.join("priory.db"))
            .env("USER", "max")
            .output()
            .unwrap()
    }

    /// Runs `priory request` in repository `owner/repo`; a later `--repo` in `args` wins.
    fn request(&self, args: &[&str]) -> Output {
        let factory = self.dir.join("factory");
        let mut all = vec!["request", factory.to_str().unwrap(), "--repo", "owner/repo"];
        all.extend(args);
        self.priory(&all)
    }

    fn published(&self) -> Vec<Stored> {
        let Ok(bus) = Bus::open(&self.dir.join("bus.db")) else {
            return vec![];
        };
        let query = MessageQuery {
            output: Some("factory.intake".into()),
            limit: 1000,
            ..MessageQuery::default()
        };
        let mut messages = bus.messages(&query).unwrap_or_default();
        messages.reverse();
        for stored in &messages {
            conform::conforms(&serde_json::to_value(&stored.message).unwrap());
        }
        messages
    }

    fn tasks(&self) -> Vec<Vec<String>> {
        let output = self.priory(&["tasks"]);
        succeeded(&output);
        String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .skip(1)
            .map(|line| line.split('\t').map(str::to_owned).collect())
            .collect()
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
fn a_request_publishes_its_source_event_then_the_task_request() {
    let setup = Setup::new("publishes");

    succeeded(&setup.request(&[
        "--workflow",
        "implement",
        "--key",
        "docs-1",
        "--band",
        "today",
        "Fix the docs",
    ]));

    let published = setup.published();
    let kinds: Vec<_> = published.iter().map(|m| m.message.kind.as_str()).collect();
    assert_eq!(
        kinds,
        ["priory.task.requested.cli", "factory.task.requested"]
    );
    let (source, task) = (&published[0].message, &published[1].message);
    assert_eq!(source.subject.as_deref(), Some("cli:docs-1"));
    assert_eq!(source.data["text"], "Fix the docs");
    assert_eq!(source.data["person"], "max");
    assert_eq!(source.data["repo"], "owner/repo");
    assert!(source.data["time"].as_str().unwrap().ends_with('Z'));
    assert_eq!(
        task.data,
        json!({
            "workflow": "implement",
            "key": "cli:docs-1",
            "band": "today",
            "lane": null,
            "trigger": "cli",
            "source": source.id.to_string(),
            "repo": "owner/repo",
        })
    );
    assert_eq!(task.subject.as_deref(), Some("cli:docs-1"));
}

#[test]
fn a_request_with_no_repo_exits_non_zero_and_publishes_nothing() {
    let setup = Setup::new("no-repo");
    let factory = setup.dir.join("factory");

    let output = setup.priory(&[
        "request",
        factory.to_str().unwrap(),
        "--workflow",
        "implement",
        "--key",
        "docs-1",
        "Fix the docs",
    ]);

    assert!(!output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "priory request: the request names no repo; pass --repo owner/name, or set repo in triggers/cli.toml\n"
    );
    assert!(setup.published().is_empty());
}

#[test]
fn a_repo_that_is_not_owner_slash_name_exits_non_zero_and_publishes_nothing() {
    let setup = Setup::new("bad-repo");

    for repo in ["/tmp/x", "../x", "owner/../x", "-x/y", "owner/.", "a/b/c"] {
        let output = setup.request(&[
            "--workflow",
            "implement",
            "--key",
            "docs-1",
            "--repo",
            repo,
            "Fix the docs",
        ]);

        assert!(!output.status.success(), "{repo} was accepted");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("owner/name"), "{repo}: {stderr}");
    }
    assert!(setup.published().is_empty());
}

#[test]
fn the_same_request_twice_publishes_nothing_new() {
    let setup = Setup::new("twice");
    let args = ["--workflow", "implement", "--key", "docs-1", "Fix the docs"];

    succeeded(&setup.request(&args));
    succeeded(&setup.request(&args));

    assert_eq!(setup.published().len(), 2);
}

#[test]
fn an_unknown_workflow_exits_non_zero_and_publishes_nothing() {
    let setup = Setup::new("unknown");

    let output = setup.request(&["--workflow", "nope", "--key", "docs-1", "Fix the docs"]);

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("nope"));
    assert!(setup.published().is_empty());
}

#[test]
fn an_unknown_band_exits_non_zero_and_publishes_nothing() {
    let setup = Setup::new("unknown-band");

    let output = setup.request(&[
        "--workflow",
        "implement",
        "--key",
        "docs-1",
        "--band",
        "someday",
        "Fix the docs",
    ]);

    assert!(!output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "priory request: band `someday` is not one of blocker, now, today, later\n"
    );
    assert!(setup.published().is_empty());
}

#[test]
fn the_daemon_turns_a_request_into_one_queued_task() {
    let setup = Setup::new("daemon");
    succeeded(&setup.request(&[
        "--workflow",
        "implement",
        "--key",
        "docs-1",
        "--lane",
        "docs",
        "Fix the docs",
    ]));

    let factory = setup.dir.join("factory");
    succeeded(&setup.priory(&["daemon", factory.to_str().unwrap(), "--once"]));

    assert_eq!(
        setup.tasks(),
        [[
            "1",
            "cli:docs-1",
            "implement",
            "implement",
            "queued",
            "later",
            "docs",
            "-",
            "1",
            "1",
            "-",
            "-",
            "-"
        ]]
    );
}

#[test]
fn a_colon_in_the_key_or_text_does_not_merge_two_requests() {
    let setup = Setup::new("collision");

    succeeded(&setup.request(&["--workflow", "implement", "--key", "a:b", "c"]));
    succeeded(&setup.request(&["--workflow", "implement", "--key", "a", "b:c"]));

    assert_eq!(setup.published().len(), 4);
}

#[test]
fn a_flag_without_its_value_is_a_usage_error_and_publishes_nothing() {
    let setup = Setup::new("missing-value");

    let output = setup.request(&["--workflow", "implement", "--key", "--band", "today"]);

    assert_eq!(output.status.code(), Some(2));
    assert!(setup.published().is_empty());
}

#[test]
fn text_after_a_double_dash_is_literal() {
    let setup = Setup::new("literal");

    succeeded(&setup.request(&["--workflow", "implement", "--key", "k", "--", "--band"]));

    assert_eq!(setup.published()[0].message.data["text"], "--band");
}

#[test]
fn the_source_event_time_is_the_time_in_its_data() {
    let setup = Setup::new("time");
    succeeded(&setup.request(&["--workflow", "implement", "--key", "k", "text"]));

    let source = &setup.published()[0].message;

    assert_eq!(source.time.to_string(), source.data["time"]);
}

#[test]
fn a_second_request_for_a_key_is_kept_on_its_task() {
    let setup = Setup::new("follow-up");
    let factory = setup.dir.join("factory");
    succeeded(&setup.request(&["--workflow", "implement", "--key", "k", "first"]));
    succeeded(&setup.priory(&["daemon", factory.to_str().unwrap(), "--once"]));
    succeeded(&setup.request(&["--workflow", "implement", "--key", "k", "second"]));
    succeeded(&setup.priory(&["daemon", factory.to_str().unwrap(), "--once"]));

    let inbox = String::from_utf8(setup.priory(&["inbox"]).stdout).unwrap();
    let outcomes: Vec<_> = inbox
        .lines()
        .skip(1)
        .map(|l| l.split('\t').collect::<Vec<_>>())
        .collect();

    assert_eq!(outcomes.len(), 4);
    assert_eq!(outcomes[2][3..5], ["1", "kept"]);
    assert_eq!(setup.tasks().len(), 1);
}

#[test]
fn a_request_with_a_hold_files_its_task_held() {
    let setup = Setup::new("held");
    let request = ["--workflow", "implement", "--key"];
    succeeded(
        &setup.request(
            &[
                &request[..],
                &["docs-1", "--hold", "after the release"],
                &["--until", "2030-01-01T00:00:00Z", "Fix the docs"],
            ]
            .concat(),
        ),
    );
    succeeded(
        &setup.request(
            &[
                &request[..],
                &["docs-2", "--hold", "later", "--after", "99", "Fix more"],
            ]
            .concat(),
        ),
    );
    let both = [
        "--after",
        "1",
        "--after",
        "ENG-7",
        "--merge",
        "owner/repo#7",
    ];
    succeeded(
        &setup.request(
            &[
                &request[..],
                &["docs-3", "--hold", "both"],
                &both,
                &["Fix it"],
            ]
            .concat(),
        ),
    );
    let factory = setup.dir.join("factory");

    succeeded(&setup.priory(&["daemon", factory.to_str().unwrap(), "--once"]));

    assert_eq!(
        setup.published()[1].message.data["hold"],
        json!({ "reason": "after the release", "until": "2030-01-01T00:00:00Z" })
    );
    let tasks: Vec<_> = setup
        .tasks()
        .into_iter()
        .map(|row| [row[4].clone(), row[7].clone()])
        .collect();
    assert_eq!(
        tasks,
        [
            [
                "held",
                "held: after the release, until 2030-01-01T00:00:00.000Z"
            ],
            [
                "held",
                "held: later, until a person releases it, since its condition failed: after must name another task, not 99"
            ],
            [
                "held",
                "held: both, until task 1 ends and until ENG-7 is Done or Canceled and until github:owner/repo#7 merges, and asks if it closes unmerged"
            ]
        ]
    );
    assert_eq!(
        setup.published()[5].message.data["hold"],
        json!({ "reason": "both", "after": [1, "ENG-7"], "merge": ["owner/repo#7"] })
    );
}

const DEFAULTS: &str = "name = \"demo\"\n\n[request]\nworkflow = \"implement\"\nband = \"today\"\n";

impl Setup {
    /// A factory whose `factory.toml` is `settings`.
    fn with(name: &str, settings: &str) -> Self {
        let setup = Setup::new(name);
        fs::write(setup.dir.join("factory/factory.toml"), settings).unwrap();
        setup
    }

    /// Runs `priory add` on the factory on 2026-11-20 in UTC.
    fn add(&self, args: &[&str]) -> Output {
        let factory = self.dir.join("factory");
        let mut all = vec!["add", factory.to_str().unwrap()];
        all.extend(args);
        Command::new(env!("CARGO_BIN_EXE_priory"))
            .args(all)
            .env("HERDR_BUS_DB", self.dir.join("bus.db"))
            .env("PRIORY_DB", self.dir.join("priory.db"))
            .env("PRIORY_NOW", "2026-11-20T10:00:00Z")
            .env("TZ", "UTC")
            .env("USER", "max")
            .output()
            .unwrap()
    }

    fn daemon_once(&self) {
        let factory = self.dir.join("factory");
        succeeded(&self.priory(&["daemon", factory.to_str().unwrap(), "--once"]));
    }
}

#[test]
fn add_queues_a_task_on_the_default_workflow_and_band_under_a_generated_key() {
    let setup = Setup::with("add", DEFAULTS);

    let output = setup.add(&["--repo", "owner/repo", "Fix the docs"]);

    succeeded(&output);
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "cli:2026-11-20-1\n"
    );
    setup.daemon_once();
    let task = &setup.tasks()[0];
    assert_eq!(
        task[..6],
        [
            "1",
            "cli:2026-11-20-1",
            "implement",
            "implement",
            "queued",
            "today"
        ]
    );
    assert_eq!(setup.published()[0].message.data["text"], "Fix the docs");
}

#[test]
fn adds_on_the_same_day_get_different_keys_even_when_they_run_at_once() {
    let setup = Setup::with("add-twice", DEFAULTS);

    let keys: std::collections::BTreeSet<String> = std::thread::scope(|scope| {
        let adds: Vec<_> = (0..4)
            .map(|_| scope.spawn(|| setup.add(&["--repo", "owner/repo", "Same text"])))
            .collect();
        adds.into_iter()
            .map(|add| {
                let output = add.join().unwrap();
                succeeded(&output);
                String::from_utf8(output.stdout).unwrap().trim().to_owned()
            })
            .collect()
    });

    let expected: std::collections::BTreeSet<String> =
        (1..=4).map(|n| format!("cli:2026-11-20-{n}")).collect();
    assert_eq!(keys, expected);
    setup.daemon_once();
    assert_eq!(setup.tasks().len(), 4);
}

#[test]
fn a_github_issue_url_in_the_text_sets_the_repo_and_the_key() {
    let setup = Setup::with("add-github", DEFAULTS);

    let issue = setup.add(&["See https://github.com/cli/cli/issues/42."]);
    let pull = setup.add(&[
        "Review (https://github.com/cli/cli/pull/7/files), see https://github.com/cli/cli/pull/7",
    ]);
    let two =
        setup.add(&["https://github.com/cli/cli/issues/1 and https://github.com/cli/cli/issues/2"]);

    let keys: Vec<_> = [issue, pull, two]
        .iter()
        .map(|output| {
            succeeded(output);
            String::from_utf8_lossy(&output.stdout).trim().to_owned()
        })
        .collect();
    assert_eq!(
        keys,
        ["github:cli/cli#42", "github:cli/cli#7", "cli:2026-11-20-1"]
    );
    let published = setup.published();
    let repos: Vec<_> = published
        .iter()
        .filter(|stored| stored.message.kind == "factory.task.requested")
        .map(|stored| &stored.message.data["repo"])
        .collect();
    assert_eq!(repos, ["cli/cli", "cli/cli", "cli/cli"]);
}

#[test]
fn a_linear_issue_url_sets_the_key_and_its_team_trigger_s_repo() {
    let setup = Setup::with("add-linear", DEFAULTS);
    fs::create_dir_all(setup.dir.join("factory/triggers")).unwrap();
    fs::write(
        setup.dir.join("factory/triggers/eng.toml"),
        "source = \"linear\"\nrepo = \"owner/eng\"\non = \"issue.created\"\nfilter = { team = \"ENG\" }\nworkflow = \"implement\"\n",
    )
    .unwrap();

    let output = setup.add(&["Do https://linear.app/acme/issue/ENG-7/fix-it"]);

    succeeded(&output);
    assert_eq!(String::from_utf8_lossy(&output.stdout), "linear:ENG-7\n");
    assert_eq!(setup.published()[1].message.data["repo"], "owner/eng");
}

#[test]
fn flags_override_the_defaults_and_the_url_s_repo() {
    let setup = Setup::with("add-flags", DEFAULTS);
    fs::write(setup.dir.join("factory/workflows/groom.toml"), WORKFLOW).unwrap();

    succeeded(&setup.add(&[
        "--workflow",
        "groom",
        "--band",
        "now",
        "--lane",
        "docs/site",
        "--repo",
        "owner/other",
        "See https://github.com/cli/cli/issues/42",
    ]));

    let task = &setup.published()[1].message.data;
    assert_eq!(
        [
            &task["workflow"],
            &task["band"],
            &task["lane"],
            &task["repo"]
        ],
        ["groom", "now", "docs/site", "owner/other"]
    );
}

#[test]
fn add_without_a_request_table_or_workflow_says_what_to_set_and_publishes_nothing() {
    let setup = Setup::new("add-no-defaults");

    let output = setup.add(&["--repo", "owner/repo", "Fix the docs"]);

    assert!(!output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "priory add: the request names no workflow; pass --workflow, or set workflow in factory.toml's [request] table\n"
    );
    assert!(setup.published().is_empty());
}

#[test]
fn two_linear_triggers_of_one_team_with_different_repos_set_no_repo() {
    let setup = Setup::with("add-linear-ambiguous", DEFAULTS);
    fs::create_dir_all(setup.dir.join("factory/triggers")).unwrap();
    for (name, repo) in [("eng", "owner/eng"), ("eng-web", "owner/web")] {
        fs::write(
            setup.dir.join(format!("factory/triggers/{name}.toml")),
            format!("source = \"linear\"\nrepo = \"{repo}\"\non = \"issue.created\"\nfilter = {{ team = \"ENG\" }}\nworkflow = \"implement\"\n"),
        )
        .unwrap();
    }

    let output = setup.add(&["Do https://linear.app/acme/issue/ENG-7/fix-it"]);

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("names no repo"));
    assert!(setup.published().is_empty());
}

#[test]
fn a_requests_intervention_reaches_its_task() {
    let setup = Setup::new("intervention");
    succeeded(&setup.request(&[
        "--workflow",
        "implement",
        "--key",
        "docs-1",
        "--intervention",
        "plan",
        "Fix the docs",
    ]));

    let factory = setup.dir.join("factory");
    succeeded(&setup.priory(&["daemon", factory.to_str().unwrap(), "--once"]));

    let task: serde_json::Value =
        serde_json::from_slice(&setup.priory(&["task", "1", "--json"]).stdout).unwrap();
    assert_eq!(task["intervention"], "plan", "{task}");
}

#[test]
fn a_hands_off_request_without_a_cli_lead_exits_non_zero_and_publishes_nothing() {
    let setup = Setup::new("hands-off-no-lead");

    let output = setup.request(&[
        "--workflow",
        "implement",
        "--key",
        "docs-1",
        "--intervention",
        "hands-off",
        "Fix the docs",
    ]);

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "priory request: --intervention hands-off needs a lead; set lead in triggers/cli.toml\n"
    );
    assert!(setup.published().is_empty());
}

#[test]
fn add_takes_an_intervention() {
    let setup = Setup::with("add-intervention", DEFAULTS);

    succeeded(&setup.add(&[
        "--repo",
        "owner/repo",
        "--intervention",
        "plan",
        "Fix the docs",
    ]));

    assert_eq!(setup.published()[1].message.data["intervention"], "plan");
}

#[test]
fn an_unknown_intervention_is_a_usage_error() {
    let setup = Setup::new("bad-intervention");

    let output = setup.request(&[
        "--workflow",
        "implement",
        "--key",
        "docs-1",
        "--intervention",
        "yolo",
        "Fix the docs",
    ]);

    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(setup.published().is_empty());
}
