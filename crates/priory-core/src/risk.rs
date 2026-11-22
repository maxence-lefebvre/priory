//! `priory report risk`: for each irreversible step, the share of tasks through it that got an
//! adverse outcome label within a fixed horizon, as a Beta(1, 1) posterior. Nothing reads it to
//! approve, skip or order work.

use crate::label::MARKED_WRONG;
use jiff::{SignedDuration, Timestamp};
use priory_factory::factory::{Decider, Effect, Factory};
use rusqlite::Connection;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

/// The days after its reach that decide a trial's outcome.
pub const HORIZON_DAYS: i64 = 30;
/// The trials a row needs before it shows percentages.
pub const FLOOR: usize = 20;
/// The window `--since` replaces.
pub const SINCE: &str = "180 days";

/// The labels that make a trial `judged wrong`.
const JUDGED: [&str; 6] = [
    "changes-requested",
    "closed-unmerged",
    "approval-rejected",
    "red-after-merge",
    "reverted",
    MARKED_WRONG,
];
/// The labels that make a trial `landed wrong`. A person's `marked-wrong` does too when a pull
/// request of the task had merged by the mark's date.
const LANDED: [&str; 2] = ["red-after-merge", "reverted"];

/// The share of the Beta(a, b) distribution at or below `x`, for whole `a` and `b` of at least 1:
/// the chance that a binomial of `a + b - 1` draws at `x` gets at least `a`.
pub fn beta_cdf(x: f64, a: u64, b: u64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    let m = a + b - 1;
    let ln_fact: Vec<f64> = (0..=m)
        .scan(0.0, |sum, i| {
            *sum += if i == 0 { 0.0 } else { (i as f64).ln() };
            Some(*sum)
        })
        .collect();
    let (ln_x, ln_y) = (x.ln(), (1.0 - x).ln());
    let total: f64 = (a..=m)
        .map(|j| {
            let ln_choose = ln_fact[m as usize] - ln_fact[j as usize] - ln_fact[(m - j) as usize];
            (ln_choose + j as f64 * ln_x + (m - j) as f64 * ln_y).exp()
        })
        .sum();
    total.min(1.0)
}

/// The `p` quantile of Beta(a, b), by bisection on `beta_cdf`.
pub fn beta_quantile(p: f64, a: u64, b: u64) -> f64 {
    let (mut low, mut high) = (0.0, 1.0);
    for _ in 0..100 {
        let middle = (low + high) / 2.0;
        if beta_cdf(middle, a, b) < p {
            low = middle;
        } else {
            high = middle;
        }
    }
    (low + high) / 2.0
}

/// `k` adverse trials of a row's `n`, and the posterior's 5th, 50th and 95th percentiles.
struct Estimate {
    k: usize,
    quantiles: [f64; 3],
}

impl Estimate {
    fn new(k: usize, n: usize) -> Self {
        let (a, b) = (1 + k as u64, 1 + (n - k) as u64);
        Estimate {
            k,
            quantiles: [0.05, 0.5, 0.95].map(|p| beta_quantile(p, a, b)),
        }
    }

    fn json(&self) -> Value {
        let [p05, median, p95] = self.quantiles;
        json!({ "k": self.k, "p05": p05, "median": median, "p95": p95 })
    }
}

/// A target's own row, with no `step`, or a conditioned row: the tasks whose last record of `step`
/// before the reach had `outcome`.
#[derive(Default)]
struct Row {
    judged: usize,
    landed: usize,
    n: usize,
}

struct Adverse {
    task: String,
    outcome: &'static str,
    label: String,
    happened: Timestamp,
    evidence: String,
}

/// A `fixes` relation on a task that reached the target, which a person has not marked wrong.
struct Candidate {
    task: String,
    fixed_by: i64,
    by: String,
}

/// An irreversible step of a workflow, with its rows and counts.
#[derive(Default)]
struct Target {
    workflow: String,
    step: String,
    /// The lane of every task the target counts, with `--by lane`; `-` for a task with none.
    lane: Option<String>,
    reached: usize,
    caught: BTreeSet<String>,
    pending: usize,
    unjudged: usize,
    undated: usize,
    open: usize,
    late: usize,
    revisions: BTreeSet<String>,
    models: BTreeSet<String>,
    own: Row,
    rows: BTreeMap<(String, String), Row>,
    adverse: Vec<Adverse>,
    /// The tasks whose reach lies in the window.
    tasks: BTreeSet<String>,
    candidates: Vec<Candidate>,
}

/// A human step that approves a target: its id and the outcomes that lead to `stop`.
struct Gate {
    id: String,
    stops: BTreeSet<String>,
}

/// Each workflow's irreversible steps with the human steps that approve them, read from the
/// factory once. The default knows none, and finds the targets in the records alone.
#[derive(Default)]
pub struct Gates(BTreeMap<(String, String), Vec<Gate>>);

impl Gates {
    pub fn of(factory: &Factory) -> Self {
        let mut gates = BTreeMap::new();
        for (name, loaded) in &factory.workflows {
            let steps = &loaded.value.steps;
            for target in steps
                .iter()
                .filter(|step| step.effect == Effect::Irreversible)
            {
                let approving = steps
                    .iter()
                    .filter(|gate| {
                        gate.decides == Decider::Human && gate.approves.contains(&target.id)
                    })
                    .map(|gate| Gate {
                        id: gate.id.clone(),
                        stops: gate
                            .next
                            .iter()
                            .filter(|(_, to)| *to == "stop")
                            .map(|(outcome, _)| outcome.clone())
                            .collect(),
                    })
                    .collect();
                gates.insert((name.clone(), target.id.clone()), approving);
            }
        }
        Gates(gates)
    }
}

/// Each target's trials and estimates.
pub struct Risk {
    since: String,
    by_lane: bool,
    targets: Vec<Target>,
}

struct Ended {
    time: Timestamp,
    task: String,
    workflow: String,
    step: String,
    effect: String,
    outcome: String,
    revision: Option<String>,
    model: Option<String>,
}

struct Label {
    label: String,
    happened: Option<Timestamp>,
    evidence: String,
}

/// A pull request of a task: when Priory first knew of it, when it closed, and when it merged.
struct Pull {
    seen: Timestamp,
    closed: Option<Timestamp>,
    merged: Option<Timestamp>,
}

fn instant(text: &str) -> Result<Timestamp, String> {
    text.parse()
        .map_err(|error| format!("time {text:?}: {error}"))
}

/// The trials of each irreversible step of `gates`' workflows and of the records, or only
/// `workflow`'s, whose reach lies between `start` and `now`. With `by_lane`, each target splits
/// into one per lane its tasks have now.
pub fn risk(
    db: &Connection,
    gates: &Gates,
    workflow: Option<&str>,
    since: &str,
    start: &str,
    now: Timestamp,
    by_lane: bool,
) -> Result<Risk, String> {
    let error = |error: rusqlite::Error| error.to_string();
    let start = instant(start)?;
    let horizon = SignedDuration::from_hours(24 * HORIZON_DAYS);
    let ended: Vec<Ended> = db
        .prepare(
            "SELECT time, json_extract(event, '$.data.task'), json_extract(event, '$.data.workflow'),
               json_extract(event, '$.data.step'), json_extract(event, '$.data.effect'),
               json_extract(event, '$.data.outcome'),
               json_extract(event, '$.data.factory_revision'), json_extract(event, '$.data.model')
             FROM trace WHERE type = 'factory.step.ended'
               AND json_extract(event, '$.data.task') IS NOT NULL
               AND json_extract(event, '$.data.workflow') IS NOT NULL
               AND json_extract(event, '$.data.step') IS NOT NULL
               AND json_extract(event, '$.data.effect') IS NOT NULL
               AND json_extract(event, '$.data.outcome') IS NOT NULL
             ORDER BY seq",
        )
        .and_then(|mut statement| {
            statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        Ended {
                            time: Timestamp::UNIX_EPOCH,
                            task: row.get(1)?,
                            workflow: row.get(2)?,
                            step: row.get(3)?,
                            effect: row.get(4)?,
                            outcome: row.get(5)?,
                            revision: row.get(6)?,
                            model: row.get(7)?,
                        },
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .map_err(error)?
        .into_iter()
        .map(|(time, ended)| Ok(Ended { time: instant(&time)?, ..ended }))
        .collect::<Result<_, String>>()?;
    let mut labels: BTreeMap<String, Vec<Label>> = BTreeMap::new();
    // A clear withdraws the marks before it, whenever it was written: it says the mark was a
    // mistake, not that something happened.
    let rows: Vec<(String, String, Option<String>, String)> = db
        .prepare(
            "SELECT json_extract(event, '$.data.task'), json_extract(event, '$.data.label'),
               json_extract(event, '$.data.happened'), json_extract(event, '$.data.evidence')
             FROM trace AS labeled WHERE type = 'factory.step.labeled'
             AND NOT EXISTS (SELECT 1 FROM trace AS cleared WHERE cleared.type = 'factory.step.unlabeled'
               AND cleared.seq > labeled.seq
               AND json_extract(cleared.event, '$.data.task') = json_extract(labeled.event, '$.data.task')
               AND json_extract(cleared.event, '$.data.label') = json_extract(labeled.event, '$.data.label'))
             ORDER BY seq",
        )
        .and_then(|mut statement| {
            statement
                .query_map([], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
                })?
                .collect()
        })
        .map_err(error)?;
    for (task, label, happened, evidence) in rows {
        let happened = happened.as_deref().map(instant).transpose()?;
        labels.entry(task).or_default().push(Label {
            label,
            happened,
            evidence,
        });
    }
    let mut pulls: BTreeMap<String, Vec<Pull>> = BTreeMap::new();
    let rows: Vec<(i64, String, Option<String>, Option<String>)> = db
        .prepare(
            "WITH seen AS (
               SELECT task, resource, created AS time FROM pull_requests
               UNION ALL
               SELECT owns.task, 'github:' || json_extract(inbox.data, '$.repo') || '#'
                 || json_extract(inbox.data, '$.pull_request'), inbox.time
               FROM inbox JOIN owns ON owns.resource = 'github:' || json_extract(inbox.data, '$.repo')
                 || '@' || json_extract(inbox.data, '$.head_branch')
               WHERE inbox.type LIKE 'github.pull_request%'
                 AND json_extract(inbox.data, '$.pull_request') IS NOT NULL)
             SELECT task, min(time), (SELECT min(time) FROM inbox
               WHERE type = 'github.pull_request.closed' AND 'github:' || json_extract(data, '$.repo')
                 || '#' || json_extract(data, '$.pull_request') = seen.resource),
               (SELECT min(time) FROM inbox
               WHERE type = 'github.pull_request.closed' AND json_extract(data, '$.merged') = 1
                 AND 'github:' || json_extract(data, '$.repo')
                 || '#' || json_extract(data, '$.pull_request') = seen.resource)
             FROM seen GROUP BY task, resource",
        )
        .and_then(|mut statement| {
            statement
                .query_map([], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
                })?
                .collect()
        })
        .map_err(error)?;
    for (task, seen, closed, merged) in rows {
        pulls.entry(task.to_string()).or_default().push(Pull {
            seen: instant(&seen)?,
            closed: closed.as_deref().map(instant).transpose()?,
            merged: merged.as_deref().map(instant).transpose()?,
        });
    }
    let fixes: Vec<(i64, i64, String)> = db
        .prepare("SELECT other, task, by FROM relations WHERE kind = 'fixes' ORDER BY other, task")
        .and_then(|mut statement| {
            statement
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
                .collect()
        })
        .map_err(error)?;

    let lanes: BTreeMap<String, String> = db
        .prepare("SELECT CAST(id AS TEXT), coalesce(lane, '-') FROM tasks")
        .and_then(|mut statement| {
            statement
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect()
        })
        .map_err(error)?;
    let lane_of = |task: &str| lanes.get(task).map_or("-", String::as_str);

    let mut wanted: BTreeSet<(String, String)> = gates.0.keys().cloned().collect();
    wanted.extend(
        ended
            .iter()
            .filter(|record| record.effect == "irreversible")
            .map(|record| (record.workflow.clone(), record.step.clone())),
    );
    wanted.retain(|(name, _)| workflow.is_none_or(|workflow| workflow == name));

    let mut targets = Vec::new();
    for (name, step) in wanted {
        let approving = gates
            .0
            .get(&(name.clone(), step.clone()))
            .map(Vec::as_slice)
            .unwrap_or_default();
        let tasks: BTreeSet<&str> = ended
            .iter()
            .filter(|record| record.workflow == name)
            .map(|record| record.task.as_str())
            .collect();
        let groups: Vec<Option<&str>> = match by_lane {
            true => tasks
                .iter()
                .map(|task| lane_of(task))
                .collect::<BTreeSet<_>>()
                .into_iter()
                .map(Some)
                .collect(),
            false => vec![None],
        };
        for group in groups {
            let mut target = Target {
                workflow: name.clone(),
                step: step.clone(),
                lane: group.map(str::to_owned),
                ..Target::default()
            };
            for &task in tasks
                .iter()
                .filter(|task| group.is_none_or(|lane| lane_of(task) == lane))
            {
                let records: Vec<&Ended> = ended
                    .iter()
                    .filter(|record| record.task == task && record.workflow == name)
                    .collect();
                let reach = records.iter().position(|record| record.step == step);
                let before = &records[..reach.unwrap_or(records.len())];
                let rejected = before.iter().any(|record| {
                    (start..=now).contains(&record.time)
                        && approving.iter().any(|gate| {
                            gate.id == record.step
                                && (record.outcome.to_lowercase().starts_with("reject")
                                    || gate.stops.contains(&record.outcome))
                        })
                });
                if rejected {
                    target.caught.insert(task.to_owned());
                }
                let Some(reach) = reach.map(|at| records[at]) else {
                    continue;
                };
                if !(start..=now).contains(&reach.time) {
                    continue;
                }
                target.reached += 1;
                target.tasks.insert(task.to_owned());
                let end = reach.time + horizon;
                if end > now {
                    target.pending += 1;
                    continue;
                }
                let pulls = pulls.get(task).map(Vec::as_slice).unwrap_or_default();
                if !pulls.iter().any(|pull| pull.seen <= end) {
                    target.unjudged += 1;
                    continue;
                }
                let labels = labels.get(task).map(Vec::as_slice).unwrap_or_default();
                let dated: Option<Vec<(&Label, Timestamp)>> = labels
                    .iter()
                    .map(|label| label.happened.map(|happened| (label, happened)))
                    .collect();
                let Some(dated) = dated else {
                    target.undated += 1;
                    continue;
                };
                let merged_by = |at: Timestamp| {
                    pulls
                        .iter()
                        .any(|pull| pull.merged.is_some_and(|merged| merged <= at))
                };
                let first = |landed: bool| {
                    dated
                        .iter()
                        .filter(|(label, happened)| {
                            let label = label.label.as_str();
                            let counts = match landed {
                                true => {
                                    LANDED.contains(&label)
                                        || (label == MARKED_WRONG && merged_by(*happened))
                                }
                                false => JUDGED.contains(&label),
                            };
                            counts && *happened > reach.time
                        })
                        .min_by_key(|(_, happened)| *happened)
                        .copied()
                };
                let landed = first(true).filter(|(_, happened)| *happened <= end);
                let judged = first(false).filter(|(_, happened)| *happened <= end);
                if judged.is_none() && first(false).is_some() {
                    target.late += 1;
                }
                if pulls
                    .iter()
                    .any(|pull| pull.seen <= end && pull.closed.is_none_or(|closed| closed > end))
                {
                    target.open += 1;
                }
                if let Some((label, happened)) = landed.or(judged) {
                    target.adverse.push(Adverse {
                        task: task.to_owned(),
                        outcome: if landed.is_some() {
                            "landed wrong"
                        } else {
                            "judged wrong"
                        },
                        label: label.label.clone(),
                        happened,
                        evidence: label.evidence.clone(),
                    });
                }
                target.revisions.extend(reach.revision.clone());
                target
                    .models
                    .extend(before.iter().filter_map(|record| record.model.clone()));
                let mut last: BTreeMap<&str, &str> = BTreeMap::new();
                for record in before {
                    last.insert(&record.step, &record.outcome);
                }
                let (judged, landed) = (judged.is_some(), landed.is_some());
                target.own.count(judged, landed);
                for (step, outcome) in last {
                    let key = (step.to_owned(), outcome.to_owned());
                    target.rows.entry(key).or_default().count(judged, landed);
                }
            }
            target.candidates = fixes
                .iter()
                .filter(|(fixed, ..)| {
                    let fixed = fixed.to_string();
                    target.tasks.contains(&fixed)
                        && !labels.get(&fixed).is_some_and(|labels| {
                            labels.iter().any(|label| label.label == MARKED_WRONG)
                        })
                })
                .map(|(fixed, fixed_by, by)| Candidate {
                    task: fixed.to_string(),
                    fixed_by: *fixed_by,
                    by: by.clone(),
                })
                .collect();
            targets.push(target);
        }
    }
    Ok(Risk {
        since: since.to_owned(),
        by_lane,
        targets,
    })
}

impl Row {
    fn count(&mut self, judged: bool, landed: bool) {
        self.n += 1;
        self.judged += usize::from(judged);
        self.landed += usize::from(landed);
    }

    fn json(&self, step: &str, outcome: Option<&str>) -> Value {
        json!({
            "step": step,
            "outcome": outcome,
            "trials": self.n,
            "shown": self.n >= FLOOR,
            "judged_wrong": Estimate::new(self.judged, self.n).json(),
            "landed_wrong": Estimate::new(self.landed, self.n).json(),
        })
    }

    /// The row's cells after its name: two groups of `k of n` and three percentiles.
    fn cells(&self) -> Vec<String> {
        let mut cells = Vec::new();
        for k in [self.judged, self.landed] {
            if self.n < FLOOR {
                cells.push(format!("{k} of {}, needs {FLOOR}", self.n));
                cells.extend([String::new(), String::new(), String::new()]);
                continue;
            }
            cells.push(format!("{k} of {}", self.n));
            cells.extend(
                Estimate::new(k, self.n)
                    .quantiles
                    .map(|q| format!("{:.1}%", q * 100.0)),
            );
        }
        cells
    }
}

/// Every target's trials over the default window up to `now`, for the asks and the flows.
pub fn recent(db: &Connection, gates: &Gates, now: Timestamp) -> Result<Risk, String> {
    let start = crate::report::start(db, SINCE, &now.to_string())?;
    risk(db, gates, None, SINCE, &start, now, false)
}

impl Risk {
    /// The own row of `workflow`'s irreversible step `step`, with the one History line an ask
    /// shows. A step no task reached has no trials.
    pub fn estimate(&self, workflow: &str, step: &str) -> Value {
        let target = self
            .targets
            .iter()
            .find(|target| target.workflow == workflow && target.step == step);
        let empty = Row::default();
        let own = target.map_or(&empty, |target| &target.own);
        let mut value = own.json(step, None);
        let line = format!(
            "History: {} of {} tasks through this step were judged wrong within {HORIZON_DAYS} days, {} after the merge.",
            own.judged, own.n, own.landed
        );
        let line = match own.n < FLOOR {
            true => format!("{line} Too few to estimate."),
            false => format!(
                "{line} 95\u{a0}% posterior upper bounds: {:.1}\u{a0}% and {:.1}\u{a0}%.",
                value["judged_wrong"]["p95"].as_f64().unwrap_or_default() * 100.0,
                value["landed_wrong"]["p95"].as_f64().unwrap_or_default() * 100.0
            ),
        };
        if let Some(object) = value.as_object_mut() {
            object.remove("outcome");
        }
        value["workflow"] = json!(workflow);
        value["line"] = json!(line);
        value
    }

    /// `estimate` with `caught`, the tasks a person stopped at the step's gate, as the flows show
    /// it. Only a `Risk` built with the factory's `Gates` knows them.
    pub fn step_history(&self, workflow: &str, step: &str) -> Value {
        let caught = self
            .targets
            .iter()
            .find(|target| target.workflow == workflow && target.step == step)
            .map_or(0, |target| target.caught.len());
        let mut value = self.estimate(workflow, step);
        value["caught"] = json!(caught);
        value
    }

    pub fn json(&self) -> Value {
        json!({
            "horizon_days": HORIZON_DAYS,
            "floor": FLOOR,
            "since": self.since,
            "by": self.by_lane.then_some("lane"),
            "targets": self.targets.iter().map(|target| {
                let mut value = json!({
                "workflow": target.workflow,
                "step": target.step,
                "reached": target.reached,
                "caught": target.caught.len(),
                "pending": target.pending,
                "unjudged": target.unjudged,
                "undated": target.undated,
                "open": target.open,
                "late": target.late,
                "factory_revisions": target.revisions.len(),
                "models": target.models.len(),
                "rows": std::iter::once(target.own.json(&target.step, None))
                    .chain(target.rows.iter().map(|((step, outcome), row)| row.json(step, Some(outcome))))
                    .collect::<Vec<_>>(),
                "adverse": target.adverse.iter().map(|adverse| json!({
                    "task": adverse.task,
                    "outcome": adverse.outcome,
                    "label": adverse.label,
                    "happened": adverse.happened.to_string(),
                    "evidence": adverse.evidence,
                })).collect::<Vec<_>>(),
                "candidates": target.candidates.iter().map(|candidate| json!({
                    "task": candidate.task,
                    "fixed_by": candidate.fixed_by.to_string(),
                    "by": candidate.by,
                })).collect::<Vec<_>>(),
                });
                if let Some(lane) = &target.lane {
                    value["lane"] = json!(lane);
                }
                value
            }).collect::<Vec<_>>(),
        })
    }

    pub fn print(&self) {
        for (at, target) in self.targets.iter().enumerate() {
            if at > 0 {
                crate::state::emit("");
            }
            let lane = target
                .lane
                .as_ref()
                .map_or_else(String::new, |lane| format!(", lane {lane}"));
            crate::state::emit(&format!(
                "{} / {}{lane}: reached {}, caught {} at its gate, pending {}, unjudged {}, undated {}, open {}, late {}",
                target.workflow,
                target.step,
                target.reached,
                target.caught.len(),
                target.pending,
                target.unjudged,
                target.undated,
                target.open,
                target.late
            ));
            crate::state::emit(&format!(
                "horizon {HORIZON_DAYS} days, window {}, {} factory revisions, {} models",
                self.since,
                target.revisions.len(),
                target.models.len()
            ));
            crate::state::emit("");
            let header = [
                "row",
                "judged wrong",
                "5%",
                "median",
                "95%",
                "landed wrong",
                "5%",
                "median",
                "95%",
            ];
            let mut table = vec![(header.map(str::to_owned).to_vec(), false)];
            let mut own = vec![target.step.clone()];
            own.extend(target.own.cells());
            table.push((own, target.own.n < FLOOR));
            for ((step, outcome), row) in &target.rows {
                let mut cells = vec![format!("{step} {outcome}")];
                cells.extend(row.cells());
                table.push((cells, row.n < FLOOR));
            }
            for line in columns(&table) {
                crate::state::emit(&line);
            }
            if !target.adverse.is_empty() {
                crate::state::emit("");
                crate::state::emit("adverse trials");
                let mut table = vec![(
                    ["task", "outcome", "label", "happened", "evidence"]
                        .map(str::to_owned)
                        .to_vec(),
                    false,
                )];
                for adverse in &target.adverse {
                    table.push((
                        vec![
                            adverse.task.clone(),
                            adverse.outcome.to_owned(),
                            adverse.label.clone(),
                            adverse.happened.strftime("%Y-%m-%d").to_string(),
                            adverse.evidence.clone(),
                        ],
                        false,
                    ));
                }
                for line in columns(&table) {
                    crate::state::emit(&line);
                }
            }
            if !target.candidates.is_empty() {
                crate::state::emit("");
                crate::state::emit("candidates: `fixes` relations a person has not marked");
                let mut table = vec![(
                    ["task", "fixed by", "recorded by"]
                        .map(str::to_owned)
                        .to_vec(),
                    false,
                )];
                for candidate in &target.candidates {
                    table.push((
                        vec![
                            candidate.task.clone(),
                            candidate.fixed_by.to_string(),
                            candidate.by.clone(),
                        ],
                        false,
                    ));
                }
                for line in columns(&table) {
                    crate::state::emit(&line);
                }
            }
        }
    }
}

/// Lines of `table` with each column padded to its widest cell. A row marked short spans each
/// `k of n, needs 20` cell over the three percentile columns after it.
fn columns(table: &[(Vec<String>, bool)]) -> Vec<String> {
    let count = table[0].0.len();
    let mut widths = vec![0; count];
    for (cells, short) in table {
        for (column, cell) in cells.iter().enumerate() {
            if !short || column == 0 {
                widths[column] = widths[column].max(cell.len());
            }
        }
    }
    for (cells, _) in table.iter().filter(|(_, short)| *short) {
        for column in [1, 5] {
            let span = widths[column..column + 4].iter().sum::<usize>() + 6;
            widths[column] += cells[column].len().saturating_sub(span);
        }
    }
    table
        .iter()
        .map(|(cells, short)| {
            let mut line = String::new();
            let mut column = 0;
            while column < count {
                let (cell, width) = match (*short, column) {
                    (true, 1 | 5) => (
                        &cells[column],
                        widths[column..column + 4].iter().sum::<usize>() + 6,
                    ),
                    _ => (&cells[column], widths[column]),
                };
                line.push_str(&format!("{cell:width$}  "));
                column += if *short && matches!(column, 1 | 5) {
                    4
                } else {
                    1
                };
            }
            line.trim_end().to_owned()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn percent(k: u64, n: u64) -> [String; 3] {
        [0.05, 0.5, 0.95].map(|p| format!("{:.1}", beta_quantile(p, 1 + k, 1 + n - k) * 100.0))
    }

    #[test]
    fn the_quantiles_match_the_design_notes_table() {
        let table = [
            (0, 10, ["0.5", "6.1", "23.8"]),
            (0, 20, ["0.2", "3.2", "13.3"]),
            (0, 50, ["0.1", "1.3", "5.7"]),
            (2, 41, ["2.0", "6.3", "14.2"]),
            (6, 41, ["8.1", "15.8", "26.3"]),
        ];
        for (k, n, expected) in table {
            assert_eq!(percent(k, n), expected.map(str::to_owned), "{k} of {n}");
        }
    }

    proptest! {
        #[test]
        fn quantiles_are_ordered_in_range_and_invert_the_distribution(n in 0u64..500, k in 0u64..500) {
            let k = k % (n + 1);
            let (a, b) = (1 + k, 1 + n - k);
            let quantiles = [0.05, 0.5, 0.95].map(|p| (p, beta_quantile(p, a, b)));
            prop_assert!(quantiles[0].1 <= quantiles[1].1 && quantiles[1].1 <= quantiles[2].1);
            for (p, q) in quantiles {
                prop_assert!((0.0..=1.0).contains(&q));
                prop_assert!((beta_cdf(q, a, b) - p).abs() < 1e-6, "{p} at {q}");
            }
        }
    }
}
