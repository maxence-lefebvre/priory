use super::Window;
use rusqlite::Connection;
use serde_json::{Value, json};

/// A gap between two of the person's acts this long or longer counts as away.
const AWAY_MS: i64 = 2 * 60 * 60 * 1000;

/// `x`, an SQLite time, in milliseconds since the Julian epoch, so times of either format compare.
fn ms(x: &str) -> String {
    format!("CAST(round(julianday({x}) * 86400000) AS INTEGER)")
}

/// A person's act longer than this after their previous one costs only this much attention.
const ACT_MS: i64 = 5 * 60 * 1000;

/// What `priory report attention` can split by.
pub const BY: [&str; 3] = ["workflow", "lane", "kind"];

/// The wait of one revision of an ask, from when it was asked to the person's first act on it.
struct Wait {
    key: String,
    asked: i64,
    acted: Option<i64>,
}

/// A person's act: when, and the split's value for it.
struct Act {
    at: i64,
    key: String,
}

/// A task's current generation, from its creation to its end or the window's end, split by what
/// it waited on.
struct Task {
    id: i64,
    key: String,
    workflow: String,
    lane: Option<String>,
    state: String,
    since: String,
    until: Option<String>,
    wall: i64,
    split: [i64; 4],
}

/// What a task's time went to, in order of precedence: an interval of a higher one hides the
/// lower ones it overlaps. The time no interval covers is queued.
const PERSON: usize = 0;
const WORLD: usize = 1;
const WORKING: usize = 2;
const QUEUED: usize = 3;

/// Where a person's attention went in a window: how long asks waited on them, and how each task's
/// time split.
pub struct Attention {
    since: String,
    until: String,
    start: i64,
    by: String,
    acts: Vec<Act>,
    waits: Vec<Wait>,
    tasks: Vec<Task>,
    merges: i64,
}

/// The SQL value of the split `by` for a row joined to `asks` and `tasks`, `-` for none.
fn key(by: &str) -> Result<&'static str, String> {
    match by {
        "workflow" => Ok("coalesce(tasks.workflow, '-')"),
        "lane" => Ok("coalesce(tasks.lane, '-')"),
        "kind" => Ok("coalesce(asks.kind, '-')"),
        _ => Err(format!("--by {by:?} is none of {}", BY.join(", "))),
    }
}

/// The person's acts up to `until`, oldest first: answers, dismissals, refused answers, snoozes and
/// unsnoozes.
fn acts(db: &Connection, until: &str, key: &str) -> rusqlite::Result<Vec<Act>> {
    let mut statement = db.prepare(&format!(
        "WITH acts AS (
           SELECT answered AS time, ask, NULL AS task FROM ask_revisions WHERE answered IS NOT NULL
           UNION ALL SELECT withdrawn_at, id, NULL FROM asks
             WHERE withdrawn = -1 AND withdrawn_at IS NOT NULL
           UNION ALL SELECT created, ask, NULL FROM ask_refusals
           UNION ALL SELECT time, CASE json_extract(event, '$.data.item.kind') WHEN 'ask'
               THEN CAST(json_extract(event, '$.data.item.id') AS INTEGER) END,
             CAST(json_extract(event, '$.data.task') AS INTEGER) FROM trace
             WHERE type IN ('factory.task.snoozed', 'factory.task.unsnoozed'))
         SELECT {time}, {key} FROM acts LEFT JOIN asks ON asks.id = acts.ask
         LEFT JOIN tasks ON tasks.id = coalesce(asks.task, acts.task)
         WHERE julianday(acts.time) <= julianday(?1) ORDER BY 1",
        time = ms("acts.time"),
    ))?;
    statement
        .query_map([until], |row| {
            Ok(Act {
                at: row.get(0)?,
                key: row.get(1)?,
            })
        })?
        .collect()
}

/// Each revision of an ask asked in `window`, and the person's first act on it before the next
/// revision and up to `until`: its answer, its dismissal or a snooze. A revision that no act ended
/// is kept only while it is the ask's open revision.
fn waits(db: &Connection, window: &Window, key: &str) -> rusqlite::Result<Vec<Wait>> {
    let mut statement = db.prepare(&format!(
        "WITH revisions AS (
           SELECT ask, revision, created, answered, (SELECT min(later.created)
             FROM ask_revisions AS later WHERE later.ask = ask_revisions.ask
             AND later.revision > ask_revisions.revision) AS next
           FROM ask_revisions)
         SELECT {asked}, (SELECT min({time}) FROM (
             SELECT revisions.answered AS time
             UNION ALL SELECT asks.withdrawn_at WHERE asks.withdrawn = -1
             UNION ALL SELECT trace.time FROM trace WHERE trace.type = 'factory.task.snoozed'
               AND json_extract(trace.event, '$.data.item.kind') = 'ask'
               AND json_extract(trace.event, '$.data.item.id') = CAST(asks.id AS TEXT))
             WHERE julianday(time) >= julianday(revisions.created)
             AND (revisions.next IS NULL OR julianday(time) < julianday(revisions.next))
             AND julianday(time) <= julianday(?2)) AS acted,
           asks.revision = revisions.revision AND asks.answer IS NULL AND asks.withdrawn IS NULL,
           {key}
         FROM revisions JOIN asks ON asks.id = revisions.ask
         LEFT JOIN tasks ON tasks.id = asks.task
         WHERE julianday(revisions.created) >= julianday(?1)
         AND julianday(revisions.created) < julianday(?2)
         ORDER BY 1",
        asked = ms("revisions.created"),
        time = ms("time"),
    ))?;
    let rows = statement.query_map([&window.start, &window.until], |row| {
        Ok((
            Wait {
                asked: row.get(0)?,
                acted: row.get(1)?,
                key: row.get(3)?,
            },
            row.get::<_, bool>(2)?,
        ))
    })?;
    let mut waits = Vec::new();
    for row in rows {
        let (wait, open) = row?;
        if wait.acted.is_some() || open {
            waits.push(wait);
        }
    }
    Ok(waits)
}

/// The tasks whose current generation overlaps `window`, each split by the intervals of its runs,
/// open asks, waits and holds. An interval with no stored end ends at the task's next run, else at
/// the task's end.
fn tasks(db: &Connection, window: &Window) -> rusqlite::Result<Vec<Task>> {
    let next_run = |table: &str| {
        format!(
            "(SELECT min(started) FROM runs WHERE runs.task = {table}.task
              AND julianday(runs.started) > julianday({table}.created))"
        )
    };
    let mut intervals = db.prepare(&format!(
        "SELECT kind, {from}, {to} FROM (
           SELECT {WORKING} AS kind, started AS start, ended AS end FROM runs WHERE task = ?1
           UNION ALL SELECT {PERSON}, created, coalesce(answered, withdrawn_at, {ask_run})
             FROM asks WHERE task = ?1
           UNION ALL SELECT {WORLD}, created, coalesce(fired, {wait_run}) FROM waits WHERE task = ?1
           UNION ALL SELECT CASE kind WHEN 'person' THEN {PERSON} ELSE {WORLD} END, created,
             released FROM holds WHERE task = ?1)",
        from = ms("start"),
        to = ms("end"),
        ask_run = next_run("asks"),
        wait_run = next_run("waits"),
    ))?;
    let mut statement = db.prepare(&format!(
        "SELECT id, key, workflow, lane, state, created, ended, {since}, {until} FROM (
           SELECT *, CASE WHEN state IN ('done', 'failed', 'dropped') THEN (SELECT max(ended)
             FROM task_ends WHERE task = tasks.id AND generation = tasks.generation) END AS ended
           FROM tasks)
         WHERE julianday(created) < julianday(?2)
         AND (ended IS NULL OR julianday(ended) >= julianday(?1))
         ORDER BY id",
        since = ms("created"),
        until = ms("coalesce(ended, ?2)"),
    ))?;
    let rows = statement.query_map([&window.start, &window.until], |row| {
        Ok((
            Task {
                id: row.get(0)?,
                key: row.get(1)?,
                workflow: row.get(2)?,
                lane: row.get(3)?,
                state: row.get(4)?,
                since: row.get(5)?,
                until: row.get(6)?,
                wall: 0,
                split: [0; 4],
            },
            row.get::<_, i64>(7)?,
            row.get::<_, i64>(8)?,
        ))
    })?;
    let mut tasks = Vec::new();
    for row in rows {
        let (mut task, since, until) = row?;
        let spans = intervals
            .query_map([task.id], |row| {
                Ok((
                    row.get::<_, i64>(0)? as usize,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Option<i64>>(2)?.unwrap_or(until),
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        task.wall = (until - since).max(0);
        task.split = split(since, until, &spans);
        tasks.push(task);
    }
    Ok(tasks)
}

/// The time from `since` to `until` that each kind of interval in `spans` covers, the kind of
/// highest precedence winning where they overlap.
fn split(since: i64, until: i64, spans: &[(usize, i64, i64)]) -> [i64; 4] {
    let mut cuts: Vec<i64> = spans
        .iter()
        .flat_map(|&(_, from, to)| [from, to])
        .chain([since, until])
        .filter(|cut| (since..=until).contains(cut))
        .collect();
    cuts.sort_unstable();
    cuts.dedup();
    let mut split = [0; 4];
    for pair in cuts.windows(2) {
        let kind = spans
            .iter()
            .filter(|&&(_, from, to)| from <= pair[0] && to >= pair[1])
            .map(|&(kind, ..)| kind)
            .min()
            .unwrap_or(QUEUED);
        split[kind] += pair[1] - pair[0];
    }
    split
}

/// The attention report over the current window of `window`, split by `by`, one of `BY`.
pub fn attention(db: &Connection, window: &Window, by: &str) -> Result<Attention, String> {
    let key = key(by)?;
    let error = |error: rusqlite::Error| error.to_string();
    let (start, merges) = db
        .query_row(
            &format!(
                "{MERGED} SELECT {start}, (SELECT count(*) FROM merged
                   WHERE julianday(time) >= julianday(?1) AND julianday(time) < julianday(?2))",
                MERGED = super::MERGED,
                start = ms("?1"),
            ),
            [&window.start, &window.until],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(error)?;
    Ok(Attention {
        since: window.start.clone(),
        until: window.until.clone(),
        start,
        by: by.to_owned(),
        acts: acts(db, &window.until, key).map_err(error)?,
        waits: waits(db, window, key).map_err(error)?,
        tasks: tasks(db, window).map_err(error)?,
        merges,
    })
}

/// The part of `from..to` that falls in gaps of `AWAY_MS` or more between consecutive `acts`.
fn away(acts: &[Act], from: i64, to: i64) -> i64 {
    acts.windows(2)
        .filter(|pair| pair[1].at - pair[0].at >= AWAY_MS)
        .map(|pair| (to.min(pair[1].at) - from.max(pair[0].at)).max(0))
        .sum()
}

/// A summary of the window's acts and waits, or of those of one value of the split.
struct Summary {
    acts: usize,
    person_minutes: f64,
    acted: usize,
    open: usize,
    /// The median and p90 of the acted waits, with the away time, then without it.
    figures: [Option<f64>; 4],
}

impl Summary {
    fn json(&self) -> Value {
        let [median, p90, median_present, p90_present] = self.figures;
        json!({
            "acted": self.acted, "open": self.open, "median_minutes": median, "p90_minutes": p90,
            "median_present_minutes": median_present, "p90_present_minutes": p90_present,
        })
    }

    /// The acted and open waits and the figures, as text cells.
    fn cells(&self) -> String {
        let figures = self
            .figures
            .map(|figure| figure.map_or_else(|| "-".to_owned(), |minutes| format!("{minutes:.1}")));
        format!("{}\t{}\t{}", self.acted, self.open, figures.join("\t"))
    }
}

/// The nearest-rank `percent` percentile of `sorted`, which is not empty.
fn percentile(sorted: &[f64], percent: f64) -> f64 {
    let rank = (percent / 100.0 * sorted.len() as f64).ceil() as usize;
    sorted[rank.max(1) - 1]
}

fn minutes(ms: i64) -> f64 {
    ms as f64 / 60_000.0
}

/// The median and p90 of `minutes`, or none when it is empty.
fn spread(mut minutes: Vec<f64>) -> [Option<f64>; 2] {
    minutes.sort_by(f64::total_cmp);
    match minutes.is_empty() {
        true => [None, None],
        false => [percentile(&minutes, 50.0), percentile(&minutes, 90.0)].map(Some),
    }
}

impl Attention {
    /// The summary of the acts in the window and the waits, of the split's value `key`, or of all.
    /// Each act costs the time since the person's previous act, up to `ACT_MS`.
    fn summary(&self, key: Option<&str>) -> Summary {
        let of = |other: &str| key.is_none_or(|key| key == other);
        let costs: Vec<_> = self
            .acts
            .iter()
            .enumerate()
            .filter(|(_, act)| act.at >= self.start && of(&act.key))
            .map(|(at, act)| match at {
                0 => ACT_MS,
                _ => (act.at - self.acts[at - 1].at).min(ACT_MS),
            })
            .collect();
        let acted: Vec<_> = self
            .waits
            .iter()
            .filter(|wait| of(&wait.key))
            .filter_map(|wait| wait.acted.map(|acted| (wait.asked, acted)))
            .collect();
        let whole = spread(
            acted
                .iter()
                .map(|(asked, acted)| minutes(acted - asked))
                .collect(),
        );
        let present = spread(
            acted
                .iter()
                .map(|&(asked, acted)| minutes(acted - asked - away(&self.acts, asked, acted)))
                .collect(),
        );
        Summary {
            acts: costs.len(),
            person_minutes: minutes(costs.iter().sum()),
            acted: acted.len(),
            open: self.waits.iter().filter(|wait| of(&wait.key)).count() - acted.len(),
            figures: [whole[0], whole[1], present[0], present[1]],
        }
    }

    /// Each value of the split that an act or a wait has, sorted, with its summary.
    fn split(&self) -> Vec<(&str, Summary)> {
        let mut keys: Vec<&str> = self
            .acts
            .iter()
            .filter(|act| act.at >= self.start)
            .map(|act| act.key.as_str())
            .chain(self.waits.iter().map(|wait| wait.key.as_str()))
            .collect();
        keys.sort_unstable();
        keys.dedup();
        keys.into_iter()
            .map(|key| (key, self.summary(Some(key))))
            .collect()
    }

    /// The person-minutes of the window per merge in it, or none without a merge.
    fn per_merge(&self, all: &Summary) -> Option<f64> {
        (self.merges > 0).then(|| all.person_minutes / self.merges as f64)
    }

    pub fn json(&self) -> Value {
        let all = self.summary(None);
        json!({
            "since": self.since, "until": self.until, "by": self.by, "acts": all.acts,
            "person_minutes": all.person_minutes, "merges": self.merges,
            "person_minutes_per_merge": self.per_merge(&all),
            "waits": all.json(),
            "split": self.split().into_iter().map(|(key, summary)| json!({
                &self.by: key, "acts": summary.acts, "person_minutes": summary.person_minutes,
                "waits": summary.json(),
            })).collect::<Vec<_>>(),
            "tasks": self.tasks.iter().map(|task| json!({
                "id": task.id, "key": task.key, "workflow": task.workflow, "lane": task.lane,
                "state": task.state, "since": task.since, "until": task.until,
                "wall_minutes": minutes(task.wall),
                "working_minutes": minutes(task.split[WORKING]),
                "person_wait_minutes": minutes(task.split[PERSON]),
                "world_wait_minutes": minutes(task.split[WORLD]),
                "queued_minutes": minutes(task.split[QUEUED]),
            })).collect::<Vec<_>>(),
        })
    }

    pub fn print(&self) {
        let all = self.summary(None);
        let per_merge = self
            .per_merge(&all)
            .map_or_else(|| "-".to_owned(), |minutes| format!("{minutes:.1}"));
        crate::state::emit("since\tuntil\tacts\tperson minutes\tmerges\tper merge");
        crate::state::emit(&format!(
            "{}\t{}\t{}\t{:.1}\t{}\t{per_merge}",
            self.since, self.until, all.acts, all.person_minutes, self.merges
        ));
        crate::state::emit("");
        crate::state::emit("asks\tacted\topen\tmedian\tp90\tmedian present\tp90 present");
        crate::state::emit(&format!("all\t{}", all.cells()));
        crate::state::emit("");
        crate::state::emit(&format!(
            "{}\tacts\tperson minutes\tacted\topen\tmedian\tp90\tmedian present\tp90 present",
            self.by
        ));
        for (key, summary) in self.split() {
            crate::state::emit(&format!(
                "{key}\t{}\t{:.1}\t{}",
                summary.acts,
                summary.person_minutes,
                summary.cells()
            ));
        }
        crate::state::emit("");
        crate::state::emit(
            "task\tkey\tworkflow\tlane\tstate\twall\tworking\twaiting on person\twaiting on world\tqueued",
        );
        for task in &self.tasks {
            let times = [
                task.wall,
                task.split[WORKING],
                task.split[PERSON],
                task.split[WORLD],
            ]
            .into_iter()
            .chain([task.split[QUEUED]])
            .map(|time| format!("{:.1}", minutes(time)))
            .collect::<Vec<_>>()
            .join("\t");
            crate::state::emit(&format!(
                "{}\t{}\t{}\t{}\t{}\t{times}",
                task.id,
                task.key,
                task.workflow,
                task.lane.as_deref().unwrap_or("-"),
                task.state
            ));
        }
    }
}
