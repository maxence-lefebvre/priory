//! `priory bench FILE --base REF --head REF`: compares two configurations of a factory on a
//! benchmark's cases. It replays each case in pairs, one run per side, and grades each replay with
//! the benchmark's scorers. [`table`] turns those pairs into the comparison's table and its one
//! verdict, from an exact sign test on the primary scorer. docs/daemon/bench.md describes it.

use crate::judge;
use crate::replay::{self, Ending};
use crate::score::{self, Score, Verdict};
use crate::trace::{self, Trace};
use priory_factory::case;
use priory_factory::factory::{self, Factory, ScoreKind, ScoreOn};
use priory_factory::{plural, rules};
use priory_runtime::{room, transcript};
use rusqlite::{Connection, TransactionBehavior};
use serde_json::json;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

/// One replay of one case under one side, with each scorer's verdict on it.
#[derive(Clone, Debug)]
pub struct Played {
    pub ending: Ending,
    pub cost: f64,
    pub took: Duration,
    /// Each scorer's verdict, by name. A scorer with no verdict scored no run of the replay.
    pub verdicts: BTreeMap<String, Verdict>,
}

/// One run of each side on the same case.
#[derive(Clone, Debug)]
pub struct Pair {
    pub base: Played,
    pub head: Played,
}

/// The pairs one case got.
#[derive(Clone, Debug)]
pub struct CaseRuns {
    pub name: String,
    /// Whether the capture left gaps in the case.
    pub partial: bool,
    pub pairs: Vec<Pair>,
    /// The pairs the budget cut, each dropped whole.
    pub dropped: usize,
}

/// What a comparison ran and what it saw.
#[derive(Clone, Debug)]
pub struct Comparison {
    pub benchmark: String,
    pub base: String,
    pub head: String,
    /// This comparison's number among the comparisons of the same commits and benchmark.
    pub number: usize,
    /// What the number counts in, such as `these commits`.
    pub of: String,
    /// Whether an earlier complete comparison of the same commits and benchmark gave the verdict.
    pub repeat: bool,
    /// Whether the change touches `scorers/` or `benchmarks/`, so it moved the ruler too.
    pub ruler_moved: bool,
    /// Why the comparison stopped short, when it did.
    pub incomplete: Option<String>,
    /// The pairs each case was to get.
    pub runs: usize,
    pub spent: f64,
    pub budget: f64,
    /// The scorers of the table, the primary one first, with what each reads.
    pub scorers: Vec<(String, ScoreOn)>,
    pub cases: Vec<CaseRuns>,
    /// The cases a `command` scorer could not score on their start commit, each with why.
    pub broken: Vec<(String, String)>,
    /// One line per judge scorer: its agreement, or why it was left out on both sides.
    pub judges: Vec<String>,
}

/// The primary scorer's verdict.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Call {
    Better,
    Worse,
    NoClearDifference,
    /// Fewer than 6 cases did not tie, so no split could reach the threshold.
    TooFewCases,
}

impl Call {
    pub fn name(self) -> &'static str {
        match self {
            Call::Better => "better",
            Call::Worse => "worse",
            Call::NoClearDifference => "no clear difference",
            Call::TooFewCases => "too few cases",
        }
    }
}

/// The least number of cases that did not tie for which a sign test can reach 0.05.
pub const LEAST_UNTIED: usize = 6;

/// The two-sided probability of a split at least as uneven as `won` against `lost`, when each
/// untied case is as likely to go either way: an exact sign test.
pub fn sign_test(won: usize, lost: usize) -> f64 {
    let n = won + lost;
    if n == 0 {
        return 1.0;
    }
    let mut term = 0.5f64.powi(n as i32);
    let mut tail = 0.0;
    for k in 0..=won.min(lost) {
        tail += term;
        term = term * (n - k) as f64 / (k + 1) as f64;
    }
    (2.0 * tail).min(1.0)
}

/// The verdict of `won` against `lost` cases: `better` or `worse` when the sign test is under 0.05.
pub fn call(won: usize, lost: usize) -> Call {
    if won + lost < LEAST_UNTIED {
        return Call::TooFewCases;
    }
    match sign_test(won, lost) < 0.05 {
        true if won > lost => Call::Better,
        true => Call::Worse,
        false => Call::NoClearDifference,
    }
}

/// One scorer's counts over every case.
#[derive(Debug, PartialEq)]
pub struct Row {
    pub scorer: String,
    /// Each side's passes, the replays that count, and the errors.
    pub base: (usize, usize, usize),
    pub head: (usize, usize, usize),
    /// The mean over the compared cases of the head's pass share less the base's, in points.
    pub mean: Option<f64>,
    pub won: usize,
    pub tied: usize,
    pub lost: usize,
}

/// Whether `played` counts as passed for a scorer that reads `on`, or `None` when it does not
/// count. A replay that ended with no result counts as not passed for a `result` or `diff`
/// scorer, so a change cannot raise its share by asking more. An `ask` scorer counts only the
/// replays that asked.
fn passed(played: &Played, scorer: &str, on: ScoreOn) -> Option<bool> {
    let verdict = played.verdicts.get(scorer).copied();
    match on {
        ScoreOn::Ask => match verdict {
            None | Some(Verdict::NotApplicable) => None,
            Some(verdict) => Some(verdict == Verdict::Pass),
        },
        _ => Some(
            matches!(played.ending, Ending::Reached(_) | Ending::Stopped)
                && verdict == Some(Verdict::Pass),
        ),
    }
}

/// Counts one side of a case: its passes, the replays that count, and its errors.
fn side<'a>(
    runs: impl Iterator<Item = &'a Played>,
    scorer: &str,
    on: ScoreOn,
) -> (usize, usize, usize) {
    let (mut passes, mut counted, mut errors) = (0, 0, 0);
    for played in runs {
        if let Some(pass) = passed(played, scorer, on) {
            counted += 1;
            passes += usize::from(pass);
            errors += usize::from(played.verdicts.get(scorer) == Some(&Verdict::Error));
        }
    }
    (passes, counted, errors)
}

/// The counts of one scorer over the cases. Each case gives one difference: the share of the
/// head's replays that passed, less the base's. A case where a side has no replay that counts is
/// left out of the difference.
pub fn row(cases: &[CaseRuns], scorer: &str, on: ScoreOn) -> Row {
    let mut row = Row {
        scorer: scorer.to_owned(),
        base: (0, 0, 0),
        head: (0, 0, 0),
        mean: None,
        won: 0,
        tied: 0,
        lost: 0,
    };
    let mut differences = Vec::new();
    for case in cases {
        let base = side(case.pairs.iter().map(|pair| &pair.base), scorer, on);
        let head = side(case.pairs.iter().map(|pair| &pair.head), scorer, on);
        for (total, part) in [(&mut row.base, base), (&mut row.head, head)] {
            total.0 += part.0;
            total.1 += part.1;
            total.2 += part.2;
        }
        if base.1 == 0 || head.1 == 0 {
            continue;
        }
        let difference = head.0 as f64 / head.1 as f64 - base.0 as f64 / base.1 as f64;
        match difference {
            d if d > 1e-9 => row.won += 1,
            d if d < -1e-9 => row.lost += 1,
            _ => row.tied += 1,
        }
        differences.push(difference);
    }
    if !differences.is_empty() {
        row.mean = Some(100.0 * differences.iter().sum::<f64>() / differences.len() as f64);
    }
    row
}

/// The primary scorer's verdict, or `None` when the comparison carries none: it is incomplete or
/// a repeat.
pub fn verdict(comparison: &Comparison) -> Option<Call> {
    if comparison.incomplete.is_some() || comparison.repeat {
        return None;
    }
    let (primary, on) = comparison.scorers.first()?;
    let row = row(&comparison.cases, primary, *on);
    Some(call(row.won, row.lost))
}

/// A budget in dollars, with cents unless they are zero: `$40`, `$31.20`.
fn money(dollars: f64) -> String {
    let cents = (dollars * 100.0).round();
    match cents % 100.0 == 0.0 {
        true => format!("${:.0}", cents / 100.0),
        false => format!("${:.2}", cents / 100.0),
    }
}

fn minutes(took: Duration) -> String {
    match took.as_secs() {
        seconds if seconds < 60 => format!("{seconds} s"),
        seconds => format!("{} min", (seconds as f64 / 60.0).round()),
    }
}

fn median(mut values: Vec<f64>) -> Option<f64> {
    values.sort_by(f64::total_cmp);
    let middle = values.len() / 2;
    match values.len() {
        0 => None,
        n if n % 2 == 1 => Some(values[middle]),
        _ => Some((values[middle - 1] + values[middle]) / 2.0),
    }
}

/// An ending's column in the table: its kind, with the step for `reached`, ranked in the order
/// the endings are listed.
fn column(ending: &Ending) -> (u8, String) {
    match ending {
        Ending::Reached(step) => (0, format!("reached {step}")),
        Ending::Stopped => (1, "stopped".to_owned()),
        Ending::Asked => (2, "asked".to_owned()),
        Ending::Waited => (3, "waited".to_owned()),
        Ending::TimedOut => (4, "timed out".to_owned()),
        Ending::OverBudget => (5, "over budget".to_owned()),
        Ending::AgentError(_) => (6, "agent error".to_owned()),
        Ending::CostUnknown(_) => (7, "cost unknown".to_owned()),
    }
}

/// Lays `rows` out in columns two spaces apart, the last one unpadded.
fn columns(rows: &[Vec<String>]) -> String {
    let count = rows.iter().map(Vec::len).max().unwrap_or(0);
    let widths: Vec<usize> = (0..count)
        .map(|at| {
            rows.iter()
                .filter_map(|row| row.get(at))
                .map(|cell| cell.chars().count())
                .max()
                .unwrap_or(0)
        })
        .collect();
    let mut text = String::new();
    for row in rows {
        let mut line = String::new();
        for (at, cell) in row.iter().enumerate() {
            line.push_str(cell);
            if at + 1 < row.len() {
                let pad = widths[at] + 2 - cell.chars().count();
                line.push_str(&" ".repeat(pad));
            }
        }
        text.push_str(line.trim_end());
        text.push('\n');
    }
    text
}

fn short(sha: &str) -> &str {
    sha.get(..7).unwrap_or(sha)
}

/// The comparison's table: its header, one row per scorer, and each side's endings, median cost
/// and median time.
pub fn table(comparison: &Comparison) -> String {
    let mut text = String::new();
    if let Some(why) = &comparison.incomplete {
        text.push_str(&format!(
            "Incomplete: {why}, so this table gives no verdict\n"
        ));
    }
    if comparison.ruler_moved {
        text.push_str("The change touches scorers/ or benchmarks/, so it moved the ruler too\n");
    }
    text.push_str(&format!(
        "benchmark {}, base {}, head {}, comparison {} of {}",
        comparison.benchmark,
        short(&comparison.base),
        short(&comparison.head),
        comparison.number,
        comparison.of
    ));
    if comparison.repeat {
        text.push_str(", a repeat: the first complete comparison gave the verdict");
    }
    text.push('\n');
    let dropped: usize = comparison.cases.iter().map(|case| case.dropped).sum();
    text.push_str(&format!(
        "{}, {} each, {} dropped, ${:.2} of {}\n",
        plural(comparison.cases.len(), "case", "cases"),
        plural(comparison.runs, "pair", "pairs"),
        plural(dropped, "pair", "pairs"),
        comparison.spent,
        money(comparison.budget)
    ));
    let partial: Vec<&str> = comparison
        .cases
        .iter()
        .filter(|case| case.partial)
        .map(|case| case.name.as_str())
        .collect();
    if !partial.is_empty() {
        text.push_str(&format!("partial cases: {}\n", partial.join(", ")));
    }
    for (case, why) in &comparison.broken {
        text.push_str(&format!("dropped case {case}: {why}\n"));
    }
    for line in &comparison.judges {
        text.push_str(line);
        text.push('\n');
    }

    let mut rows = vec![
        [
            "scorer",
            "base",
            "errors",
            "head",
            "errors",
            "mean diff",
            "won",
            "tied",
            "lost",
            "verdict",
        ]
        .map(str::to_owned)
        .to_vec(),
    ];
    let verdict = verdict(comparison);
    for (at, (scorer, on)) in comparison.scorers.iter().enumerate() {
        let row = row(&comparison.cases, scorer, *on);
        let shown = match (at, verdict) {
            (0, Some(call)) => call.name(),
            (0, None) if comparison.incomplete.is_none() => "repeat",
            _ => "",
        };
        rows.push(vec![
            scorer.clone(),
            format!("{}/{}", row.base.0, row.base.1),
            row.base.2.to_string(),
            format!("{}/{}", row.head.0, row.head.1),
            row.head.2.to_string(),
            row.mean
                .map_or_else(|| "-".to_owned(), |mean| format!("{mean:+.1}")),
            row.won.to_string(),
            row.tied.to_string(),
            row.lost.to_string(),
            shown.to_owned(),
        ]);
    }
    text.push('\n');
    text.push_str(&columns(&rows));

    let sides: [Vec<&Played>; 2] = [
        comparison
            .cases
            .iter()
            .flat_map(|case| case.pairs.iter().map(|pair| &pair.base))
            .collect(),
        comparison
            .cases
            .iter()
            .flat_map(|case| case.pairs.iter().map(|pair| &pair.head))
            .collect(),
    ];
    let mut endings: Vec<(u8, String)> = sides
        .iter()
        .flatten()
        .map(|played| column(&played.ending))
        .collect();
    endings.sort();
    endings.dedup();
    let mut header = vec!["side".to_owned()];
    header.extend(endings.iter().map(|(_, name)| name.clone()));
    header.extend(["median cost".to_owned(), "median time".to_owned()]);
    let mut rows = vec![header];
    for (name, runs) in ["base", "head"].iter().zip(&sides) {
        let mut row = vec![(*name).to_owned()];
        for ending in &endings {
            let count = runs
                .iter()
                .filter(|played| column(&played.ending) == *ending)
                .count();
            row.push(count.to_string());
        }
        let cost = median(runs.iter().map(|played| played.cost).collect());
        let took = median(
            runs.iter()
                .map(|played| played.took.as_secs_f64())
                .collect(),
        );
        row.push(cost.map_or_else(|| "-".to_owned(), |cost| format!("${cost:.2}")));
        row.push(took.map_or_else(
            || "-".to_owned(),
            |took| minutes(Duration::from_secs_f64(took)),
        ));
        rows.push(row);
    }
    text.push('\n');
    text.push_str(&columns(&rows));
    text
}

/// One verdict per scorer for one replay, from the scores of its finished runs. A `result` or
/// `diff` scorer takes its verdict on the replay's last finished run. An `ask` scorer takes the
/// worst of its verdicts on every run that asked: an error, then a fail, then a pass.
pub fn verdicts(scores: &[Score], scorers: &[(String, ScoreOn)]) -> BTreeMap<String, Verdict> {
    let mut verdicts = BTreeMap::new();
    for (name, on) in scorers {
        let mine = scores.iter().filter(|score| score.scorer == *name);
        let verdict = match on {
            ScoreOn::Ask => {
                let seen: Vec<Verdict> = mine.map(|score| score.verdict).collect();
                [Verdict::Error, Verdict::Fail, Verdict::Pass]
                    .into_iter()
                    .find(|worst| seen.contains(worst))
                    .or_else(|| (!seen.is_empty()).then_some(Verdict::NotApplicable))
            }
            _ => mine
                .max_by_key(|score| score.run)
                .map(|score| score.verdict),
        };
        if let Some(verdict) = verdict {
            verdicts.insert(name.clone(), verdict);
        }
    }
    verdicts
}

pub struct Options {
    /// The pull request whose comparison this is, `owner/name#12`, when `--post` names one.
    pub post: Option<String>,
    /// The most the comparison may spend, in dollars.
    pub budget: f64,
    /// The pairs each case gets.
    pub runs: usize,
    /// The one case to run, by its folder's name.
    pub case: Option<String>,
    /// The longest one turn may run.
    pub turn: Duration,
}

fn io(path: &Path) -> impl Fn(std::io::Error) -> String + '_ {
    move |error| format!("{}: {error}", path.display())
}

/// Runs git in `dir` and returns what it printed, trimmed.
fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("git: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// The comparison's scratch folder, which holds a checkout of each side. Dropping it removes it.
struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        priory_runtime::runtime::remove_sealed(&self.0);
    }
}

/// Writes the files of `commit` of the repository `top` into the new folder `into`.
fn export(top: &Path, commit: &str, into: &Path) -> Result<(), String> {
    fs::create_dir_all(into).map_err(io(into))?;
    let mut archive = Command::new("git")
        .arg("-C")
        .arg(top)
        .args(["archive", "--format=tar", commit])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("git archive: {error}"))?;
    let stdout = archive.stdout.take().ok_or("git archive printed nothing")?;
    let unpacked = Command::new("tar")
        .arg("-x")
        .arg("-C")
        .arg(into)
        .stdin(stdout)
        .status()
        .map_err(|error| format!("tar: {error}"))?;
    let archived = archive
        .wait()
        .map_err(|error| format!("git archive: {error}"))?;
    if !archived.success() || !unpacked.success() {
        return Err(format!(
            "cannot write the files of {commit} into {}",
            into.display()
        ));
    }
    Ok(())
}

/// Loads the factory in `dir`, refusing one that `priory check` would refuse.
fn configuration(dir: &Path, side: &str, commit: &str) -> Result<Factory, String> {
    let (factory, mut problems) = factory::load(dir);
    if problems.is_empty() {
        problems = rules::check(&factory);
    }
    match problems.first() {
        None => Ok(factory),
        Some(problem) => Err(format!(
            "the {side} commit {} fails priory check: {problem}",
            short(commit)
        )),
    }
}

/// A case of the benchmark, loaded from the head's checkout.
struct Loaded {
    name: String,
    loaded: case::Loaded,
}

/// Compares the commits `base` and `head` of the factory repository on the benchmark `file`, and
/// returns the table. `db` is this machine's state, where each comparison leaves a
/// `factory.bench.compared` record; `home` holds Priory's clones. `say` prints a line as the
/// comparison goes.
pub fn compare(
    db: &mut Connection,
    home: &Path,
    file: &Path,
    base: &str,
    head: &str,
    options: &Options,
    say: &mut dyn FnMut(&str),
) -> Result<Compared, String> {
    let name = file
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or_else(|| format!("{} names no benchmark file", file.display()))?
        .to_owned();
    let folder = file
        .parent()
        .filter(|folder| folder.ends_with("benchmarks"));
    let Some(dir) = folder.and_then(Path::parent) else {
        return Err(format!(
            "{} is not in a factory's benchmarks/ folder",
            file.display()
        ));
    };
    let dir = match dir.as_os_str().is_empty() {
        true => Path::new("."),
        false => dir,
    };
    let top = git(dir, &["rev-parse", "--show-toplevel"])
        .map_err(|_| format!("{} is not in a Git repository", dir.display()))?;
    let top = PathBuf::from(top);
    let prefix = git(dir, &["rev-parse", "--show-prefix"])?;
    let resolve = |reference: &str| {
        git(
            &top,
            &[
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("{reference}^{{commit}}"),
            ],
        )
        .map_err(|_| format!("no commit {reference} in {}", top.display()))
    };
    let (base, head) = match &options.post {
        None => (resolve(base)?, resolve(head)?),
        // A pull request's commits may not be in this clone yet, and its base moves on.
        Some(_) => {
            let tip = fetched(&top, base)?;
            fetched(&top, head)?;
            let head = resolve(head)?;
            let base = git(&top, &["merge-base", &tip, &head])
                .map_err(|_| format!("{} and {} have no merge base", short(&tip), short(&head)))?;
            (base, head)
        }
    };

    let scratch =
        Scratch(std::env::temp_dir().join(format!("priory-bench-{}", std::process::id())));
    fs::create_dir(&scratch.0).map_err(io(&scratch.0))?;
    export(&top, &base, &scratch.0.join("base"))?;
    export(&top, &head, &scratch.0.join("head"))?;
    let (base_dir, head_dir) = (
        scratch.0.join("base").join(&prefix),
        scratch.0.join("head").join(&prefix),
    );
    let configurations = [
        configuration(&base_dir, "base", &base)?,
        configuration(&head_dir, "head", &head)?,
    ];
    let heads = &configurations[1];
    let benchmark = heads
        .benchmarks
        .get(&name)
        .ok_or_else(|| format!("the head commit {} has no benchmark {name}", short(&head)))?;
    let mut scorers = vec![benchmark.value.primary.clone()];
    for scorer in &benchmark.value.scorers {
        if !scorers.contains(scorer) {
            scorers.push(scorer.clone());
        }
    }
    let calibrations = score::Judging::new(db, heads, None).calibrations;
    let mut judges = Vec::new();
    // A judge that is not calibrated is left out whole, on both sides.
    for (name, calibration) in &calibrations {
        if !scorers.contains(name) {
            continue;
        }
        match calibration {
            Err(why) if *name == benchmark.value.primary => {
                return Err(format!(
                    "the primary scorer {name} is a judge whose verdict does not count yet: {why}"
                ));
            }
            Err(why) => judges.push(format!("left out on both sides: {name}: {why}")),
            Ok(calibration) => judges.push(format!(
                "judge {name}: {}, agreed on {}",
                heads.scorers[name]
                    .value
                    .model
                    .as_deref()
                    .unwrap_or_default(),
                calibration.shown()
            )),
        }
    }
    let scorers: Vec<(String, ScoreOn)> = scorers
        .into_iter()
        .filter(|name| calibrations.get(name).is_none_or(Result::is_ok))
        .filter_map(|name| {
            let on = heads.scorers.get(&name)?.value.on;
            Some((name, on))
        })
        .collect();
    // A judge is priced from what one call cost when it was calibrated.
    let judged: f64 = scorers
        .iter()
        .filter_map(|(name, _)| calibrations.get(name)?.as_ref().ok())
        .map(judge::Calibration::per_call)
        .sum();
    let paths = [format!("{prefix}scorers"), format!("{prefix}benchmarks")];
    let ruler_moved = !git(
        &top,
        &[
            "diff",
            "--name-only",
            &base,
            &head,
            "--",
            &paths[0],
            &paths[1],
        ],
    )?
    .is_empty();

    let mut cases = Vec::new();
    for path in &benchmark.value.cases {
        let case_name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        if options.case.as_ref().is_some_and(|only| *only != case_name) {
            continue;
        }
        let at = head_dir.join("benchmarks").join(path);
        let loaded =
            case::load(&at).map_err(|error| format!("the case {}: {error}", path.display()))?;
        cases.push(Loaded {
            name: case_name,
            loaded,
        });
    }
    if let Some(only) = &options.case
        && cases.is_empty()
    {
        return Err(format!("the benchmark {name} has no case {only}"));
    }

    let (mut each, mut overrun) = (0.0, 0.0f64);
    for case in &cases {
        for configuration in &configurations {
            let estimate = replay::estimate(&case.loaded.case, configuration, options.turn)?;
            // A judge scores each finished run, and a replay has at most one per model step.
            each += estimate.dollars + judged * case.loaded.case.spent.len().max(1) as f64;
            overrun = overrun.max(estimate.overrun);
        }
    }
    let total = each * options.runs as f64;
    say(&format!(
        "estimate: ${total:.4} of a ${:.4} budget, plus at most ${overrun:.4} if it ends inside a turn of {}",
        options.budget,
        jiff::SignedDuration::try_from(options.turn).map_or_else(
            |_| format!("{:?}", options.turn),
            |turn| format!("{turn:#}")
        )
    ));
    if total > options.budget {
        let fits = (options.budget / each).floor() as usize;
        let budget = (total * 100.0).ceil() / 100.0;
        return Err(match fits {
            0 => format!(
                "the estimate, ${total:.4}, is over the ${:.4} budget: give --budget {budget:.2} or more, or --case for fewer cases",
                options.budget
            ),
            runs => format!(
                "the estimate, ${total:.4}, is over the ${:.4} budget: give --runs {runs}, or --budget {budget:.2} or more",
                options.budget
            ),
        });
    }

    // A scorer that errors on a case's untouched start commit cannot measure the change there.
    let mut broken = Vec::new();
    cases.retain(|case| {
        let start = &case.loaded.case.start;
        let repo = case.loaded.case.repo.as_deref().unwrap_or_default();
        for (scorer, _) in &scorers {
            let Some(loaded) = heads.scorers.get(scorer) else {
                continue;
            };
            if loaded.value.kind != ScoreKind::Command {
                continue;
            }
            let result = json!({ "commit": start });
            let (verdict, why) =
                score::on_commit(&heads.dir, home, repo, start, start, &loaded.value, &result);
            if verdict == Verdict::Error {
                broken.push((
                    case.name.clone(),
                    format!("{scorer} errors on its start commit: {why}"),
                ));
                return false;
            }
        }
        true
    });

    let (mut earlier, complete) =
        trace::comparisons(db, &name, &base, &head).map_err(|error| error.to_string())?;
    if let Some(pull) = &options.post {
        earlier = trace::comparisons_of(db, &name, pull).map_err(|error| error.to_string())?;
    }
    let names: Vec<&str> = scorers.iter().map(|(name, _)| name.as_str()).collect();
    let mut spent = 0.0;
    let mut incomplete: Option<String> = None;
    let mut ran = Vec::new();
    let mut order = 0;
    for case in &cases {
        let mut runs = CaseRuns {
            name: case.name.clone(),
            partial: case.loaded.case.partial,
            pairs: Vec::new(),
            dropped: 0,
        };
        for pair in 1..=options.runs {
            if incomplete.is_none() && spent >= options.budget {
                incomplete = Some("the budget ended before every pair ran".to_owned());
            }
            if incomplete.is_some() {
                runs.dropped += 1;
                continue;
            }
            // The side that goes first alternates, so a warm cache or a drifting service favors
            // neither.
            let sides = match order % 2 {
                0 => [0, 1],
                _ => [1, 0],
            };
            order += 1;
            let mut played: [Option<Played>; 2] = [None, None];
            for at in sides {
                let configuration = &configurations[at];
                let side = ["base", "head"][at];
                let dir = [&base_dir, &head_dir][at];
                let replay_options = replay::Options {
                    budget: options.budget - spent,
                    turn: options.turn,
                };
                let mut judging = score::Judging {
                    calibrations: calibrations.clone(),
                    budget: Some(options.budget - spent),
                    spent: 0.0,
                };
                let replayed = replay::replay(
                    dir,
                    configuration,
                    &case.loaded,
                    &replay_options,
                    &mut |_| {},
                    &mut |db, home, task| {
                        score::grade(db, heads, home, task, &names, &mut judging)
                            .unwrap_or_default()
                    },
                )?;
                let cost = replayed.spent + judging.spent;
                spent += cost;
                say(&format!(
                    "{} pair {pair}: {side} {}, ${cost:.4}",
                    case.name, replayed.ending
                ));
                // A judge that ran out of budget, or spent past it, leaves the pair unmeasured.
                let judges_over = (judging.spent > 0.0 && spent > options.budget)
                    || replayed.scores.iter().any(|score| {
                        score.kind == ScoreKind::Judge && score.reason.starts_with("over budget")
                    });
                match &replayed.ending {
                    _ if judges_over => {
                        incomplete = Some("the budget ended inside a pair".to_owned());
                    }
                    Ending::OverBudget => {
                        incomplete = Some("the budget ended inside a pair".to_owned());
                    }
                    Ending::CostUnknown(why) => {
                        incomplete = Some(format!("a turn's cost is unknown: {why}"));
                    }
                    _ => {}
                }
                if incomplete.is_some() {
                    break;
                }
                played[at] = Some(Played {
                    verdicts: verdicts(&replayed.scores, &scorers),
                    ending: replayed.ending,
                    cost,
                    took: replayed.took,
                });
            }
            match played {
                [Some(base), Some(head)] => runs.pairs.push(Pair { base, head }),
                _ => runs.dropped += 1,
            }
        }
        ran.push(runs);
    }

    let comparison = Comparison {
        benchmark: name.clone(),
        base: base.clone(),
        head: head.clone(),
        number: earlier + 1,
        of: match options.post {
            Some(_) => "this pull request".to_owned(),
            None => "these commits".to_owned(),
        },
        repeat: complete,
        ruler_moved,
        incomplete,
        runs: options.runs,
        spent,
        budget: options.budget,
        scorers,
        cases: ran,
        broken,
        judges,
    };
    let text = table(&comparison);
    let call = verdict(&comparison);
    let trace = Trace {
        factory: heads,
        revision: Some(head.clone()),
        runner: None,
        machine: room::hostname(),
        usage: transcript::usage,
    };
    let data = json!({
        "factory": heads.name,
        "benchmark": name,
        "base": base,
        "head": head,
        "number": comparison.number,
        "complete": comparison.incomplete.is_none(),
        "repeat": comparison.repeat,
        "verdict": call.map(Call::name),
        "primary": comparison.scorers.first().map(|(name, _)| name.clone()),
        "cases": comparison.cases.len(),
        "runs": comparison.runs,
        "spent": (comparison.spent * 10_000.0).round() / 10_000.0,
        "budget": comparison.budget,
        "machine": trace.machine,
        "pull_request": options.post,
    });
    let error = |error: rusqlite::Error| error.to_string();
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(error)?;
    trace::compared(&tx, &trace, data).map_err(error)?;
    tx.commit().map_err(error)?;
    Ok(Compared {
        status: status(&comparison),
        table: text,
        head,
    })
}

/// What a comparison printed, and what `--post` sets on its head commit.
pub struct Compared {
    pub table: String,
    /// The head commit, in full.
    pub head: String,
    /// The commit status `priory/benchmark` and its description, or none for a repeat.
    pub status: Option<(&'static str, String)>,
}

/// The commit status a posted comparison sets: `success` when it is complete and its primary
/// scorer does not read `worse`, `failure` otherwise. A repeat leaves the status as it is.
pub fn status(comparison: &Comparison) -> Option<(&'static str, String)> {
    if comparison.repeat {
        return None;
    }
    let primary = comparison
        .scorers
        .first()
        .map_or("the primary scorer", |(name, _)| name.as_str());
    let (state, description) = match (&comparison.incomplete, verdict(comparison)) {
        (Some(why), _) => ("failure", format!("Incomplete: {why}")),
        (None, Some(Call::Worse)) => ("failure", format!("{primary}: worse")),
        (None, Some(call)) => ("success", format!("{primary}: {}", call.name())),
        (None, None) => return None,
    };
    // GitHub refuses a description over 140 characters.
    Some((state, description.chars().take(140).collect()))
}

/// The commit status context `--post` sets.
pub const CONTEXT: &str = "priory/benchmark";

/// The comment `--post` writes on the pull request: the table, as the terminal shows it.
pub fn comment(table: &str) -> String {
    format!("Priory's benchmark comparison of this pull request:\n\n```text\n{table}```\n")
}

/// Fetches `reference` from the origin of the repository `top`, and returns the commit it named.
fn fetched(top: &Path, reference: &str) -> Result<String, String> {
    git(
        top,
        &[
            "fetch",
            "--quiet",
            "--no-tags",
            "origin",
            "--end-of-options",
            reference,
        ],
    )
    .map_err(|error| {
        format!(
            "cannot fetch {reference} from the origin of {}: {error}",
            top.display()
        )
    })?;
    git(top, &["rev-parse", "FETCH_HEAD"])
}

#[cfg(test)]
mod tests {
    use super::*;

    const RESULT: ScoreOn = ScoreOn::Result;

    fn played(ending: Ending, verdicts: &[(&str, Verdict)]) -> Played {
        Played {
            ending,
            cost: 0.0,
            took: Duration::ZERO,
            verdicts: verdicts
                .iter()
                .map(|(name, verdict)| ((*name).to_owned(), *verdict))
                .collect(),
        }
    }

    fn reached(verdict: Verdict) -> Played {
        played(
            Ending::Reached("publish".to_owned()),
            &[("passed", verdict)],
        )
    }

    /// A case whose base and head replays pass `passed` as listed.
    fn case(name: &str, base: &[Played], head: &[Played]) -> CaseRuns {
        CaseRuns {
            name: name.to_owned(),
            partial: false,
            pairs: base
                .iter()
                .zip(head)
                .map(|(base, head)| Pair {
                    base: base.clone(),
                    head: head.clone(),
                })
                .collect(),
            dropped: 0,
        }
    }

    fn passes(n: usize, of: usize) -> Vec<Played> {
        (0..of)
            .map(|at| match at < n {
                true => reached(Verdict::Pass),
                false => reached(Verdict::Fail),
            })
            .collect()
    }

    fn comparison(cases: Vec<CaseRuns>) -> Comparison {
        Comparison {
            benchmark: "implement".to_owned(),
            base: "999daf0aa".to_owned(),
            head: "4c1e2b7bb".to_owned(),
            number: 1,
            of: "these commits".to_owned(),
            repeat: false,
            ruler_moved: false,
            incomplete: None,
            runs: 3,
            spent: 1.0,
            budget: 2.0,
            scorers: vec![("passed".to_owned(), RESULT)],
            cases,
            broken: vec![],
            judges: vec![],
        }
    }

    fn score(run: i64, scorer: &str, verdict: Verdict) -> Score {
        Score {
            run,
            step: "implement".to_owned(),
            scorer: scorer.to_owned(),
            kind: ScoreKind::Outcome,
            verdict,
            reason: String::new(),
            cost: 0.0,
            judged: None,
            intent: None,
        }
    }

    #[test]
    fn a_replay_takes_its_last_runs_verdict_and_the_worst_of_its_asks() {
        let scores = [
            score(1, "passed", Verdict::Pass),
            score(1, "short-ask", Verdict::Fail),
            score(2, "passed", Verdict::Fail),
            score(2, "short-ask", Verdict::Pass),
            score(2, "quiet", Verdict::NotApplicable),
        ];
        let scorers = [
            ("passed".to_owned(), ScoreOn::Result),
            ("short-ask".to_owned(), ScoreOn::Ask),
            ("quiet".to_owned(), ScoreOn::Ask),
            ("gate".to_owned(), ScoreOn::Diff),
        ];

        assert_eq!(
            verdicts(&scores, &scorers),
            BTreeMap::from([
                ("passed".to_owned(), Verdict::Fail),
                ("short-ask".to_owned(), Verdict::Fail),
                ("quiet".to_owned(), Verdict::NotApplicable),
            ])
        );
    }

    #[test]
    fn the_sign_test_is_exact_and_two_sided() {
        assert_eq!(sign_test(6, 0), 2.0 / 64.0);
        assert_eq!(sign_test(0, 6), 2.0 / 64.0);
        assert_eq!(sign_test(7, 1), 2.0 * 9.0 / 256.0);
        assert_eq!(sign_test(3, 3), 1.0);
        assert_eq!(sign_test(0, 0), 1.0);
        assert_eq!(call(6, 0), Call::Better);
        assert_eq!(call(0, 6), Call::Worse);
        assert_eq!(call(7, 1), Call::NoClearDifference);
        assert_eq!(call(5, 0), Call::TooFewCases);
        assert_eq!(call(9, 1), Call::Better);
    }

    #[test]
    fn a_base_that_passes_2_of_3_beats_a_head_that_passes_1_and_errors_twice_or_asks_twice() {
        let errors = vec![
            reached(Verdict::Pass),
            reached(Verdict::Error),
            reached(Verdict::Error),
        ];
        let asks = vec![
            reached(Verdict::Pass),
            played(Ending::Asked, &[("passed", Verdict::Fail)]),
            played(Ending::Asked, &[]),
        ];

        let erred = row(&[case("a", &passes(2, 3), &errors)], "passed", RESULT);
        let asked = row(&[case("a", &passes(2, 3), &asks)], "passed", RESULT);

        assert_eq!(
            erred,
            Row {
                scorer: "passed".to_owned(),
                base: (2, 3, 0),
                head: (1, 3, 2),
                mean: Some(100.0 * (1.0 / 3.0 - 2.0 / 3.0)),
                won: 0,
                tied: 0,
                lost: 1,
            }
        );
        assert_eq!((asked.head, asked.lost, asked.won), ((1, 3, 0), 1, 0));
    }

    #[test]
    fn a_replay_with_no_result_never_passes_even_with_a_stale_pass() {
        let stale = played(Ending::TimedOut, &[("passed", Verdict::Pass)]);

        let counted = row(&[case("a", &passes(0, 1), &[stale])], "passed", RESULT);

        assert_eq!(counted.head, (0, 1, 0));
        assert_eq!(counted.tied, 1);
    }

    #[test]
    fn ties_count_as_tied_and_leave_the_sign_test() {
        // Five cases the head won and seven it tied: only five did not tie, too few for a verdict.
        let mut cases: Vec<CaseRuns> = (0..5)
            .map(|at| case(&format!("won{at}"), &passes(1, 3), &passes(3, 3)))
            .collect();
        cases.extend((0..7).map(|at| case(&format!("tied{at}"), &passes(2, 3), &passes(2, 3))));
        let few = comparison(cases.clone());
        cases.push(case("won5", &passes(0, 3), &passes(1, 3)));
        let enough = comparison(cases);

        let counted = row(&few.cases, "passed", RESULT);

        assert_eq!((counted.won, counted.tied, counted.lost), (5, 7, 0));
        assert_eq!(verdict(&few), Some(Call::TooFewCases));
        assert_eq!(verdict(&enough), Some(Call::Better));
        assert!(table(&few).contains("\npassed  19/36  0       29/36  0       +27.8      5    7     0     too few cases\n"), "{}", table(&few));
    }

    #[test]
    fn a_dropped_pair_is_counted_and_leaves_the_comparison_with_no_verdict() {
        let mut cut = case("cut", &passes(2, 2), &passes(2, 2));
        cut.dropped = 1;
        let mut comparison = comparison(vec![case("whole", &passes(2, 3), &passes(3, 3)), cut]);
        comparison.incomplete = Some("the budget ended before every pair ran".to_owned());

        let text = table(&comparison);

        assert_eq!(verdict(&comparison), None);
        assert_eq!(
            text.lines().take(3).collect::<Vec<_>>(),
            [
                "Incomplete: the budget ended before every pair ran, so this table gives no verdict",
                "benchmark implement, base 999daf0, head 4c1e2b7, comparison 1 of these commits",
                "2 cases, 3 pairs each, 1 pair dropped, $1.00 of $2",
            ]
        );
        assert!(
            text.contains("\npassed  4/5   0       5/5   0       +16.7      1    1     0\n"),
            "{text}"
        );
    }

    #[test]
    fn a_repeat_shows_its_table_and_carries_no_verdict() {
        let cases: Vec<CaseRuns> = (0..6)
            .map(|at| case(&format!("c{at}"), &passes(0, 3), &passes(3, 3)))
            .collect();
        let mut repeat = comparison(cases);
        repeat.number = 2;
        repeat.repeat = true;
        repeat.ruler_moved = true;

        let text = table(&repeat);

        assert_eq!(verdict(&repeat), None);
        assert_eq!(
            text.lines().take(2).collect::<Vec<_>>(),
            [
                "The change touches scorers/ or benchmarks/, so it moved the ruler too",
                "benchmark implement, base 999daf0, head 4c1e2b7, comparison 2 of these commits, a repeat: the first complete comparison gave the verdict",
            ]
        );
        assert!(
            text.contains(
                "\npassed  0/18  0       18/18  0       +100.0     6    0     0     repeat\n"
            ),
            "{text}"
        );
    }

    #[test]
    fn a_posted_comparison_passes_unless_incomplete_or_worse_and_a_repeat_leaves_the_status() {
        let split = |base: usize, head: usize| -> Vec<CaseRuns> {
            (0..6)
                .map(|at| case(&format!("c{at}"), &passes(base, 3), &passes(head, 3)))
                .collect()
        };
        let better = comparison(split(0, 3));
        let worse = comparison(split(3, 0));
        let few = comparison(split(0, 3)[..2].to_vec());
        let mut incomplete = comparison(split(0, 3));
        incomplete.incomplete = Some("the budget ended inside a pair".to_owned());
        let mut repeat = comparison(split(3, 0));
        repeat.repeat = true;

        assert_eq!(
            status(&better),
            Some(("success", "passed: better".to_owned()))
        );
        assert_eq!(
            status(&worse),
            Some(("failure", "passed: worse".to_owned()))
        );
        assert_eq!(
            status(&few),
            Some(("success", "passed: too few cases".to_owned()))
        );
        assert_eq!(
            status(&incomplete),
            Some((
                "failure",
                "Incomplete: the budget ended inside a pair".to_owned()
            ))
        );
        assert_eq!(status(&repeat), None);
    }

    /// One scorer's passes on each side of a case, out of 3.
    type Split = (usize, usize);

    /// The cases of the design note's example, built from each case's splits of gate, passed and
    /// merged-tests, its endings and the ask scorer's verdicts.
    fn november() -> Vec<CaseRuns> {
        const GATE: [Split; 12] = [
            (2, 3),
            (2, 3),
            (2, 3),
            (2, 3),
            (2, 3),
            (1, 3),
            (3, 3),
            (3, 3),
            (3, 3),
            (2, 2),
            (2, 2),
            (2, 2),
        ];
        const PASSED: [Split; 12] = [
            (2, 3),
            (2, 3),
            (1, 3),
            (3, 2),
            (3, 3),
            (3, 3),
            (3, 3),
            (3, 3),
            (3, 3),
            (3, 3),
            (2, 2),
            (2, 2),
        ];
        const MERGED: [Split; 12] = [
            (1, 2),
            (1, 2),
            (1, 2),
            (0, 2),
            (3, 3),
            (3, 3),
            (0, 2),
            (2, 1),
            (2, 1),
            (2, 2),
            (2, 2),
            (2, 2),
        ];
        let reached = || Ending::Reached("open-pr".to_owned());
        let mut cases = Vec::new();
        for at in 0..12 {
            let mut pairs = Vec::new();
            for run in 0..3 {
                let one = |side: usize| {
                    let pick = |split: Split| if side == 0 { split.0 } else { split.1 };
                    let verdict = |split: Split| match run < pick(split) {
                        true => Verdict::Pass,
                        false => Verdict::Fail,
                    };
                    let ending = match (side, at, run) {
                        (0, 0 | 1, 2) | (1, 10 | 11, 2) => Ending::Asked,
                        (0, 2, 2) => Ending::TimedOut,
                        (0, 4, _) | (1, 3, 0) => Ending::Stopped,
                        _ => reached(),
                    };
                    let mut verdicts = BTreeMap::from([
                        ("gate".to_owned(), verdict(GATE[at])),
                        ("passed".to_owned(), verdict(PASSED[at])),
                        ("merged-tests".to_owned(), verdict(MERGED[at])),
                    ]);
                    if (side, at, run) == (0, 5, 1) {
                        verdicts.insert("gate".to_owned(), Verdict::Error);
                    }
                    let asked = match (side, at, run) {
                        (0, 0, 2) => Some(Verdict::Fail),
                        (1, 0, 0) | (0, 10, 0) | (1, 10, 2) => Some(Verdict::Pass),
                        _ => None,
                    };
                    verdicts.insert(
                        "short-ask".to_owned(),
                        asked.unwrap_or(Verdict::NotApplicable),
                    );
                    Played {
                        ending,
                        cost: [0.41, 0.47][side],
                        took: Duration::from_secs([660, 720][side]),
                        verdicts,
                    }
                };
                pairs.push(Pair {
                    base: one(0),
                    head: one(1),
                });
            }
            cases.push(CaseRuns {
                name: format!("case-{at}"),
                partial: false,
                pairs,
                dropped: 0,
            });
        }
        cases
    }

    #[test]
    fn the_design_notes_example_prints_its_table() {
        let comparison = Comparison {
            benchmark: "implement-november".to_owned(),
            base: "999daf0".to_owned(),
            head: "4c1e2b7".to_owned(),
            number: 1,
            of: "this pull request".to_owned(),
            repeat: false,
            ruler_moved: false,
            incomplete: None,
            runs: 3,
            spent: 31.2,
            budget: 40.0,
            scorers: vec![
                ("gate".to_owned(), ScoreOn::Diff),
                ("passed".to_owned(), ScoreOn::Result),
                ("merged-tests".to_owned(), ScoreOn::Diff),
                ("short-ask".to_owned(), ScoreOn::Ask),
            ],
            cases: november(),
            broken: vec![],
            judges: vec![],
        };

        assert_eq!(
            table(&comparison),
            "\
benchmark implement-november, base 999daf0, head 4c1e2b7, comparison 1 of this pull request
12 cases, 3 pairs each, 0 pairs dropped, $31.20 of $40

scorer        base   errors  head   errors  mean diff  won  tied  lost  verdict
gate          26/36  1       33/36  0       +19.4      6    6     0     better
passed        30/36  0       33/36  0       +8.3       3    8     1
merged-tests  19/36  0       24/36  0       +13.9      5    5     2
short-ask     1/2    0       2/2    0       +50.0      1    1     0

side  reached open-pr  stopped  asked  timed out  median cost  median time
base  30               3        2      1          $0.41        11 min
head  33               1        2      0          $0.47        12 min
"
        );
    }
}
