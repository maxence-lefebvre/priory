//! Call ids: a line sent again after its reply was lost gets the reply it had, and is applied once.

use rusqlite::{Connection, OptionalExtension, params};

/// `{"call": id, "line": text}`: a line with the id its client gave it.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Envelope {
    pub call: String,
    pub line: String,
}

/// The line `text` carries, with or without its call id: for a reply that changes nothing.
pub(crate) fn inner(text: &str) -> String {
    serde_json::from_str::<Envelope>(text)
        .map_or_else(|_| text.to_owned(), |envelope| envelope.line)
}

/// What a line sent again gets when the daemon took it and stopped before it answered.
pub(crate) const UNANSWERED: &str =
    "the daemon may have applied this call before it stopped: read the state before you retry";

/// The reply stored for `call`, `Some(None)` when it was taken and never answered.
pub(crate) fn stored(db: &Connection, call: &str) -> rusqlite::Result<Option<Option<String>>> {
    db.query_row("SELECT reply FROM replies WHERE call = ?1", [call], |row| {
        row.get(0)
    })
    .optional()
}

/// Notes that `call` is taken, before it is applied, and forgets the replies of over an hour ago.
pub(crate) fn take(db: &Connection, call: &str) -> rusqlite::Result<()> {
    db.execute(
        "DELETE FROM replies WHERE taken < strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-1 hour')",
        [],
    )?;
    db.execute("INSERT INTO replies (call) VALUES (?1)", [call])?;
    Ok(())
}

pub(crate) fn answered(db: &Connection, call: &str, reply: &str) -> rusqlite::Result<()> {
    db.execute(
        "UPDATE replies SET reply = ?2 WHERE call = ?1",
        params![call, reply],
    )?;
    Ok(())
}
