use crate::mcp;
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// An evaluation file: the variants of one prompt, and the cases each variant runs.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Eval {
    /// The tool whose description each variant replaces. Unset, a variant fills `{prompt}`.
    pub tool: Option<String>,
    /// The doc whose result table this evaluation rewrites, relative to the file.
    pub doc: Option<PathBuf>,
    /// Name, in an `ask` ending, the ask's shape: a short question, and which of `whatWaits` and
    /// `nextAction` it filled.
    #[serde(default)]
    pub shape: bool,
    pub variants: Vec<Variant>,
    pub cases: Vec<Case>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Variant {
    pub name: String,
    /// Set for every variant once [`load`] has read its `file`.
    pub text: Option<String>,
    /// Instead of `text`, relative to the evaluation file.
    pub file: Option<PathBuf>,
    /// Set once [`load`] has read `system_file`: appended to the agent's system prompt.
    #[serde(skip)]
    pub system: Option<String>,
    /// A file with the text to append to the agent's system prompt, relative to the evaluation file.
    pub system_file: Option<PathBuf>,
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
    /// Text for the agent's system prompt, as Claude's `--append-system-prompt`.
    pub system: Option<&'a str>,
    /// The tool description the fake MCP server replaces, as name and text.
    pub describe: Option<(&'a str, &'a str)>,
    /// The most the turn may spend, in dollars.
    pub budget: f64,
}

/// What a turn left behind: its session, to resume, its cost in dollars, and why it failed.
pub struct Spent {
    pub session: String,
    pub cost: f64,
    pub error: Option<String>,
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
    /// False when the budget ran out or a turn failed, so the table must not replace the doc's.
    pub complete: bool,
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
        complete: true,
    };
    if let Some(name) = options.case
        && !eval.cases.iter().any(|case| case.name == name)
    {
        return Err(format!("no case {name}"));
    }
    let cases = eval
        .cases
        .iter()
        .enumerate()
        .filter(|(_, case)| options.case.is_none_or(|name| case.name == name));
    for (c, case) in cases {
        for (v, variant) in eval.variants.iter().enumerate() {
            let mut did = BTreeMap::new();
            for n in 0..options.runs {
                let dir = scratch.join(format!("{c}-{v}-{n}"));
                fs::create_dir_all(&dir)
                    .map_err(|error| format!("cannot create {}: {error}", dir.display()))?;
                let Some(verdict) = one(eval, case, variant, &dir, options, &mut report, agent)?
                else {
                    break;
                };
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

/// Plays one run's turns and returns its ending, or `None` when the budget ran out first.
fn one(
    eval: &Eval,
    case: &Case,
    variant: &Variant,
    dir: &Path,
    options: &Options,
    report: &mut Report,
    agent: &mut dyn Agent,
) -> Result<Option<String>, String> {
    let text = variant.text.as_deref().unwrap_or_default();
    let prompt = case.prompt.replace("{prompt}", text);
    let turns = case
        .before
        .iter()
        .map(String::as_str)
        .chain([prompt.as_str()]);
    let mut session = None;
    let mut verdict = String::new();
    for (index, prompt) in turns.enumerate() {
        if report.spent >= options.budget {
            report.complete = false;
            return Ok(None);
        }
        let log = dir.join(format!("calls-{index}.jsonl"));
        let turn = Turn {
            prompt,
            session: session.as_deref(),
            dir,
            log: &log,
            system: variant.system.as_deref(),
            describe: eval.tool.as_deref().map(|tool| (tool, text)),
            budget: options.budget - report.spent,
        };
        let spent = agent.turn(&turn)?;
        report.spent += spent.cost;
        if report.spent > options.budget {
            report.complete = false;
        }
        if let Some(error) = spent.error {
            report.complete = false;
            return Ok(Some(format!("agent error: {error}")));
        }
        session = Some(spent.session);
        verdict = ending(&log, eval.shape)?;
        if index < case.before.len() && verdict != NO_EXIT {
            return Ok(Some(format!("{verdict} before the prompt")));
        }
    }
    Ok(Some(verdict))
}

const NO_EXIT: &str = "no exit";

/// The first `result`, `ask` or `wait` call in `log`, with its outcome or condition, or `no exit`.
/// With `shape`, an ask names its [`shape_of`].
fn ending(log: &Path, shape: bool) -> Result<String, String> {
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
            "ask" if shape => &shape_of(arguments),
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

/// What an ask's shape shows: `short` when its question is one line of at most 25 words, then the
/// optional fields it filled, `whatWaits` before `nextAction`.
pub fn shape_of(arguments: &Value) -> String {
    let filled = |field: &&str| {
        arguments[*field]
            .as_str()
            .is_some_and(|text| !text.trim().is_empty())
    };
    let short = arguments["question"].as_str().is_some_and(|question| {
        !question.trim().contains('\n') && question.split_whitespace().count() <= 25
    });
    short
        .then_some("short")
        .into_iter()
        .chain(["whatWaits", "nextAction"].iter().copied().filter(filled))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Reads the evaluation file at `path`, with each variant's `file` read into its text.
pub fn load(path: &Path) -> Result<Eval, String> {
    let unreadable = |error: &dyn std::fmt::Display| format!("{}: {error}", path.display());
    let mut eval: Eval =
        toml::from_str(&fs::read_to_string(path).map_err(|error| unreadable(&error))?)
            .map_err(|error| unreadable(&error))?;
    if eval.variants.is_empty() || eval.cases.is_empty() {
        return Err(unreadable(&"needs at least one variant and one case"));
    }
    let names = |kind: &str, names: Vec<&String>| {
        let mut seen = std::collections::BTreeSet::new();
        match names.into_iter().find(|name| !seen.insert(*name)) {
            Some(name) => Err(unreadable(&format!("two {kind} named {name}"))),
            None => Ok(()),
        }
    };
    names(
        "variants",
        eval.variants.iter().map(|variant| &variant.name).collect(),
    )?;
    names("cases", eval.cases.iter().map(|case| &case.name).collect())?;
    let dir = path.parent().unwrap_or(Path::new("."));
    for variant in &mut eval.variants {
        if let Some(file) = &variant.system_file {
            let file = dir.join(file);
            let text = fs::read_to_string(&file)
                .map_err(|error| format!("{}: {error}", file.display()))?;
            variant.system = Some(text.trim_end().to_owned());
        }
        match (&variant.text, &variant.file) {
            (Some(_), None) => {}
            (None, Some(file)) => {
                let file = dir.join(file);
                let text = fs::read_to_string(&file)
                    .map_err(|error| format!("{}: {error}", file.display()))?;
                variant.text = Some(text.trim_end().to_owned());
            }
            _ => {
                return Err(unreadable(&format!(
                    "variant {} needs one of text or file",
                    variant.name
                )));
            }
        }
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
        if report.complete {
            ""
        } else {
            " Incomplete: the budget ran out or a turn failed."
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

const TURN_TIMEOUT: Duration = Duration::from_secs(600);

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
            .args(
                turn.system
                    .iter()
                    .flat_map(|system| ["--append-system-prompt", system]),
            )
            .current_dir(turn.dir);
        for name in crate::runtime::NESTED {
            command.env_remove(name);
        }
        let output =
            crate::runtime::fed_within(&mut command, Some(turn.prompt.into()), TURN_TIMEOUT)
                .map_err(|error| format!("cannot run claude: {error}"))?
                .ok_or("claude ran past its turn timeout")?;
        let parsed: Value = serde_json::from_slice(&output.stdout).unwrap_or_default();
        let (Some(session), Some(cost)) = (
            parsed["session_id"].as_str(),
            parsed["total_cost_usd"].as_f64(),
        ) else {
            return Err(format!(
                "claude gave no session and cost: {}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ));
        };
        let error = if parsed["is_error"] == true {
            Some(parsed["subtype"].as_str().unwrap_or("error").to_owned())
        } else {
            (!output.status.success()).then(|| match output.status.code() {
                Some(code) => format!("exit {code}"),
                None => "killed".to_owned(),
            })
        };
        Ok(Spent {
            session: session.to_owned(),
            cost,
            error,
        })
    }
}

/// What one judge call answered: its text, its cost in dollars, and why it failed.
pub struct Judged {
    pub answer: String,
    pub cost: f64,
    pub error: Option<String>,
}

/// Asks `model` once, as `priory eval` runs a turn but with no tool and no MCP server at all:
/// a headless `claude -p` in the empty folder `dir`, so it reads no project settings, with
/// `system` appended to its system prompt, `prompt` on its stdin, and at most `budget` dollars.
pub fn judge(
    model: &str,
    system: &str,
    prompt: &str,
    budget: f64,
    dir: &Path,
) -> Result<Judged, String> {
    let mut command = Command::new("claude");
    command
        .args(["-p", "--output-format", "json", "--model", model])
        .args([
            "--no-session-persistence",
            "--strict-mcp-config",
            "--setting-sources",
            "project",
            "--tools",
            "",
            "--max-budget-usd",
        ])
        .arg(format!("{budget:.4}"))
        .args(["--append-system-prompt", system])
        .current_dir(dir);
    for name in crate::runtime::NESTED {
        command.env_remove(name);
    }
    let output = crate::runtime::fed_within(&mut command, Some(prompt.into()), TURN_TIMEOUT)
        .map_err(|error| format!("cannot run claude: {error}"))?
        .ok_or("claude ran past its timeout")?;
    let parsed: Value = serde_json::from_slice(&output.stdout).unwrap_or_default();
    let Some(cost) = parsed["total_cost_usd"].as_f64() else {
        return Err(format!(
            "claude gave no cost: {}{}",
            String::from_utf8_lossy(&output.stdout).trim(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    };
    let error = if parsed["is_error"] == true {
        Some(parsed["subtype"].as_str().unwrap_or("error").to_owned())
    } else {
        (!output.status.success()).then(|| match output.status.code() {
            Some(code) => format!("exit {code}"),
            None => "killed".to_owned(),
        })
    };
    Ok(Judged {
        answer: parsed["result"].as_str().unwrap_or_default().to_owned(),
        cost,
        error,
    })
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
    mcp::serve_with(&tools, &mcp::instructions(), &|params| {
        Ok(mcp::content(record(log, params)))
    })
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

    /// Calls result passed after a prompt that says "clear", and nothing otherwise, for 0.01 a
    /// turn. With `fail`, every turn fails as Claude does past its budget.
    #[derive(Default)]
    struct Fake {
        prompts: Vec<String>,
        systems: Vec<Option<String>>,
        fail: bool,
    }

    impl Agent for Fake {
        fn turn(&mut self, turn: &Turn) -> Result<Spent, String> {
            self.prompts.push(turn.prompt.to_owned());
            self.systems.push(turn.system.map(str::to_owned));
            if turn.prompt.contains("clear") {
                let call = json!({ "name": "result", "arguments": { "outcome": "passed" } });
                record(turn.log, &call)?;
            }
            Ok(Spent {
                session: "s".to_owned(),
                cost: 0.01,
                error: self.fail.then(|| "error_max_budget_usd".to_owned()),
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

    fn report(name: &str, fake: &mut Fake, runs: usize, budget: f64) -> Report {
        let dir = scratch(name);
        let options = Options {
            runs,
            budget,
            case: None,
        };
        let report = run(&nudge(), &dir, &options, fake).unwrap();
        fs::remove_dir_all(dir).unwrap();
        report
    }

    #[test]
    fn a_run_counts_what_the_agent_did_after_the_prompt_under_test_per_variant() {
        let mut fake = Fake::default();

        let report = report("count", &mut fake, 2, 1.0);

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
        assert!(report.complete);
        assert_eq!(fake.prompts[..2], ["Do the step.", "Go on. End it."]);
    }

    #[test]
    fn a_variants_system_file_reaches_every_turn_of_its_runs_and_no_other_variants() {
        let dir = scratch("system");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("guide.md"), "Lead with the question.\n").unwrap();
        fs::write(
            dir.join("a.eval.toml"),
            r#"
            variants = [{ name = "bare", text = "" }, { name = "guided", text = "", system_file = "guide.md" }]
            [[cases]]
            name = "c"
            before = ["Set the scene."]
            prompt = "Go."
            expect = "no exit"
            "#,
        )
        .unwrap();
        let eval = load(&dir.join("a.eval.toml")).unwrap();
        let mut fake = Fake::default();
        let options = Options {
            runs: 1,
            budget: 1.0,
            case: None,
        };

        run(&eval, &dir.join("out"), &options, &mut fake).unwrap();

        let guide = Some("Lead with the question.".to_owned());
        assert_eq!(fake.systems, [None, None, guide.clone(), guide]);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn with_shape_an_ask_ending_names_which_of_what_waits_and_next_action_it_filled() {
        let dir = scratch("shape");
        fs::create_dir_all(&dir).unwrap();
        let log = dir.join("calls.jsonl");
        let ask = json!({ "name": "ask", "arguments": { "question": "Q?", "nextAction": "Then I ship." } });
        record(&log, &ask).unwrap();

        assert_eq!(ending(&log, true).unwrap(), "ask short nextAction");
        assert_eq!(ending(&log, false).unwrap(), "ask");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_run_starts_no_turn_once_the_budget_is_spent_and_drops_the_unfinished_run() {
        let mut fake = Fake::default();

        let report = report("budget", &mut fake, 2, 0.03);

        assert!(!report.complete);
        assert_eq!(fake.prompts.len(), 3);
        assert_eq!(
            report.tallies[0].did,
            BTreeMap::from([("no exit".to_owned(), 1)])
        );
        assert!(report.tallies[1].did.is_empty());
    }

    #[test]
    fn a_last_turn_that_overspends_leaves_the_report_incomplete() {
        let report = report("overspend", &mut Fake::default(), 1, 0.035);

        assert!(!report.complete);
        assert_eq!(report.tallies[1].did.values().sum::<usize>(), 1);
    }

    #[test]
    fn load_reads_an_empty_text_and_refuses_text_with_file() {
        let dir = scratch("empty-text");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("p.txt"), "From file.\n").unwrap();
        let file = dir.join("x.eval.toml");
        let case = "[[cases]]\nname = \"a\"\nprompt = \"p\"\nexpect = \"ask\"\n";

        fs::write(
            &file,
            format!("variants = [{{ name = \"none\", text = \"\" }}]\n{case}"),
        )
        .unwrap();
        assert_eq!(load(&file).unwrap().variants[0].text.as_deref(), Some(""));
        fs::write(
            &file,
            format!("variants = [{{ name = \"both\", text = \"\", file = \"p.txt\" }}]\n{case}"),
        )
        .unwrap();
        assert!(
            load(&file)
                .unwrap_err()
                .contains("needs one of text or file")
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_failed_turn_counts_as_an_agent_error_and_leaves_the_report_incomplete() {
        let mut fake = Fake {
            fail: true,
            ..Fake::default()
        };

        let report = report("failed", &mut fake, 1, 1.0);

        assert!(!report.complete);
        assert_eq!(
            report.tallies[1].did,
            BTreeMap::from([("agent error: error_max_budget_usd".to_owned(), 1)])
        );
        assert_eq!(fake.prompts, ["Do the step.", "Do the step."]);
    }

    #[test]
    fn load_refuses_duplicate_names_and_empty_lists() {
        let dir = scratch("load");
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("x.eval.toml");
        let case = "[[cases]]\nname = \"a\"\nprompt = \"p\"\nexpect = \"ask\"\n";
        for (text, error) in [
            (
                format!(
                    "variants = [{{ name = \"v\", text = \"t\" }}, {{ name = \"v\", text = \"u\" }}]\n{case}"
                ),
                "two variants named v",
            ),
            (
                format!("variants = [{{ name = \"v\", text = \"t\" }}]\n{case}{case}"),
                "two cases named a",
            ),
            (
                "variants = []\ncases = []\n".to_owned(),
                "needs at least one variant and one case",
            ),
        ] {
            fs::write(&file, text).unwrap();
            let refused = load(&file).unwrap_err();
            assert!(refused.contains(error), "{refused}");
        }
        fs::remove_dir_all(dir).unwrap();
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
            let _ = ending(&log, true);
            fs::remove_file(log).unwrap();
        }

        #[test]
        fn load_reads_any_file_without_panicking(text in "[ -~\n]{0,200}") {
            let file = scratch("load-any").with_extension("toml");
            fs::write(&file, text).unwrap();
            let _ = load(&file);
            fs::remove_file(file).unwrap();
        }
    }
}
