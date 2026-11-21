use serde_json::{Value, json};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn dir(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("eval")
        .join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Feeds `lines` to `priory mcp` with `args` and returns its JSON replies.
fn serve(args: &[&str], lines: &[Value]) -> Vec<Value> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_priory"))
        .arg("mcp")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    for line in lines {
        writeln!(stdin, "{line}").unwrap();
    }
    drop(stdin);
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

const EVAL: &str = r#"
doc = "behaviour.md"
variants = [{ name = "old", file = "prompt.txt" }, { name = "new", text = "Call result." }]

[[cases]]
name = "done"
before = ["Do the step."]
prompt = "{prompt} Outcomes: passed."
expect = "result passed"
"#;

const DOC: &str = "# Behaviour\n\n<!-- priory eval prompt.eval.toml -->\nold table\n<!-- /priory eval -->\n\nAfter.\n";

/// Runs `priory eval` in `dir` with the fake Claude, which ends every turn without a tool call.
fn eval(dir: &Path, args: &[&str]) -> std::process::Output {
    let fake = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fake");
    let path = std::env::join_paths(
        std::iter::once(fake).chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
    )
    .unwrap();
    fs::create_dir_all(dir.join("fake")).unwrap();
    Command::new(env!("CARGO_BIN_EXE_priory"))
        .arg("eval")
        .args(args)
        .current_dir(dir)
        .env("PATH", path)
        .env("FAKE_DIR", dir.join("fake"))
        .output()
        .unwrap()
}

#[test]
fn eval_runs_each_case_under_each_variant_and_rewrites_the_table_in_the_doc() {
    let dir = dir("doc");
    fs::write(dir.join("prompt.eval.toml"), EVAL).unwrap();
    fs::write(dir.join("prompt.txt"), "Go on.\n").unwrap();
    fs::write(dir.join("behaviour.md"), DOC).unwrap();

    let output = eval(&dir, &["prompt.eval.toml", "--runs", "2", "--budget", "1"]);

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let table = "| Case | Variant | Expected | Hits | What the agent did |\n\
                 | ---- | ------- | -------- | ---- | ------------------ |\n\
                 | done | old | result passed | 0/2 | no exit 2 |\n\
                 | done | new | result passed | 0/2 | no exit 2 |\n";
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains(table), "{stdout}");
    let doc = fs::read_to_string(dir.join("behaviour.md")).unwrap();
    assert!(doc.starts_with("# Behaviour\n\n<!-- priory eval prompt.eval.toml -->\nModel `haiku`, runs per case: 2, $0.0168 spent.\n\n"), "{doc}");
    assert!(
        doc.ends_with(&format!("{table}<!-- /priory eval -->\n\nAfter.\n")),
        "{doc}"
    );
    let typed = fs::read_to_string(dir.join("fake/handler.log")).unwrap();
    assert!(
        typed.contains("Do the step.---\nGo on. Outcomes: passed.---\n"),
        "{typed}"
    );
    let args = fs::read_to_string(dir.join("fake/handler-args.log")).unwrap();
    assert!(args.contains("--resume\nfake-session\n"), "{args}");
    assert!(args.contains("--model\nhaiku\n"), "{args}");
}

#[test]
fn eval_of_one_case_prints_its_table_and_leaves_the_doc() {
    let dir = dir("one-case");
    fs::write(dir.join("prompt.eval.toml"), EVAL).unwrap();
    fs::write(dir.join("prompt.txt"), "Go on.\n").unwrap();
    fs::write(dir.join("behaviour.md"), DOC).unwrap();

    let output = eval(
        &dir,
        &[
            "prompt.eval.toml",
            "--case",
            "done",
            "--runs",
            "1",
            "--budget",
            "0.1",
        ],
    );

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("| done | new | result passed | 0/1 | no exit 1 |")
    );
    assert_eq!(fs::read_to_string(dir.join("behaviour.md")).unwrap(), DOC);
}

#[test]
fn eval_refuses_to_run_without_a_budget() {
    let dir = dir("no-budget");
    fs::write(dir.join("prompt.eval.toml"), EVAL).unwrap();

    let output = eval(&dir, &["prompt.eval.toml"]);

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("give --budget USD"));
    assert!(!dir.join("fake/handler-args.log").exists());
}

#[test]
fn the_eval_server_serves_priorys_tools_with_one_description_replaced_and_logs_each_call() {
    let dir = dir("server");
    let log = dir.join("calls.jsonl");
    let result = json!({ "name": "result", "arguments": { "outcome": "passed" } });
    let replies = serve(
        &[
            "--eval",
            log.to_str().unwrap(),
            "--describe",
            "wait",
            "Sleep, new text.",
        ],
        &[
            json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }),
            json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": result }),
        ],
    );

    let tools = replies[0]["result"]["tools"].as_array().unwrap();
    let description =
        |name: &str| tools.iter().find(|tool| tool["name"] == name).unwrap()["description"].clone();
    assert_eq!(description("wait"), "Sleep, new text.");
    assert!(
        description("result")
            .as_str()
            .unwrap()
            .starts_with("Finish the current workflow step.")
    );
    assert_eq!(replies[1]["result"]["isError"], false);
    let logged: Value = serde_json::from_str(fs::read_to_string(&log).unwrap().trim()).unwrap();
    assert_eq!(logged, result);
}
