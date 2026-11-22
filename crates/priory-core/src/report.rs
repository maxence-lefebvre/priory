use priory_factory::factory::Price;
use rusqlite::Connection;
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub mod attention;

/// The window a report covers when it names none.
pub const SINCE: &str = "7 days";

/// The start of the window `since`: an RFC 3339 instant, or a duration back from `now` such as
/// `7 days`, in jiff's friendly or ISO 8601 form. `now` is an RFC 3339 instant or `now`. Calendar
/// units count back in UTC.
pub fn start(db: &Connection, since: &str, now: &str) -> Result<String, String> {
    let refused = || {
        format!("--since {since:?} is neither an RFC 3339 instant nor a duration such as 7 days")
    };
    let instant = match since.parse::<jiff::Timestamp>() {
        Ok(instant) => instant,
        Err(_) => {
            let span = since.parse::<jiff::Span>().map_err(|_| refused())?;
            if span.is_negative() {
                return Err(refused());
            }
            let now = match now {
                "now" => jiff::Timestamp::now(),
                now => now
                    .parse()
                    .map_err(|error| format!("now {now:?}: {error}"))?,
            };
            now.to_zoned(jiff::tz::TimeZone::UTC)
                .checked_sub(span)
                .map_err(|_| refused())?
                .timestamp()
        }
    };
    db.query_row(
        "SELECT strftime('%Y-%m-%dT%H:%M:%fZ', ?1)",
        [instant.to_string()],
        |row| row.get::<_, Option<String>>(0),
    )
    .map_err(|error| error.to_string())?
    .ok_or_else(refused)
}

/// Prints each step's outcome counts for `workflow` since `since`. Each rate's denominator is the
/// visits that reached the step: those that ended it, and active tasks still at it.
/// With `by_lane`, each lane is counted apart, by the lane its task has now.
pub fn outcomes(db: &Connection, workflow: &str, since: &str, by_lane: bool) -> Result<(), String> {
    let start = start(db, since, "now")?;
    let lane = |table: &str| {
        if by_lane {
            format!("coalesce({table}.lane, '-')")
        } else {
            "''".to_owned()
        }
    };
    let mut statement = db
        .prepare(&format!(
            "WITH ended AS (
               SELECT json_extract(trace.event, '$.data.step') AS step,
                 json_extract(trace.event, '$.data.outcome') AS outcome, {lane} AS lane
               FROM trace LEFT JOIN tasks
                 ON tasks.id = CAST(json_extract(trace.event, '$.data.task') AS INTEGER)
               WHERE trace.type = 'factory.step.ended'
               AND json_extract(trace.event, '$.data.workflow') = ?1 AND trace.time >= ?2)
             SELECT lane, step, outcome, count(*), (SELECT count(*) FROM ended AS reached
               WHERE reached.step = ended.step AND reached.lane = ended.lane)
               + (SELECT count(*) FROM tasks AS active
               WHERE active.workflow = ?1 AND active.step = ended.step AND active.entered >= ?2
               AND {active} = ended.lane
               AND active.state NOT IN ('done', 'failed', 'dropped'))
             FROM ended GROUP BY lane, step, outcome ORDER BY lane, step, outcome",
            lane = lane("tasks"),
            active = lane("active"),
        ))
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([workflow, &start], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })
        .and_then(Iterator::collect::<rusqlite::Result<Vec<_>>>)
        .map_err(|error| error.to_string())?;
    let lane_column = if by_lane { "lane\t" } else { "" };
    crate::state::emit(&format!("{lane_column}step\toutcome\tcount\treached\trate"));
    for (lane, step, outcome, count, reached) in rows {
        let rate = 100.0 * count as f64 / reached as f64;
        let lane = if by_lane { format!("{lane}\t") } else { lane };
        crate::state::emit(&format!(
            "{lane}{step}\t{outcome}\t{count}\t{reached}\t{rate:.1}%"
        ));
    }
    Ok(())
}

/// Tokens of each kind, as a step record counts them.
#[derive(Default)]
struct Tokens([i64; 4]);

impl Tokens {
    fn add(&mut self, other: &Tokens) {
        for (sum, more) in self.0.iter_mut().zip(other.0) {
            *sum += more;
        }
    }

    fn dollars(&self, price: &Price) -> f64 {
        let rates = [
            price.input,
            price.output,
            price.cache_read,
            price.cache_write,
        ];
        self.0
            .iter()
            .zip(rates)
            .map(|(tokens, rate)| *tokens as f64 * rate / 1_000_000.0)
            .sum()
    }

    fn cells(&self) -> String {
        self.0.map(|tokens| tokens.to_string()).join("\t")
    }
}

/// Prints the tokens and cost of each task of `workflow` since `since`, per model step, then in
/// total. A model missing from `prices` keeps its tokens and names the missing price instead of a
/// cost. A step whose tokens are unknown says so, and so does its task's total.
pub fn cost(
    db: &Connection,
    workflow: &str,
    since: &str,
    prices: &BTreeMap<String, Price>,
) -> Result<(), String> {
    let start = start(db, since, "now")?;
    let mut statement = db
        .prepare(
            "SELECT json_extract(event, '$.data.task') AS task,
               CASE type WHEN 'factory.handler.called' THEN 'handler'
                 WHEN 'factory.summarizer.called' THEN 'summarizer'
                 ELSE json_extract(event, '$.data.step') END AS step,
               coalesce(json_extract(event, '$.data.model'), '-') AS model,
               sum(json_extract(event, '$.data.tokens.input')),
               sum(json_extract(event, '$.data.tokens.output')),
               sum(json_extract(event, '$.data.tokens.cache_read')),
               sum(json_extract(event, '$.data.tokens.cache_write')),
               count(json_extract(event, '$.data.tokens')) = count(*)
             FROM trace WHERE json_extract(event, '$.data.workflow') = ?1 AND time >= ?2
             AND (type IN ('factory.handler.called', 'factory.summarizer.called')
               OR (type = 'factory.step.ended'
               AND json_extract(event, '$.data.decider') = 'model'))
             GROUP BY task, step, model ORDER BY CAST(task AS INTEGER), min(seq)",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([workflow, &start], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, bool>(7)?
                    .then(|| {
                        Ok::<_, rusqlite::Error>(Tokens([
                            row.get(3)?,
                            row.get(4)?,
                            row.get(5)?,
                            row.get(6)?,
                        ]))
                    })
                    .transpose()?,
            ))
        })
        .and_then(Iterator::collect::<rusqlite::Result<Vec<_>>>)
        .map_err(|error| error.to_string())?;
    crate::state::emit("task\tstep\tmodel\tinput\toutput\tcache_read\tcache_write\tcost");
    let mut rows = rows.into_iter().peekable();
    while let Some((task, ..)) = rows.peek() {
        let task = task.clone();
        let mut total = Tokens::default();
        let mut dollars = 0.0;
        let mut missing = Vec::new();
        let mut unknown = Vec::new();
        while let Some((_, step, model, tokens)) = rows.next_if(|row| row.0 == task) {
            let Some(tokens) = tokens else {
                crate::state::emit(&format!(
                    "{task}\t{step}\t{model}\t-\t-\t-\t-\ttokens unknown"
                ));
                unknown.push(step);
                continue;
            };
            let cost = match prices.get(&model) {
                Some(price) => {
                    dollars += tokens.dollars(price);
                    format!("${:.6}", tokens.dollars(price))
                }
                None => {
                    if !missing.contains(&model) {
                        missing.push(model.clone());
                    }
                    format!("price missing for {model}")
                }
            };
            crate::state::emit(&format!(
                "{task}\t{step}\t{model}\t{}\t{cost}",
                tokens.cells()
            ));
            total.add(&tokens);
        }
        let cost = if !unknown.is_empty() {
            format!("tokens unknown for {}", unknown.join(", "))
        } else if !missing.is_empty() {
            format!("price missing for {}", missing.join(", "))
        } else {
            format!("${dollars:.6}")
        };
        crate::state::emit(&format!("{task}\ttotal\t-\t{}\t{cost}", total.cells()));
    }
    Ok(())
}

/// What `priory costs` can split the current window's cost by.
pub const BY: [&str; 6] = ["workflow", "agent", "model", "machine", "repo", "lane"];

/// The cost of a set of records: priced tokens in dollars, the tokens of models without a price
/// apart, and the records whose tokens are unknown.
#[derive(Default)]
struct Total {
    priced: bool,
    dollars: f64,
    tokens: Tokens,
    unpriced: BTreeMap<String, Tokens>,
    unknown: i64,
    merges: i64,
}

impl Total {
    fn add(&mut self, model: &str, tokens: Option<&Tokens>, prices: &BTreeMap<String, Price>) {
        let Some(tokens) = tokens else {
            self.unknown += 1;
            return;
        };
        self.tokens.add(tokens);
        match prices.get(model) {
            Some(price) => {
                self.priced = true;
                self.dollars += tokens.dollars(price);
            }
            None => self
                .unpriced
                .entry(model.to_owned())
                .or_default()
                .add(tokens),
        }
    }

    /// The dollars, or none when no record was priced and some were not, so unpriced tokens never
    /// read as a zero cost.
    fn cost(&self) -> Option<f64> {
        (self.priced || (self.unpriced.is_empty() && self.unknown == 0)).then_some(self.dollars)
    }

    fn json(&self) -> Value {
        json!({
            "cost_usd": self.cost(),
            "tokens": self.tokens.json(),
            "unpriced": self.unpriced.iter()
                .map(|(model, tokens)| json!({ "model": model, "tokens": tokens.json() }))
                .collect::<Vec<_>>(),
            "unknown": self.unknown,
        })
    }

    /// The cost cell and the notes cell of a text row.
    fn cells(&self) -> String {
        let cost = self
            .cost()
            .map_or_else(|| "-".to_owned(), |dollars| format!("${dollars:.6}"));
        let mut notes: Vec<_> = self
            .unpriced
            .iter()
            .map(|(model, tokens)| {
                format!(
                    "price missing for {model}: {} tokens",
                    tokens.0.iter().sum::<i64>()
                )
            })
            .collect();
        if self.unknown > 0 {
            notes.push(format!("tokens unknown for {} records", self.unknown));
        }
        let notes = if notes.is_empty() {
            "-".to_owned()
        } else {
            notes.join("; ")
        };
        format!("{cost}\t{notes}")
    }
}

impl Tokens {
    fn json(&self) -> Value {
        let [input, output, cache_read, cache_write] = self.0;
        json!({ "input": input, "output": output, "cache_read": cache_read,
                "cache_write": cache_write })
    }
}

/// The current window against the one before it, day by day, and the current window split by one
/// of `BY`.
pub struct Costs {
    by: String,
    windows: [(String, String, Total); 2],
    /// The local date each window starts on, as the days are keyed.
    since_days: [String; 2],
    days: BTreeMap<String, Total>,
    split: BTreeMap<String, Total>,
    /// Each split value and scorer with its runs' latest verdicts in the current window: pass, fail,
    /// error and not applicable.
    scores: Vec<(String, String, [i64; 4])>,
}

/// The current window, from `start` to `until`, and the previous one, from `previous` to `start`.
pub struct Window {
    until: String,
    start: String,
    previous: String,
}

/// The windows up to `now`. With no `since`, the current window starts this Monday at 00:00 local
/// time and the previous one is last week; otherwise the previous window has the current one's
/// length. Refuses a `since` that does not parse or does not start before `now`.
pub fn window(db: &Connection, since: Option<&str>, now: &str) -> Result<Window, String> {
    let row = |row: &rusqlite::Row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<String>>(2)?,
        ))
    };
    let (until, start, previous) = match since {
        None => db.query_row(
            "SELECT strftime('%Y-%m-%dT%H:%M:%fZ', ?1),
               strftime('%Y-%m-%dT%H:%M:%fZ', ?1, 'localtime', 'start of day', '-6 days',
                 'weekday 1', 'utc'),
               strftime('%Y-%m-%dT%H:%M:%fZ', ?1, 'localtime', 'start of day', '-6 days',
                 'weekday 1', '-7 days', 'utc')",
            [now],
            row,
        ),
        Some(since) => {
            let start = start(db, since, now)?;
            db.query_row(
                "SELECT strftime('%Y-%m-%dT%H:%M:%fZ', ?1), ?2,
                   strftime('%Y-%m-%dT%H:%M:%fZ', 2 * julianday(?2) - julianday(?1))",
                [now, &start],
                row,
            )
        }
    }
    .map_err(|error| error.to_string())?;
    match previous {
        Some(previous) if start < until || since.is_none() => Ok(Window {
            until,
            start,
            previous,
        }),
        _ => Err(format!(
            "--since {:?} does not start before now, or its previous window is out of range",
            since.unwrap_or_default()
        )),
    }
}

/// The merges of pull requests a task owns or owned the head branch of, each at its first merged
/// close, as the `merged` table of a `WITH` clause.
const MERGED: &str = "WITH merged AS (
  SELECT min(strftime('%Y-%m-%dT%H:%M:%fZ', time)) AS time FROM inbox
  WHERE type = 'github.pull_request.closed' AND json_extract(data, '$.merged') = 1
  AND EXISTS (SELECT 1 FROM owns WHERE owns.resource IN ('github:' || inbox.subject,
    'github:' || json_extract(inbox.data, '$.repo') || '@'
      || json_extract(inbox.data, '$.head_branch')))
  GROUP BY subject)";

/// The costs of model steps, handler calls and summarizer calls in `window` and the one before
/// it, priced from `prices`, with the merges of owned pull requests. Days are local.
pub fn costs(
    db: &Connection,
    prices: &BTreeMap<String, Price>,
    window: &Window,
    by: &str,
) -> Result<Costs, String> {
    let key = match by {
        "workflow" => "coalesce(json_extract(trace.event, '$.data.workflow'), '-')",
        "agent" => {
            "CASE trace.type WHEN 'factory.handler.called' THEN 'handler'
               WHEN 'factory.summarizer.called' THEN 'summarizer'
               ELSE coalesce(runs.agent, '-') END"
        }
        "model" => "coalesce(json_extract(trace.event, '$.data.model'), '-')",
        "machine" => "coalesce(json_extract(trace.event, '$.data.machine'), '-')",
        "repo" => "coalesce(tasks.repo, '-')",
        "lane" => "coalesce(tasks.lane, '-')",
        _ => return Err(format!("--by {by:?} is none of {}", BY.join(", "))),
    };
    let error = |error: rusqlite::Error| error.to_string();
    let Window {
        until,
        start,
        previous,
    } = window;
    let local_date = |at: &str| {
        db.query_row("SELECT date(?1, 'localtime')", [at], |row| row.get(0))
            .map_err(error)
    };
    let mut costs = Costs {
        by: by.to_owned(),
        windows: [
            (start.clone(), until.clone(), Total::default()),
            (previous.clone(), start.clone(), Total::default()),
        ],
        since_days: [local_date(start)?, local_date(previous)?],
        days: BTreeMap::new(),
        split: BTreeMap::new(),
        scores: Vec::new(),
    };
    let mut statement = db
        .prepare(&format!(
            "SELECT date(trace.time, 'localtime'), trace.time >= ?2, {key},
               coalesce(json_extract(trace.event, '$.data.model'), '-'),
               json_extract(trace.event, '$.data.tokens') IS NOT NULL,
               coalesce(json_extract(trace.event, '$.data.tokens.input'), 0),
               coalesce(json_extract(trace.event, '$.data.tokens.output'), 0),
               coalesce(json_extract(trace.event, '$.data.tokens.cache_read'), 0),
               coalesce(json_extract(trace.event, '$.data.tokens.cache_write'), 0)
             FROM trace
             LEFT JOIN tasks ON tasks.id = CAST(json_extract(trace.event, '$.data.task') AS INTEGER)
             LEFT JOIN runs ON runs.id = CAST(json_extract(trace.event, '$.data.run') AS INTEGER)
             WHERE trace.time >= ?1 AND trace.time < ?3
             AND (trace.type IN ('factory.handler.called', 'factory.summarizer.called')
               OR (trace.type = 'factory.step.ended'
               AND json_extract(trace.event, '$.data.decider') = 'model'))"
        ))
        .map_err(error)?;
    let mut rows = statement.query([previous, start, until]).map_err(error)?;
    while let Some(row) = rows.next().map_err(error)? {
        let (day, current, key, model): (String, bool, String, String) = (
            row.get(0).map_err(error)?,
            row.get(1).map_err(error)?,
            row.get(2).map_err(error)?,
            row.get(3).map_err(error)?,
        );
        let tokens = match row.get::<_, bool>(4).map_err(error)? {
            true => Some(Tokens([
                row.get(5).map_err(error)?,
                row.get(6).map_err(error)?,
                row.get(7).map_err(error)?,
                row.get(8).map_err(error)?,
            ])),
            false => None,
        };
        let tokens = tokens.as_ref();
        costs.windows[usize::from(!current)]
            .2
            .add(&model, tokens, prices);
        costs
            .days
            .entry(day)
            .or_default()
            .add(&model, tokens, prices);
        if current {
            costs
                .split
                .entry(key)
                .or_default()
                .add(&model, tokens, prices);
        }
    }
    let mut statement = db
        .prepare(&format!(
            "{MERGED} SELECT date(time, 'localtime'), time >= ?2, count(*) FROM merged
             WHERE time >= ?1 AND time < ?3 GROUP BY 1, 2"
        ))
        .map_err(error)?;
    let merges = statement
        .query_map([previous, start, until], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, bool>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })
        .and_then(Iterator::collect::<rusqlite::Result<Vec<_>>>)
        .map_err(error)?;
    for (day, current, merged) in merges {
        costs.windows[usize::from(!current)].2.merges += merged;
        costs.days.entry(day).or_default().merges += merged;
    }
    // A score names no model, so it takes its run's.
    let key = match by {
        "model" => {
            "coalesce((SELECT json_extract(step.event, '$.data.model') FROM trace AS step
               WHERE step.type = 'factory.step.ended'
                 AND json_extract(step.event, '$.data.run') = json_extract(trace.event, '$.data.run')
               ORDER BY step.seq DESC LIMIT 1), '-')"
        }
        _ => key,
    };
    let mut statement = db
        .prepare(&format!(
            "SELECT key, scorer, sum(verdict = 'pass'), sum(verdict = 'fail'),
               sum(verdict = 'error'), sum(verdict = 'not applicable')
             FROM (SELECT {key} AS key, json_extract(trace.event, '$.data.scorer') AS scorer,
               json_extract(trace.event, '$.data.verdict') AS verdict,
               row_number() OVER (PARTITION BY json_extract(trace.event, '$.data.run'),
                 json_extract(trace.event, '$.data.scorer') ORDER BY trace.seq DESC) AS latest
             FROM trace
             LEFT JOIN tasks ON tasks.id = CAST(json_extract(trace.event, '$.data.task') AS INTEGER)
             LEFT JOIN runs ON runs.id = CAST(json_extract(trace.event, '$.data.run') AS INTEGER)
             WHERE trace.type = 'factory.run.scored' AND trace.time >= ?1 AND trace.time < ?2)
             WHERE latest = 1 GROUP BY 1, 2 ORDER BY 1, 2"
        ))
        .map_err(error)?;
    costs.scores = statement
        .query_map([start, until], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                [row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?],
            ))
        })
        .and_then(Iterator::collect)
        .map_err(error)?;
    Ok(costs)
}

impl Costs {
    pub fn json(&self) -> Value {
        let window = |at: usize| {
            let (since, until, total) = &self.windows[at];
            let mut json = total.json();
            json["since"] = json!(since);
            json["since_day"] = json!(self.since_days[at]);
            json["until"] = json!(until);
            json["merges"] = json!(total.merges);
            json
        };
        json!({
            "by": self.by,
            "current": window(0),
            "previous": window(1),
            "days": self.days.iter().map(|(day, total)| {
                let mut json = total.json();
                json["day"] = json!(day);
                json["merges"] = json!(total.merges);
                json
            }).collect::<Vec<_>>(),
            "split": self.split.iter().map(|(key, total)| {
                let mut json = total.json();
                json[&self.by] = json!(key);
                json
            }).collect::<Vec<_>>(),
            "scores": self.scores.iter().map(|(key, scorer, [pass, fail, error, not_applicable])| {
                json!({ (&self.by): key, "scorer": scorer, "pass": pass, "fail": fail,
                        "error": error, "not_applicable": not_applicable })
            }).collect::<Vec<_>>(),
        })
    }

    pub fn print(&self) {
        crate::state::emit("period\tsince\tuntil\tcost\tnotes\tmerges");
        for ((since, until, total), period) in self.windows.iter().zip(["current", "previous"]) {
            crate::state::emit(&format!(
                "{period}\t{since}\t{until}\t{}\t{}",
                total.cells(),
                total.merges
            ));
        }
        crate::state::emit("");
        crate::state::emit("day\tcost\tnotes\tmerges");
        for (day, total) in &self.days {
            crate::state::emit(&format!("{day}\t{}\t{}", total.cells(), total.merges));
        }
        crate::state::emit("");
        crate::state::emit(&format!("{}\tcost\tnotes", self.by));
        for (key, total) in &self.split {
            crate::state::emit(&format!("{key}\t{}", total.cells()));
        }
        crate::state::emit("");
        crate::state::emit(&format!(
            "{}\tscorer\tpass\tfail\terror\tnot_applicable",
            self.by
        ));
        for (key, scorer, [pass, fail, error, not_applicable]) in &self.scores {
            crate::state::emit(&format!(
                "{key}\t{scorer}\t{pass}\t{fail}\t{error}\t{not_applicable}"
            ));
        }
    }
}
