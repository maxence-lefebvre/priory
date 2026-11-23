//! News that may end a task's work, such as its Linear issue moved to Duplicate. It reaches the
//! task's live run during its turn, through the `PreToolUse` hook (docs/daemon/routing.md).

use priory_intake::intake::{ISSUE_CLOSED, PR_CLOSED};
use priory_runtime::{jail, runtime};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde_json::Value;
use std::path::PathBuf;

/// The hook's line: `{"news": {"session": ...}}`.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Line {
    pub news: Asked,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Asked {
    pub session: String,
}

/// What an event of `kind` tells a task when it may end its work: its issue canceled, marked a
/// duplicate or done, or its pull request or issue closed without a merge. None otherwise.
pub(crate) fn line(kind: &str, data: &Value) -> Option<String> {
    let text = |field: &str| data[field].as_str().unwrap_or("-");
    let who = data["actor"].as_str().unwrap_or("someone");
    let said = match kind {
        "linear.issue.state_changed"
            if matches!(data["to_type"].as_str(), Some("canceled" | "completed")) =>
        {
            let duplicate = data["duplicate_of"]
                .as_str()
                .map(|of| format!(", as a duplicate of {of}"))
                .unwrap_or_default();
            format!(
                "{who} moved {} to {} ({}){duplicate}",
                text("issue"),
                text("to"),
                text("to_type")
            )
        }
        PR_CLOSED if data["merged"] == false => format!(
            "{who} closed pull request {}#{} without merging it",
            text("repo"),
            data["pull_request"]
        ),
        ISSUE_CLOSED => format!("{who} closed issue {}#{}", text("repo"), data["issue"]),
        _ => return None,
    };
    Some(format!("{said}. {}", text("url")))
}

/// The file whose presence tells the hook of `session` that news waits, so a call costs no socket
/// round trip without it. It lives in the run's spool, which a sandboxed run can read.
pub(crate) fn marker(session: &str) -> PathBuf {
    jail::spool_dir(&runtime::home(), session).join("news")
}

/// Records the news in `message` for the live run of `task`, if it runs, and marks it for the hook.
pub(crate) fn keep(
    tx: &Transaction,
    task: i64,
    seq: i64,
    kind: &str,
    data: &Value,
) -> rusqlite::Result<()> {
    let Some(text) = line(kind, data) else {
        return Ok(());
    };
    let session: Option<String> = tx
        .query_row(
            "SELECT runs.session FROM runs JOIN tasks ON tasks.id = runs.task
             WHERE runs.task = ?1 AND runs.ended IS NULL AND tasks.state = 'running'
             ORDER BY runs.id DESC LIMIT 1",
            [task],
            |row| row.get(0),
        )
        .optional()?;
    let Some(session) = session else {
        return Ok(());
    };
    tx.execute(
        "INSERT OR IGNORE INTO news (run, seq, text)
         SELECT id, ?2, ?3 FROM runs WHERE session = ?1 AND ended IS NULL",
        params![session, seq, text],
    )?;
    // Best effort: a marker that outlives a rolled-back transaction makes the hook ask for nothing.
    let marker = marker(&session);
    if let Some(folder) = marker.parent() {
        let _ = std::fs::create_dir_all(folder);
    }
    let _ = std::fs::write(marker, "");
    Ok(())
}

/// The unread news of `session`'s live run, one line each, now marked read. Empty when none.
pub(crate) fn take(tx: &Transaction, session: &str) -> rusqlite::Result<String> {
    let mut statement = tx.prepare(
        "SELECT news.text FROM news JOIN runs ON runs.id = news.run
         WHERE runs.session = ?1 AND runs.ended IS NULL AND news.read IS NULL ORDER BY news.id",
    )?;
    let lines: Vec<String> = statement
        .query_map([session], |row| row.get(0))?
        .collect::<Result<_, _>>()?;
    tx.execute(
        "UPDATE news SET read = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE read IS NULL
         AND run IN (SELECT id FROM runs WHERE session = ?1 AND ended IS NULL)",
        [session],
    )?;
    let _ = std::fs::remove_file(marker(session));
    Ok(lines.join("\n"))
}

/// The hook's answer: the unread news of the session's live run, now read.
pub(crate) fn asked(
    db: &mut Connection,
    asked: &Asked,
) -> rusqlite::Result<Result<String, String>> {
    let tx = db.transaction()?;
    let lines = take(&tx, &asked.session)?;
    tx.commit()?;
    Ok(Ok(lines))
}

/// The unread news of `session`'s live run as a refusal of the call that would end or sleep the run, now
/// read. None when it has none.
pub(crate) fn refusal(tx: &Transaction, session: &str) -> rusqlite::Result<Option<String>> {
    let lines = take(tx, session)?;
    Ok((!lines.is_empty()).then(|| held(&lines)))
}

/// The reason a call is refused once while news waits.
pub fn held(lines: &str) -> String {
    format!(
        "Priory held this call once: news reached this task while you worked, and it may end the work.\n{lines}\nDecide with it before you open a pull request, push or end the step. Call again if the call still makes sense."
    )
}

#[cfg(test)]
mod tests {
    use super::line;
    use serde_json::json;

    #[test]
    fn a_duplicate_is_news_and_a_move_to_in_review_is_not() {
        let moved = |to: &str, to_type: &str| {
            json!({ "issue": "ENG-7", "to": to, "to_type": to_type, "duplicate_of": "ENG-9",
                    "actor": "Ana", "url": "https://linear.app/acme/issue/ENG-7" })
        };
        assert_eq!(
            line(
                "linear.issue.state_changed",
                &moved("Duplicate", "canceled")
            )
            .as_deref(),
            Some(
                "Ana moved ENG-7 to Duplicate (canceled), as a duplicate of ENG-9. https://linear.app/acme/issue/ENG-7"
            )
        );
        assert_eq!(
            line("linear.issue.state_changed", &moved("In Review", "started")),
            None
        );
        assert_eq!(
            line("linear.issue.state_changed", &json!({ "to": "Canceled" })),
            None
        );
        let closed = |merged: bool| json!({ "repo": "o/r", "pull_request": 7, "merged": merged });
        assert!(line("github.pull_request.closed", &closed(false)).is_some());
        assert_eq!(line("github.pull_request.closed", &closed(true)), None);
    }
}
