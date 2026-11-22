//! Which skills and memories with `paths` a run already got from its `PreToolUse` hook.

use rusqlite::{Connection, OptionalExtension, params};

/// The hook's line: `{"scoped": {"session": ..., "files": [...]}}`.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Line {
    pub scoped: Matched,
}

/// The files whose globs match a path the run is about to read or edit.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Matched {
    pub session: String,
    pub files: Vec<String>,
}

/// Records each of `matched.files` against the session's live run, and replies with those the run
/// had not got yet, one per line. A session with no live run gets them all, recorded nowhere.
pub(crate) fn sent(
    db: &mut Connection,
    matched: &Matched,
) -> rusqlite::Result<Result<String, String>> {
    let run: Option<i64> = db
        .query_row(
            "SELECT id FROM runs WHERE session = ?1 AND ended IS NULL ORDER BY id DESC LIMIT 1",
            [&matched.session],
            |row| row.get(0),
        )
        .optional()?;
    let Some(run) = run else {
        return Ok(Ok(matched.files.join("\n")));
    };
    let tx = db.transaction()?;
    let mut new = Vec::new();
    for file in &matched.files {
        let added = tx.execute(
            "INSERT OR IGNORE INTO scoped_sent (run, file) VALUES (?1, ?2)",
            params![run, file],
        )?;
        if added == 1 {
            new.push(file.as_str());
        }
    }
    tx.commit()?;
    Ok(Ok(new.join("\n")))
}

#[cfg(test)]
mod tests {
    use super::{Matched, sent};

    #[test]
    fn a_session_with_no_live_run_gets_every_file() {
        let mut db = crate::state::memory();
        let matched = Matched {
            session: "none".into(),
            files: vec!["/a".into(), "/b".into()],
        };
        assert_eq!(sent(&mut db, &matched).unwrap(), Ok("/a\n/b".into()));
    }
}
