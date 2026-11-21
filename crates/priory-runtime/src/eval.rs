use crate::mcp;
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// An evaluation file: the variants of one prompt, and the cases each variant runs.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Eval {
    /// The tool whose description each variant replaces. Unset, a variant fills `{prompt}`.
    pub tool: Option<String>,
    /// The doc whose result table this evaluation rewrites, relative to the file.
    pub doc: Option<PathBuf>,
    pub variants: Vec<Variant>,
    pub cases: Vec<Case>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Variant {
    pub name: String,
    pub text: Option<String>,
    /// A file that holds the text, relative to the evaluation file.
    pub file: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Case {
    pub name: String,
    /// The prompts typed before the one under test, each a turn.
    #[serde(default)]
    pub before: Vec<String>,
    /// The prompt under test; `{prompt}` stands for the variant's text.
    pub prompt: String,
    /// What a run should do, as the table counts it, such as `result passed`.
    pub expect: String,
}

/// One turn an agent takes in an evaluation run.
pub struct Turn<'a> {
    pub prompt: &'a str,
    /// The session to resume; `None` starts one.
    pub session: Option<&'a str>,
    /// The run's own folder, where the agent runs.
    pub dir: &'a Path,
    /// Where the fake MCP server logs the run's tool calls.
    pub log: &'a Path,
    /// The tool description the fake MCP server replaces, as name and text.
    pub describe: Option<(&'a str, &'a str)>,
    /// The most the turn may spend, in dollars.
    pub budget: f64,
}

/// What a turn left behind: its session, to resume, and what it cost in dollars.
pub struct Spent {
    pub session: String,
    pub cost: f64,
}

pub trait Agent {
    fn turn(&mut self, turn: &Turn) -> Result<Spent, String>;
}

/// The runs of one case under one variant, counted by what the agent did.
#[derive(Debug, PartialEq)]
pub struct Tally {
    pub case: String,
    pub variant: String,
    pub expect: String,
    pub did: BTreeMap<String, usize>,
}

#[derive(Debug, PartialEq)]
pub struct Report {
    pub tallies: Vec<Tally>,
    pub spent: f64,
    /// Whether the budget ran out before every run.
    pub stopped: bool,
}

pub struct Options<'a> {
    pub runs: usize,
    pub budget: f64,
    /// Only this case, when set.
    pub case: Option<&'a str>,
}

/// Runs each case `options.runs` times under each variant, in folders under `scratch`.
pub fn run(
    eval: &Eval,
    scratch: &Path,
    options: &Options,
    agent: &mut dyn Agent,
) -> Result<Report, String> {
    let mut report = Report {
        tallies: Vec::new(),
        spent: 0.0,
        stopped: false,
    };
    let cases = eval
        .cases
        .iter()
        .filter(|case| options.case.is_none_or(|name| case.name == name));
    for case in cases {
        for variant in &eval.variants {
            let text = variant.text.as_deref().unwrap_or_default();
            let mut did = BTreeMap::new();
            for n in 0..options.runs {
                if report.spent >= options.budget {
                    report.stopped = true;
                    break;
                }
                let dir = scratch
                    .join(&case.name)
                    .join(&variant.name)
                    .join(n.to_string());
                fs::create_dir_all(&dir)
                    .map_err(|error| format!("cannot create {}: {error}", dir.display()))?;
                let describe = eval.tool.as_deref().map(|tool| (tool, text));
                let prompt = case.prompt.replace("{prompt}", text);
                let turns = case
                    .before
                    .iter()
                    .map(String::as_str)
                    .chain([prompt.as_str()]);
                let mut session = None;
                let mut verdict = String::new();
                for (index, prompt) in turns.enumerate() {
                    let log = dir.join(format!("calls-{index}.jsonl"));
                    let turn = Turn {
                        prompt,
                        session: session.as_deref(),
                        dir: &dir,
                        log: &log,
                        describe,
                        budget: options.budget - report.spent,
                    };
                    let spent = agent.turn(&turn)?;
                    report.spent += spent.cost;
                    session = Some(spent.session);
                    verdict = ending(&log)?;
                    if index < case.before.len() && verdict != NO_EXIT {
                        verdict = format!("{verdict} before the prompt");
                        break;
                    }
                }
                *did.entry(verdict).or_default() += 1;
            }
            report.tallies.push(Tally {
                case: case.name.clone(),
                variant: variant.name.clone(),
                expect: case.expect.clone(),
                did,
            });
        }
    }
    Ok(report)
}

const NO_EXIT: &str = "no exit";

/// The first `result`, `ask` or `wait` call in `log`, with its outcome or condition, or `no exit`.
fn ending(log: &Path) -> Result<String, String> {
    let text = match fs::read_to_string(log) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(format!("cannot read {}: {error}", log.display())),
    };
    for line in text.lines() {
        let call: Value = serde_json::from_str(line)
            .map_err(|error| format!("unreadable call {line:?}: {error}"))?;
        let arguments = &call["arguments"];
        let detail = match call["name"].as_str().unwrap_or_default() {
            "result" => arguments["outcome"].as_str().unwrap_or("without outcome"),
            "ask" => "",
            "wait" => arguments["on"]["type"].as_str().unwrap_or("any"),
            _ => continue,
        };
        return Ok(
            format!("{} {detail}", call["name"].as_str().unwrap_or_default())
                .trim_end()
                .to_owned(),
        );
    }
    Ok(NO_EXIT.to_owned())
}

/// Reads the evaluation file at `path`, with each variant's `file` read into its text.
pub fn load(path: &Path) -> Result<Eval, String> {
    let unreadable = |error: &dyn std::fmt::Display| format!("{}: {error}", path.display());
    let mut eval: Eval =
        toml::from_str(&fs::read_to_string(path).map_err(|error| unreadable(&error))?)
            .map_err(|error| unreadable(&error))?;
    let dir = path.parent().unwrap_or(Path::new("."));
    for variant in &mut eval.variants {
        variant.text = match (variant.text.take(), &variant.file) {
            (Some(text), None) => Some(text),
            (None, Some(file)) => {
                let file = dir.join(file);
                let text = fs::read_to_string(&file)
                    .map_err(|error| format!("{}: {error}", file.display()))?;
                Some(text.trim_end().to_owned())
            }
            _ => {
                return Err(unreadable(&format!(
                    "variant {} needs one of text or file",
                    variant.name
                )));
            }
        };
    }
    if let Some(doc) = &mut eval.doc {
        *doc = dir.join(&*doc);
    }
    Ok(eval)
}

/// The report as a Markdown table, one row per case and variant, under a line that names the model.
pub fn table(report: &Report, model: &str, runs: usize) -> String {
    let mut table = format!(
        "Model `{model}`, runs per case: {runs}, ${:.4} spent.{}\n\n\
         | Case | Variant | Expected | Hits | What the agent did |\n\
         | ---- | ------- | -------- | ---- | ------------------ |\n",
        report.spent,
        if report.stopped {
            " The budget ran out before every run."
        } else {
            ""
        },
    );
    for tally in &report.tallies {
        let total: usize = tally.did.values().sum();
        let hits = tally.did.get(&tally.expect).copied().unwrap_or_default();
        let mut did: Vec<_> = tally.did.iter().collect();
        did.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        let did: Vec<String> = did
            .iter()
            .map(|(what, count)| format!("{what} {count}"))
            .collect();
        table += &format!(
            "| {} | {} | {} | {hits}/{total} | {} |\n",
            tally.case,
            tally.variant,
            tally.expect,
            did.join(", ")
        );
    }
    table
}

/// Replaces the lines between `<!-- priory eval NAME -->` and `<!-- /priory eval -->` in `doc`.
pub fn rewrite(doc: &Path, name: &str, table: &str) -> Result<(), String> {
    let text = fs::read_to_string(doc).map_err(|error| format!("{}: {error}", doc.display()))?;
    let open = format!("<!-- priory eval {name} -->\n");
    let close = "<!-- /priory eval -->";
    let start = text
        .find(&open)
        .map(|at| at + open.len())
        .ok_or_else(|| format!("{} has no line {}", doc.display(), open.trim_end()))?;
    let end = text[start..]
        .find(close)
        .map(|at| start + at)
        .ok_or_else(|| {
            format!(
                "{} has no line {close} after {}",
                doc.display(),
                open.trim_end()
            )
        })?;
    fs::write(doc, format!("{}{table}{}", &text[..start], &text[end..]))
        .map_err(|error| format!("{}: {error}", doc.display()))
}

/// Runs each turn as a headless `claude -p` of `model`, whose only tools are the fake server's.
pub struct Claude<'a> {
    pub model: &'a str,
}

impl Agent for Claude<'_> {
    fn turn(&mut self, turn: &Turn) -> Result<Spent, String> {
        let exe = std::env::current_exe().map_err(|error| error.to_string())?;
        let mut server = vec![
            "mcp".to_owned(),
            "--eval".to_owned(),
            turn.log.display().to_string(),
        ];
        if let Some((tool, text)) = turn.describe {
            server.extend(["--describe".to_owned(), tool.to_owned(), text.to_owned()]);
        }
        let config =
            serde_json::json!({ "mcpServers": { "priory": { "command": exe, "args": server } } });
        let mut command = Command::new("claude");
        command
            .args(["-p", "--output-format", "json", "--model", self.model])
            .args([
                "--strict-mcp-config",
                "--setting-sources",
                "project",
                "--mcp-config",
            ])
            .arg(config.to_string())
            .args([
                "--tools",
                "",
                "--allowedTools",
                "mcp__priory",
                "--max-budget-usd",
            ])
            .arg(format!("{:.4}", turn.budget))
            .args(
                turn.session
                    .iter()
                    .flat_map(|session| ["--resume", session]),
            )
            .current_dir(turn.dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for name in crate::runtime::NESTED {
            command.env_remove(name);
        }
        let mut child = command
            .spawn()
            .map_err(|error| format!("cannot start claude: {error}"))?;
        let mut stdin = child.stdin.take().expect("stdin is piped");
        stdin
            .write_all(turn.prompt.as_bytes())
            .map_err(|error| format!("cannot type the prompt: {error}"))?;
        drop(stdin);
        let output = child
            .wait_with_output()
            .map_err(|error| error.to_string())?;
        // A turn over its budget still reports its session and cost, so its status is not checked.
        let parsed: Value = serde_json::from_slice(&output.stdout).unwrap_or_default();
        match (
            parsed["session_id"].as_str(),
            parsed["total_cost_usd"].as_f64(),
        ) {
            (Some(session), Some(cost)) => Ok(Spent {
                session: session.to_owned(),
                cost,
            }),
            _ => Err(format!(
                "claude gave no session and cost: {}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )),
        }
    }
}

/// Serves Priory's agent tools as a fake for an evaluation: it appends each `tools/call` params
/// object to `log`, one JSON line, and answers `ok`. `describe` replaces one tool's description.
pub fn serve(log: &Path, describe: Option<(&str, &str)>) -> Result<(), String> {
    let mut tools = mcp::tools();
    if let Some((name, text)) = describe {
        let tool = tools
            .as_array_mut()
            .and_then(|tools| tools.iter_mut().find(|tool| tool["name"] == name))
            .ok_or_else(|| format!("no tool {name}"))?;
        tool["description"] = text.into();
    }
    mcp::serve_with(&tools, &|params| mcp::content(record(log, params)))
}

fn record(log: &Path, params: &Value) -> Result<String, String> {
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(log)
        .and_then(|mut file| writeln!(file, "{params}"))
        .map(|()| "ok".to_owned())
        .map_err(|error| format!("cannot log the call: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Calls result passed after a prompt that says "clear", and nothing otherwise, for 0.01 a turn.
    struct Fake {
        prompts: Vec<String>,
    }

    impl Agent for Fake {
        fn turn(&mut self, turn: &Turn) -> Result<Spent, String> {
            self.prompts.push(turn.prompt.to_owned());
            if turn.prompt.contains("clear") {
                let call = json!({ "name": "result", "arguments": { "outcome": "passed" } });
                record(turn.log, &call)?;
            }
            Ok(Spent {
                session: "s".to_owned(),
                cost: 0.01,
            })
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("priory-eval-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn nudge() -> Eval {
        toml::from_str(
            r#"
            variants = [{ name = "old", text = "Go on." }, { name = "new", text = "Be clear." }]
            [[cases]]
            name = "done"
            before = ["Do the step."]
            prompt = "{prompt} End it."
            expect = "result passed"
            "#,
        )
        .unwrap()
    }

    #[test]
    fn a_run_counts_what_the_agent_did_after_the_prompt_under_test_per_variant() {
        let eval = nudge();
        let mut fake = Fake {
            prompts: Vec::new(),
        };
        let options = Options {
            runs: 2,
            budget: 1.0,
            case: None,
        };

        let dir = scratch("count");
        let report = run(&eval, &dir, &options, &mut fake).unwrap();
        fs::remove_dir_all(dir).unwrap();

        let tally = |variant: &str, did: &str| Tally {
            case: "done".to_owned(),
            variant: variant.to_owned(),
            expect: "result passed".to_owned(),
            did: BTreeMap::from([(did.to_owned(), 2)]),
        };
        assert_eq!(
            report.tallies,
            [tally("old", "no exit"), tally("new", "result passed")]
        );
        assert!(!report.stopped);
        assert_eq!(fake.prompts[..2], ["Do the step.", "Go on. End it."]);
    }

    #[test]
    fn a_run_stops_starting_runs_once_the_budget_is_spent() {
        let mut fake = Fake {
            prompts: Vec::new(),
        };
        let options = Options {
            runs: 2,
            budget: 0.03,
            case: None,
        };

        let dir = scratch("budget");
        let report = run(&nudge(), &dir, &options, &mut fake).unwrap();
        fs::remove_dir_all(dir).unwrap();

        assert!(report.stopped);
        assert_eq!(fake.prompts.len(), 4);
        assert_eq!(
            report.tallies[0].did,
            BTreeMap::from([("no exit".to_owned(), 2)])
        );
        assert!(report.tallies[1].did.is_empty());
    }

    proptest::proptest! {
        #[test]
        fn ending_reads_any_call_log_without_panicking(
            name in proptest::sample::select(vec!["result", "ask", "wait", "search", ""]),
            arguments in "[ -~]{0,40}",
            junk in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..64),
        ) {
            let log = scratch("ending").with_extension("jsonl");
            let call = format!("{{\"name\":\"{name}\",\"arguments\":{arguments}}}");
            let mut text = format!("{call}\n").into_bytes();
            text.extend(junk);
            fs::write(&log, text).unwrap();
            let _ = ending(&log);
            fs::remove_file(log).unwrap();
        }
    }
}
