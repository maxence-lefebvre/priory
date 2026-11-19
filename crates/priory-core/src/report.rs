use priory_factory::factory::Price;
use rusqlite::Connection;
use std::collections::BTreeMap;

/// The window a report covers when it names none.
pub const SINCE: &str = "7 days";

/// The start of the window `since`: an RFC 3339 instant, or a duration back from now such as
/// `7 days`, as an SQLite date modifier.
fn start(db: &Connection, since: &str) -> Result<String, String> {
    let start: Option<String> = db
        .query_row(
            "SELECT coalesce(strftime('%Y-%m-%dT%H:%M:%fZ', ?1),
               strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '-' || ?1))",
            [since],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    start.ok_or_else(|| {
        format!("--since {since:?} is neither an RFC 3339 instant nor a duration such as 7 days")
    })
}

/// Prints each step's outcome counts for `workflow` since `since`. Each rate's denominator is the
/// visits that reached the step: those that ended it, and active tasks still at it.
/// With `by_lane`, each lane is counted apart, by the lane its task has now.
pub fn outcomes(db: &Connection, workflow: &str, since: &str, by_lane: bool) -> Result<(), String> {
    let start = start(db, since)?;
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
    let start = start(db, since)?;
    let mut statement = db
        .prepare(
            "SELECT json_extract(event, '$.data.task') AS task,
               iif(type = 'factory.handler.called', 'handler', json_extract(event, '$.data.step')) AS step,
               coalesce(json_extract(event, '$.data.model'), '-') AS model,
               sum(json_extract(event, '$.data.tokens.input')),
               sum(json_extract(event, '$.data.tokens.output')),
               sum(json_extract(event, '$.data.tokens.cache_read')),
               sum(json_extract(event, '$.data.tokens.cache_write')),
               count(json_extract(event, '$.data.tokens')) = count(*)
             FROM trace WHERE json_extract(event, '$.data.workflow') = ?1 AND time >= ?2
             AND (type = 'factory.handler.called' OR (type = 'factory.step.ended'
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
