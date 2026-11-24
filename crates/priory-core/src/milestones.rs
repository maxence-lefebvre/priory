//! The queue order of a Linear project's tasks: by milestone, then by the issue's own order.

use herdr_bus::Message;
use priory_factory::factory::Factory;
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use std::cmp::Ordering;
use std::collections::BTreeMap;

/// Takes the order of a Linear view's or project's issues from its marker, which the intake
/// publishes whenever one of them changes, and places the new order in the queue.
pub(crate) fn read(db: &Connection, factory: &Factory, message: &Message) -> rusqlite::Result<()> {
    if message.kind == crate::route::TASK_REQUESTED && message.data["order"].is_object() {
        return place(db);
    }
    let stream = message.data["stream"].as_str().unwrap_or_default();
    let ordered = ["linear.view:", "linear.project:"]
        .iter()
        .any(|prefix| stream.starts_with(prefix));
    if message.kind != priory_intake::intake::BASELINE || !ordered {
        return Ok(());
    }
    let triggers: Vec<&str> = factory
        .triggers
        .iter()
        .filter(|(_, trigger)| {
            ["linear.view", "linear.project"].contains(&trigger.value.source.as_str())
        })
        .map(|(name, _)| name.as_str())
        .collect();
    let mut update = db.prepare(
        "UPDATE tasks SET issue_order = ?2 WHERE key = ?1 AND issue_order IS NOT ?2
         AND state NOT IN ('done', 'failed', 'dropped')
         AND trigger IN (SELECT value FROM json_each(?3))",
    )?;
    for member in message.data["members"].as_array().into_iter().flatten() {
        // A member the intake read before it kept the order has none.
        let (Some(issue), Some(order)) = (member["issue"].as_str(), member.get("order")) else {
            continue;
        };
        update.execute(params![
            format!("linear:{issue}"),
            order.is_object().then(|| order.to_string()),
            json!(triggers).to_string()
        ])?;
    }
    place(db)
}

/// Gives the tasks of each project, in each band, the places their ids give them, in milestone
/// order. So they never pass other work. A task a person ordered keeps that place instead.
pub(crate) fn place(db: &Connection) -> rusqlite::Result<()> {
    let mut projects: BTreeMap<(String, String), Vec<(i64, Value)>> = BTreeMap::new();
    let mut before: BTreeMap<i64, Option<i64>> = BTreeMap::new();
    let mut statement = db.prepare(
        "SELECT id, coalesce(band, ''), issue_order, slot, position IS NULL
           AND state NOT IN ('done', 'failed', 'dropped')
         FROM tasks WHERE issue_order IS NOT NULL OR slot IS NOT NULL ORDER BY id",
    )?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let id = row.get(0)?;
        before.insert(id, row.get(3)?);
        let order: Option<String> = row.get(2)?;
        let order: Value = order
            .and_then(|order| serde_json::from_str(&order).ok())
            .unwrap_or_default();
        if let (true, Some(project)) = (row.get(4)?, order["project_id"].as_str()) {
            let project = (row.get(1)?, project.to_owned());
            projects.entry(project).or_default().push((id, order));
        }
    }
    let mut after: BTreeMap<i64, i64> = BTreeMap::new();
    for mut tasks in projects.into_values() {
        let places: Vec<i64> = tasks.iter().map(|(id, _)| *id).collect();
        tasks.sort_by(|(_, a), (_, b)| compare(a, b));
        after.extend(tasks.into_iter().map(|(id, _)| id).zip(places));
    }
    for (id, slot) in before {
        let place = after.get(&id).copied();
        if place != slot {
            db.execute(
                "UPDATE tasks SET slot = ?2 WHERE id = ?1",
                params![id, place],
            )?;
        }
    }
    Ok(())
}

/// Whether issue order `a` comes before `b`: by milestone, with none last, then by the issue.
fn compare(a: &Value, b: &Value) -> Ordering {
    let number = |value: &Value| value.as_f64().unwrap_or(f64::MAX);
    let sorted = |a: &Value, b: &Value| number(a).total_cmp(&number(b));
    let text = |value: &Value| value.as_str().map(str::to_owned);
    let none_last = |a: Option<String>, b: Option<String>| (a.is_none(), a).cmp(&(b.is_none(), b));
    let (ours, theirs) = (&a["milestone"], &b["milestone"]);
    sorted(&ours["sort_order"], &theirs["sort_order"])
        .then_with(|| none_last(text(&ours["target_date"]), text(&theirs["target_date"])))
        .then_with(|| none_last(text(&ours["id"]), text(&theirs["id"])))
        .then_with(|| sorted(&a["sort_order"], &b["sort_order"]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn value() -> impl Strategy<Value = Value> {
        prop_oneof![
            Just(Value::Null),
            any::<f64>().prop_map(|number| json!(number)),
            "\\PC{0,8}".prop_map(|text| json!(text)),
        ]
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        #[test]
        fn orders_of_any_shape_sort_without_panicking(
            orders in proptest::collection::vec((value(), value(), value(), value()), 0..40),
        ) {
            let mut orders: Vec<Value> = orders
                .into_iter()
                .map(|(id, sort, date, own)| json!({
                    "milestone": { "id": id, "sort_order": sort, "target_date": date },
                    "sort_order": own,
                }))
                .collect();
            orders.sort_by(compare);
            for order in &orders {
                compare(order, &Value::Null);
                compare(&order["milestone"], order);
            }
        }
    }
}
