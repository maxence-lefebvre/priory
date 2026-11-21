//! A run's screen for a person to watch: what its agent pane shows, with secrets hidden.

use crate::state;
use priory_runtime::runtime;
use regex::Regex;
use rusqlite::Connection;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::ops::Range;
use std::sync::LazyLock;
use std::time::Duration;

const HIDDEN: &str = "[redacted]";
/// How long tmux may take to show a screen.
pub const DEADLINE: Duration = Duration::from_secs(2);
/// The most a screen's rows may weigh as JSON; past it the top rows are left out.
const MOST: usize = 256 * 1024;

/// Secrets whose shape gives them away: GitHub, Anthropic, OpenAI, AWS, Slack and Linear keys, and
/// PEM private keys, even one cut off at the bottom row.
static SHAPES: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"-----BEGIN [A-Z ]*PRIVATE KEY-----(?s:.*?)(?:-----END [A-Z ]*PRIVATE KEY-----|\z)",
        r"|\b(?:gh[pousr]_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,}",
        r"|sk-[A-Za-z0-9_-]{20,}|AKIA[0-9A-Z]{16}",
        r"|xox[abpr]-[A-Za-z0-9-]{10,}|xapp-[A-Za-z0-9-]{10,}|lin_api_[A-Za-z0-9]{20,})",
    ))
    .expect("the secret shapes compile")
});

/// The values of the environment variables `names`, and the parts of each that are secrets on their
/// own: a URL's password and each value of a `key=value,...` list, each also percent-decoded. Values
/// under 8 bytes are left out, since they would match ordinary text.
pub fn secrets(names: &[String]) -> Vec<String> {
    let values = names.iter().filter_map(|name| std::env::var(name).ok());
    let parts = values.flat_map(|value| {
        let password = value
            .split_once("://")
            .and_then(|(_, rest)| rest.split_once('@'))
            .and_then(|(user, _)| user.split_once(':'))
            .map(|(_, password)| password.to_owned());
        let listed: Vec<_> = value
            .split(',')
            .filter_map(|pair| pair.split_once('=').map(|(_, part)| part.to_owned()))
            .collect();
        let decoded: Vec<_> = password
            .iter()
            .chain(&listed)
            .filter_map(|part| crate::serve::decode(part))
            .collect();
        std::iter::once(value)
            .chain(password)
            .chain(listed)
            .chain(decoded)
    });
    parts.filter(|part| part.len() >= 8).collect()
}

/// `text` with each of the `exact` values and each secret shape replaced by `[redacted]`, and how
/// many it replaced. A hidden span keeps its line breaks, so rows stay rows. A value that
/// `[redacted]` holds cannot be hidden and is skipped. None when a secret still shows after 8 passes.
pub fn redact(text: &str, exact: &[String]) -> Option<(String, usize)> {
    let exact: Vec<_> = exact
        .iter()
        .filter(|secret| !HIDDEN.contains(secret.as_str()))
        .collect();
    let (mut text, mut count) = (text.to_owned(), 0);
    // A replacement can join with its neighbours into a secret again, so repeat until none shows.
    for _ in 0..8 {
        let (hidden, found) = redact_once(&text, &exact);
        if found == 0 {
            return Some((text, count));
        }
        (text, count) = (hidden, count + found);
    }
    (redact_once(&text, &exact).1 == 0).then_some((text, count))
}

fn redact_once(text: &str, exact: &[&String]) -> (String, usize) {
    let mut spans: Vec<Range<usize>> = SHAPES.find_iter(text).map(|found| found.range()).collect();
    for secret in exact {
        let mut at = 0;
        while let Some(found) = text[at..].find(secret.as_str()) {
            let start = at + found;
            spans.push(start..start + secret.len());
            at = start + text[start..].chars().next().map_or(1, char::len_utf8);
        }
    }
    spans.sort_by_key(|span| span.start);
    let mut merged: Vec<Range<usize>> = Vec::new();
    for span in spans {
        match merged.last_mut() {
            Some(last) if span.start < last.end => last.end = last.end.max(span.end),
            _ => merged.push(span),
        }
    }
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for span in &merged {
        out.push_str(&text[at..span.start]);
        out.push_str(HIDDEN);
        out.extend(text[span.clone()].matches('\n'));
        at = span.end;
    }
    out.push_str(&text[at..]);
    (out, merged.len())
}

/// The screen of the live run `target` names, with `exact` and the secret shapes hidden, and a hash
/// of it; or why there is none. `cut` counts the top rows left out to keep it under 256 KiB.
pub fn read(db: &Connection, target: &state::Attach, exact: &[String]) -> Result<Value, String> {
    let run = state::live_run(db, target)?;
    let snapshot =
        runtime::snapshot(&run.tmux, &run.session, DEADLINE).ok_or_else(|| state::no_pane(&run))?;
    let (shown, redacted) = redact(&snapshot.lines.join("\n"), exact)
        .ok_or("a secret on this screen stays visible after redaction, so it is not shown")?;
    let mut lines: Vec<&str> = shown.split('\n').collect();
    let mut weight: usize = lines
        .iter()
        .map(|line| json!(line).to_string().len() + 1)
        .sum();
    let mut cut = 0;
    while weight > MOST && !lines.is_empty() {
        weight -= json!(lines.remove(0)).to_string().len() + 1;
        cut += 1;
    }
    let mut screen = json!({
        "run": run.id,
        "task": run.task,
        "width": snapshot.width,
        "height": snapshot.height,
        "cursor": snapshot.cursor.map(|(x, y)| json!({ "x": x, "y": y })),
        "lines": lines,
        "cut": cut,
        "redacted": redacted,
    });
    let hash: String = Sha256::digest(screen.to_string().as_bytes())[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    screen["hash"] = json!(hash);
    Ok(screen)
}

/// `screen`, or only its hash and `same` when the client already has it.
pub fn reply(screen: Value, have: Option<&str>) -> Value {
    match have {
        Some(have) if screen["hash"] == have => json!({ "hash": have, "same": true }),
        _ => screen,
    }
}

#[cfg(test)]
mod tests {
    use super::{HIDDEN, redact};
    use proptest::prelude::*;

    #[test]
    fn a_secret_the_marker_holds_is_skipped_and_one_that_keeps_coming_back_hides_the_screen() {
        assert_eq!(
            redact("x [redacted] y", &["[redacted]".to_owned()]),
            Some(("x [redacted] y".to_owned(), 0))
        );
        let returning = format!("{}[redacted]", "a".repeat(9));
        assert_eq!(redact(&returning, &["a[redacted]".to_owned()]), None);
        assert_eq!(
            redact("a pem\n-----BEGIN RSA PRIVATE KEY-----\nMIIE\nabc", &[]),
            Some(("a pem\n[redacted]\n\n".to_owned(), 1))
        );
    }

    proptest! {
        #[test]
        fn redact_never_panics_and_leaves_no_secret_on_screen(
            secrets in prop::collection::vec("[a-z\\]\\[ é\n-]{8,12}", 0..3),
            pieces in prop::collection::vec(".{0,12}|ghp_[a-z]{20}|\\[redacted\\]", 0..12),
            picks in prop::collection::vec(0usize..3, 0..12),
        ) {
            let mut text = String::new();
            for (piece, pick) in pieces.iter().zip(picks.iter().chain(std::iter::repeat(&0))) {
                text.push_str(piece);
                if let Some(secret) = secrets.get(*pick) {
                    text.extend(secret.chars().take(4));
                    text.push_str(secret);
                }
            }
            let Some((shown, _)) = redact(&text, &secrets) else { return Ok(()) };
            for secret in secrets.iter().filter(|secret| !HIDDEN.contains(secret.as_str())) {
                prop_assert!(!shown.contains(secret.as_str()), "{secret:?} in {shown:?}");
            }
            prop_assert_eq!(shown.matches('\n').count(), text.matches('\n').count());
        }
    }
}
