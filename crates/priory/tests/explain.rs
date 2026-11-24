#[allow(dead_code)]
mod harness;

use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

const FACTORY: &str = r#"name = "demo""#;

const AGENT: &str = r#"+++
description = "Implements one task."
harness = "claude-code"
+++
# Implementation
"#;

const RUNNER: &str = r#"platform = { os = "linux" }"#;

const IMPLEMENT: &str = r#"start = "plan"

[[steps]]
id = "plan"
decides = "model"
agent = "implementer"
prompt = "Plan the change. Grade it red, yellow or green."
effect = "reversible"
next = { red = "approve-plan", yellow = "implement", green = "implement" }

[[steps]]
id = "approve-plan"
decides = "human"
effect = "none"
options = ["approved", "rejected"]
next = { approved = "implement", rejected = "stop" }

[[steps]]
id = "implement"
decides = "model"
agent = "implementer"
prompt = "Implement the plan and run the tests."
effect = "reversible"
next = { passed = "approve-pr", failed = "stop" }

[[steps]]
id = "approve-pr"
decides = "human"
effect = "none"
approves = ["open-pr"]
options = ["open", "stop"]
next = { open = "open-pr", stop = "stop" }

[[steps]]
id = "open-pr"
decides = "code"
action = "open-pr"
effect = "irreversible"
next = { opened = "done", failed = "stop" }
"#;

const DRAFT: &str = r#"start = "plan"

[[steps]]
id = "plan"
decides = "model"
agent = "implementer"
prompt = "Plan the change."
effect = "reversible"
next = { small = "open-pr", large = "approve-plan" }

[[steps]]
id = "approve-plan"
decides = "human"
effect = "none"
options = ["approved", "rejected"]
next = { approved = "open-pr", rejected = "stop" }

[[steps]]
id = "open-pr"
decides = "code"
action = "open-pr"
effect = "irreversible"
autonomous = "draft PRs only"
next = { opened = "done", failed = "stop" }
"#;

fn factory(name: &str, workflows: &[(&str, &str)]) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("explain")
        .join(name);
    let _ = fs::remove_dir_all(&dir);
    let workflows = workflows
        .iter()
        .map(|(name, content)| (format!("workflows/{name}.toml"), *content));
    let files = [
        ("factory.toml".to_owned(), FACTORY),
        ("agents/implementer/agent.md".to_owned(), AGENT),
        ("runners/desktop.toml".to_owned(), RUNNER),
    ];
    for (path, content) in files.into_iter().chain(workflows) {
        let file = dir.join(path);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, content).unwrap();
    }
    dir
}

struct Outcome {
    code: i32,
    stdout: String,
    stderr: String,
}

fn priory(args: &[&str], dir: &Path) -> Outcome {
    let output = harness::priory()
        .args(args)
        .arg(dir)
        .env_remove("PRIORY_DB")
        .env(
            "HERDR_APP_HOME",
            Path::new(env!("CARGO_TARGET_TMPDIR")).join("no-herdr"),
        )
        .output()
        .unwrap();
    Outcome {
        code: output.status.code().unwrap(),
        stdout: String::from_utf8(output.stdout).unwrap(),
        stderr: String::from_utf8(output.stderr).unwrap(),
    }
}

fn explain(dir: &Path) -> Outcome {
    priory(&["check", "--explain"], dir)
}

#[test]
fn the_example_workflow_gates_open_pr_on_approve_pr() {
    let outcome = explain(&factory("example", &[("implement", IMPLEMENT)]));

    assert_eq!(outcome.stderr, "");
    assert_eq!(outcome.code, 0);
    assert_eq!(
        outcome.stdout,
        "workflow\tstep\tgate\tcaught\tjudged wrong\tlanded wrong\n\
         implement\topen-pr\tapprove-pr\t-\t-\t-\n\
         \n\
         workflow\tpath with no human decision\tcaught\tjudged wrong\tlanded wrong\n\
         \n\
         agent\tguard\tworkflow\twhy\n\
         \n\
         workflow\tstep\toutcome\tproof\n"
    );
}

#[test]
fn an_autonomous_step_lists_its_reason_and_the_path_with_no_human_decision() {
    let outcome = explain(&factory("autonomous", &[("draft", DRAFT)]));

    assert_eq!(outcome.stderr, "");
    assert_eq!(outcome.code, 0);
    assert_eq!(
        outcome.stdout,
        "workflow\tstep\tgate\tcaught\tjudged wrong\tlanded wrong\n\
         draft\topen-pr\tautonomous: draft PRs only\t-\t-\t-\n\
         \n\
         workflow\tpath with no human decision\tcaught\tjudged wrong\tlanded wrong\n\
         draft\tplan -> open-pr\t-\t-\t-\n\
         \n\
         agent\tguard\tworkflow\twhy\n\
         \n\
         workflow\tstep\toutcome\tproof\n"
    );
}

#[test]
fn an_auto_answer_level_changes_no_irreversible_step_or_path_with_no_human_decision() {
    let trigger = "source = \"github\"\nrepo = \"owner/repo\"\non = \"issues.labeled\"\nworkflow = \"draft\"\n";
    let without = factory(
        "auto-answer-unset",
        &[("implement", IMPLEMENT), ("draft", DRAFT)],
    );
    fs::create_dir_all(without.join("triggers")).unwrap();
    fs::write(without.join("triggers/agent-label.toml"), trigger).unwrap();
    let with = factory(
        "auto-answer-set",
        &[("implement", IMPLEMENT), ("draft", DRAFT)],
    );
    fs::create_dir_all(with.join("triggers")).unwrap();
    fs::write(
        with.join("triggers/agent-label.toml"),
        format!("{trigger}auto_answer = \"medium\"\n"),
    )
    .unwrap();

    let (before, after) = (explain(&without), explain(&with));

    assert_eq!((before.code, before.stderr.as_str()), (0, ""));
    assert_eq!((after.code, after.stderr.as_str()), (0, ""));
    assert_eq!(after.stdout, before.stdout);
    assert!(
        after.stdout.contains("draft\tplan -> open-pr"),
        "{}",
        after.stdout
    );
}

#[test]
fn json_holds_the_same_rows_as_the_table() {
    let dir = factory("json", &[("implement", IMPLEMENT), ("draft", DRAFT)]);
    let table = explain(&dir).stdout;
    let outcome = priory(&["check", "--explain", "--json"], &dir);

    assert_eq!(outcome.code, 0);
    let json: Value = serde_json::from_str(&outcome.stdout).unwrap();
    let mut rebuilt = String::from("workflow\tstep\tgate\tcaught\tjudged wrong\tlanded wrong\n");
    for gate in json["irreversible"].as_array().unwrap() {
        let gated = match gate["autonomous"].as_str() {
            Some(reason) => format!("autonomous: {reason}"),
            None => gate["approved_by"][0].as_str().unwrap().to_owned(),
        };
        rebuilt += &format!(
            "{}\t{}\t{gated}\t-\t-\t-\n",
            gate["workflow"].as_str().unwrap(),
            gate["step"].as_str().unwrap()
        );
    }
    rebuilt += "\nworkflow\tpath with no human decision\tcaught\tjudged wrong\tlanded wrong\n";
    for path in json["paths"].as_array().unwrap() {
        let steps: Vec<_> = path["path"]
            .as_array()
            .unwrap()
            .iter()
            .map(|step| step.as_str().unwrap())
            .collect();
        rebuilt += &format!(
            "{}\t{}\t-\t-\t-\n",
            path["workflow"].as_str().unwrap(),
            steps.join(" -> ")
        );
    }
    rebuilt += "\nagent\tguard\tworkflow\twhy\n\nworkflow\tstep\toutcome\tproof\n";
    assert_eq!(json["tools"], serde_json::json!([]));
    assert_eq!(json["proofs"], serde_json::json!([]));
    assert_eq!(rebuilt, table);
    assert_eq!(json["irreversible"].as_array().unwrap().len(), 2);
}

#[test]
fn an_explicit_by_overrides_the_steps_decider() {
    let gated = DRAFT.replace(
        "next = { small = \"open-pr\", large = \"approve-plan\" }",
        "next = { small = \"open-pr\", large = \"approve-plan\" }\nby = { small = \"human\" }",
    );
    let outcome = explain(&factory("by", &[("draft", &gated)]));

    assert_eq!(outcome.stderr, "");
    assert_eq!(
        outcome.stdout,
        "workflow\tstep\tgate\tcaught\tjudged wrong\tlanded wrong\n\
         draft\topen-pr\tautonomous: draft PRs only\t-\t-\t-\n\
         \n\
         workflow\tpath with no human decision\tcaught\tjudged wrong\tlanded wrong\n\
         \n\
         agent\tguard\tworkflow\twhy\n\
         \n\
         workflow\tstep\toutcome\tproof\n"
    );
}

#[test]
fn a_by_outcome_must_be_in_next() {
    let wrong = DRAFT.replace(
        "next = { small = \"open-pr\", large = \"approve-plan\" }",
        "next = { small = \"open-pr\", large = \"approve-plan\" }\nby = { tiny = \"asleep\" }",
    );
    let outcome = priory(&["check"], &factory("by-unknown", &[("draft", &wrong)]));

    assert_eq!(outcome.code, 1);
    assert_eq!(
        outcome.stderr,
        "workflows/draft.toml:3: step `plan`: `by` has outcome `tiny`, which is not in `next`\n"
    );
}

#[test]
fn a_human_step_after_the_irreversible_step_is_not_its_gate() {
    let publish = r#"start = "approve"

[[steps]]
id = "approve"
decides = "human"
effect = "none"
approves = ["publish"]
options = ["go"]
next = { go = "publish" }

[[steps]]
id = "publish"
decides = "code"
action = "open-pr"
effect = "irreversible"
next = { opened = "retro" }

[[steps]]
id = "retro"
decides = "human"
effect = "none"
approves = ["publish"]
options = ["ok"]
next = { ok = "done" }
"#;
    let outcome = explain(&factory("after", &[("publish", publish)]));

    assert_eq!(outcome.stderr, "");
    assert!(
        outcome
            .stdout
            .starts_with("workflow\tstep\tgate\tcaught\tjudged wrong\tlanded wrong\npublish\tpublish\tapprove\t-\t-\t-\n")
    );
}

#[test]
fn a_multiline_reason_stays_on_one_row() {
    let multiline = DRAFT.replace("\"draft PRs only\"", "\"draft\\tPRs\\nonly\"");
    let outcome = explain(&factory("multiline", &[("draft", &multiline)]));

    assert_eq!(outcome.stderr, "");
    assert!(
        outcome
            .stdout
            .contains("draft\topen-pr\tautonomous: draft\\tPRs\\nonly\t-\t-\t-\n")
    );
}

const DOCS: &str = r#"start = "write"
guard = { allow = ["git-push-default"] }

[[steps]]
id = "write"
decides = "model"
agent = "implementer"
prompt = "Write the docs and push them to main."
effect = "reversible"
next = { pushed = "release" }

[[steps]]
id = "release"
decides = "model"
agent = "implementer"
prompt = "Publish the release."
effect = "irreversible"
autonomous = "the docs site rebuilds from main"
guard = { allow = ["gh-pr-comment"] }
next = { released = "done" }
"#;

#[test]
fn an_agent_lists_the_guarded_calls_it_can_make_with_no_human_decision() {
    let outcome = explain(&factory(
        "tools",
        &[("implement", IMPLEMENT), ("docs", DOCS)],
    ));

    assert_eq!(outcome.stderr, "");
    assert_eq!(outcome.code, 0);
    let table = outcome
        .stdout
        .split("agent\tguard\tworkflow\twhy\n")
        .nth(1)
        .unwrap();
    let rows: Vec<&str> = table.lines().take_while(|row| !row.is_empty()).collect();
    assert!(
        rows.contains(&"implementer\tgit-push-default\tdocs\tallowed by the workflow"),
        "{table}"
    );
    assert!(
        rows.contains(&"implementer\tgh-pr-comment\tdocs\tautonomous step release"),
        "{table}"
    );
    assert_eq!(rows.len(), 2, "{table}");
}

#[test]
fn only_a_persons_approval_grants_a_merge_or_an_approving_review() {
    let autonomous = DOCS.replace("\"gh-pr-comment\"", "\"gh-pr-merge\"");
    let workflow = DOCS.replace("git-push-default", "gh-pr-approve");

    let outcomes = [autonomous, workflow]
        .map(|docs| priory(&["check"], &factory("allow-merge", &[("docs", &docs)])));

    assert_eq!(outcomes.each_ref().map(|outcome| outcome.code), [1, 1]);
    assert_eq!(
        outcomes[0].stderr,
        "workflows/docs.toml:12: step `release`: `guard.allow` names `gh-pr-merge`, which only a person's approval grants, and the step is `autonomous`\n"
    );
    assert_eq!(
        outcomes[1].stderr,
        "workflows/docs.toml:2: `guard.allow` names `gh-pr-approve`, which only a person's approval grants, so only an irreversible step may name it\n"
    );
}

#[test]
fn no_workflow_or_step_grants_priorys_person_verbs() {
    let step = DOCS.replace("\"gh-pr-comment\"", "\"priory-person\"");
    let workflow = DOCS.replace("git-push-default", "priory-person");

    let outcomes = [step, workflow]
        .map(|docs| priory(&["check"], &factory("allow-person", &[("docs", &docs)])));

    assert_eq!(outcomes.each_ref().map(|outcome| outcome.code), [1, 1]);
    assert_eq!(
        outcomes[0].stderr,
        "workflows/docs.toml:12: step `release`: `guard.allow` names `priory-person`, which only a person runs, so nothing may grant it\n"
    );
    assert_eq!(
        outcomes[1].stderr,
        "workflows/docs.toml:2: `guard.allow` names `priory-person`, which only a person runs, so nothing may grant it\n"
    );
}

#[test]
fn no_workflow_or_step_grants_a_connector_sign_in() {
    let step = DOCS.replace("\"gh-pr-comment\"", "\"connector-sign-in\"");
    let workflow = DOCS.replace("git-push-default", "connector-sign-in");

    let outcomes = [step, workflow]
        .map(|docs| priory(&["check"], &factory("allow-sign-in", &[("docs", &docs)])));

    assert_eq!(outcomes.each_ref().map(|outcome| outcome.code), [1, 1]);
    assert_eq!(
        outcomes[0].stderr,
        "workflows/docs.toml:12: step `release`: `guard.allow` names `connector-sign-in`, which only an agent's `connectorSignIn` grants\n"
    );
    assert_eq!(
        outcomes[1].stderr,
        "workflows/docs.toml:2: `guard.allow` names `connector-sign-in`, which only an agent's `connectorSignIn` grants\n"
    );
}

#[test]
fn no_workflow_grants_a_script_the_guard_cannot_read() {
    let workflow = DOCS.replace("git-push-default", "unreadable");

    let outcome = priory(
        &["check"],
        &factory("allow-unreadable", &[("docs", &workflow)]),
    );

    assert_eq!(outcome.code, 1);
    assert_eq!(
        outcome.stderr,
        "workflows/docs.toml:2: `guard.allow` names `unreadable`, which the guard cannot read, so nothing may grant it\n"
    );
}

#[test]
fn a_step_allows_only_guards_that_exist_and_only_when_irreversible() {
    let docs = DOCS
        .replace(
            "effect = \"reversible\"",
            "effect = \"reversible\"\nguard = { allow = [\"gh-pr-comment\"] }",
        )
        .replace(
            "[\"gh-pr-comment\"] }\nnext = { released",
            "[\"gh-pr-coment\"] }\nnext = { released",
        );

    let outcome = priory(&["check"], &factory("step-allow", &[("docs", &docs)]));

    assert_eq!(outcome.code, 1);
    assert_eq!(
        outcome.stderr,
        "workflows/docs.toml:4: step `write`: `guard.allow` grants nothing on a step that is not an irreversible model step\nworkflows/docs.toml:13: step `release`: `guard.allow` names `gh-pr-coment`, which is no built-in guard nor one of `[guard]` in factory.toml\n"
    );
}

#[test]
fn a_workflow_cannot_allow_a_guard_nobody_defined() {
    let wrong = DOCS.replace("git-push-default", "git-push-defualt");
    let outcome = priory(&["check"], &factory("allow-unknown", &[("docs", &wrong)]));

    assert_eq!(outcome.code, 1);
    assert_eq!(
        outcome.stderr,
        "workflows/docs.toml:2: `guard.allow` names `git-push-defualt`, which is no built-in guard nor one of `[guard]` in factory.toml\n"
    );
}

#[test]
fn a_factory_guard_cannot_take_a_built_in_guards_name() {
    let dir = factory("guard-clash", &[("implement", IMPLEMENT)]);
    fs::write(
        dir.join("factory.toml"),
        "name = \"demo\"\n\n[guard]\ncommands = [{ name = \"gh-pr-merge\", run = [\"hub\", \"merge\"] }]\n",
    )
    .unwrap();

    let outcome = priory(&["check"], &dir);

    assert_eq!(outcome.code, 1);
    assert_eq!(
        outcome.stderr,
        "factory.toml:3: `[guard]` command `gh-pr-merge` has the name of another guard\n"
    );
}

#[test]
fn check_reports_what_is_wrong_with_private_terms_by_label_and_never_by_term() {
    let dir = factory("private-problems", &[("implement", IMPLEMENT)]);
    let check = |terms: &str| {
        fs::write(
            dir.join("factory.toml"),
            format!("name = \"demo\"\n\n[private]\n{terms}\n"),
        )
        .unwrap();
        priory(&["check"], &dir).stderr
    };
    for (terms, problem) in [
        (
            "repos = [\"o\"]",
            "`[private] repos` `o` needs to be `owner/name`",
        ),
        (
            "terms = [{ label = \" \", text = [\"a\"] }]",
            "`[private]` term 1 needs a label",
        ),
        (
            "terms = [{ label = \"x\" }]",
            "`[private]` term `x` needs `text` or `regex`",
        ),
        (
            "terms = [{ label = \"x\", text = [\"aa\"] }, { label = \"x\", text = [\"bb\"] }]",
            "`[private]` label `x` is used twice",
        ),
        (
            "terms = [{ label = \"x\", regex = [\"(\"] }]",
            "`[private]` term `x` has a regex that does not compile: error: unclosed group",
        ),
        (
            "terms = [{ label = \"x\", text = [\"\"] }]",
            "`[private]` term `x` matches empty text",
        ),
        (
            "terms = [{ label = \"on-vault9\", text = [\"vault9\"] }]",
            "`[private]` label 1 holds a private term, and refusals print labels",
        ),
    ] {
        assert_eq!(check(terms), format!("factory.toml:3: {problem}\n"));
    }
    assert_eq!(
        check("repos = [\"o/r\"]\nterms = [{ label = \"x\", text = [\"vault9\"] }]"),
        ""
    );
}

#[test]
fn a_codex_agent_with_mcp_servers_lists_mcp_calls_the_guard_may_not_see() {
    let dir = factory("guard-codex", &[("implement", IMPLEMENT)]);
    fs::write(
        dir.join("agents/implementer/agent.md"),
        "+++\ndescription = \"Implements one task.\"\nharness = \"codex\"\nmcpServers = [\"github\"]\n+++\n",
    )
    .unwrap();

    let outcome = explain(&dir);

    assert_eq!(outcome.stderr, "");
    assert!(
        outcome.stdout.ends_with(
            "agent\tguard\tworkflow\twhy\nimplementer\tmcp-write\t-\tcodex: an MCP call may skip the guard's hook\n\nworkflow\tstep\toutcome\tproof\n"
        ),
        "{}",
        outcome.stdout
    );
}

/// `IMPLEMENT` with its `implement` step's `passed` proved by `proof`, in a factory whose gate is
/// `cargo test`.
fn proved(name: &str, proof: &str) -> PathBuf {
    let proved = IMPLEMENT.replace(
        "next = { passed = \"approve-pr\", failed = \"stop\" }",
        &format!("next = {{ passed = \"approve-pr\", failed = \"stop\" }}\n{proof}"),
    );
    let dir = factory(name, &[("implement", &proved)]);
    fs::write(
        dir.join("factory.toml"),
        "name = \"demo\"\ngate = \"cargo test\"\n",
    )
    .unwrap();
    dir
}

#[test]
fn an_outcome_with_a_proof_is_listed_as_verified() {
    let dir = proved("proof", "proof = { passed = \"gate-passed\" }");

    let outcome = explain(&dir);

    assert_eq!(outcome.stderr, "");
    assert_eq!(
        outcome.stdout,
        "workflow\tstep\tgate\tcaught\tjudged wrong\tlanded wrong\n\
         implement\topen-pr\tapprove-pr\t-\t-\t-\n\
         \n\
         workflow\tpath with no human decision\tcaught\tjudged wrong\tlanded wrong\n\
         \n\
         agent\tguard\tworkflow\twhy\n\
         \n\
         workflow\tstep\toutcome\tproof\n\
         implement\timplement\tpassed\tgate-passed\n"
    );
    let json: Value =
        serde_json::from_str(&priory(&["check", "--explain", "--json"], &dir).stdout).unwrap();
    assert_eq!(
        json["proofs"],
        serde_json::json!([
            { "workflow": "implement", "step": "implement", "outcome": "passed", "proof": "gate-passed" }
        ])
    );
}

#[test]
fn a_proof_needs_a_model_step_an_outcome_in_next_and_a_gate() {
    let cases = [
        (
            "proof = { failed = \"gate-passed\", lost = \"gate-passed\" }",
            "workflows/implement.toml:18: step `implement`: `proof` has outcome `lost`, which is not in `next`\n",
        ),
        (
            "proof = { passed = \"tests-ran\" }",
            "unknown variant `tests-ran`, expected `gate-passed`",
        ),
    ];
    for (proof, error) in cases {
        let outcome = priory(&["check"], &proved("proof-wrong", proof));

        assert_eq!(outcome.code, 1, "{proof}");
        assert!(
            outcome.stderr.contains(error),
            "{proof}: {}",
            outcome.stderr
        );
    }
    let on_human = IMPLEMENT.replace(
        "next = { approved = \"implement\", rejected = \"stop\" }",
        "next = { approved = \"implement\", rejected = \"stop\" }\nproof = { approved = \"gate-passed\" }",
    );
    let dir = factory("proof-human", &[("implement", &on_human)]);
    let outcome = priory(&["check"], &dir);
    assert_eq!(
        outcome.stderr,
        "workflows/implement.toml:11: step `approve-plan`: only a model step has a `proof`\n\
         workflows/implement.toml:11: step `approve-plan`: `proof` names `gate-passed`, and factory.toml sets no `gate`\n"
    );
}

#[test]
fn a_blank_gate_is_refused_since_it_would_pass_every_proof() {
    let dir = proved("gate-blank", "proof = { passed = \"gate-passed\" }");
    fs::write(dir.join("factory.toml"), "name = \"demo\"\ngate = \" \"\n").unwrap();

    let outcome = priory(&["check"], &dir);

    assert_eq!(outcome.code, 1);
    assert_eq!(outcome.stderr, "factory.toml:2: `gate` is empty\n");
}
