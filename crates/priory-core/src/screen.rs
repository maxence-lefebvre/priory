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
/// The most rows of scrollback a screen shows above its visible rows.
pub const HISTORY: usize = 2000;
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

/// An SGR colour code, which `capture-pane -e` writes into a row's text and the page draws.
static SGR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\x1b\[[0-9;:]*m").expect("the SGR code compiles"));
/// An OSC 8 link's start or end, the other escape `capture-pane -e` writes; the page shows no links.
static LINK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\x1b\]8;[^\x07\x1b]*(?:\x07|\x1b\\)").expect("the link code compiles")
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
/// many it replaced. A hidden span keeps its line breaks, so rows stay rows. Secrets are found in
/// the text without its SGR colour codes, so a code cannot split one; a code inside a hidden span
/// moves after it. A value that `[redacted]` holds cannot be hidden and is skipped. None when a
/// secret still shows after 8 passes.
pub fn redact(text: &str, exact: &[String]) -> Option<(String, usize)> {
    let exact: Vec<_> = exact
        .iter()
        .filter(|secret| !HIDDEN.contains(secret.as_str()))
        .collect();
    let (mut plain, mut codes, mut at) = (String::new(), Vec::new(), 0);
    for code in SGR.find_iter(text) {
        plain.push_str(&text[at..code.start()]);
        codes.push((plain.len(), code.as_str()));
        at = code.end();
    }
    plain.push_str(&text[at..]);
    let mut count = 0;
    // A replacement can join with its neighbours into a secret again, so repeat until none shows.
    for pass in 0..=8 {
        let spans = secret_spans(&plain, &exact);
        if spans.is_empty() {
            let mut out = String::with_capacity(text.len());
            let mut at = 0;
            for (offset, code) in &codes {
                out.push_str(&plain[at..*offset]);
                out.push_str(code);
                at = *offset;
            }
            out.push_str(&plain[at..]);
            return Some((out, count));
        }
        if pass == 8 {
            return None;
        }
        plain = hide(&plain, &spans, &mut codes);
        count += spans.len();
    }
    unreachable!("the last pass returns")
}

/// Where `text` shows a secret shape or one of the `exact` values, overlaps merged, in order.
fn secret_spans(text: &str, exact: &[&String]) -> Vec<Range<usize>> {
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
    merged
}

/// `text` with each of `spans` replaced by `[redacted]` and its line breaks, and each code's offset
/// moved to match: a code inside a span lands right after its replacement.
fn hide(text: &str, spans: &[Range<usize>], codes: &mut [(usize, &str)]) -> String {
    let mut out = String::with_capacity(text.len());
    let (mut at, mut next) = (0, codes.iter_mut().peekable());
    for span in spans {
        while let Some(code) = next.next_if(|code| code.0 <= span.start) {
            code.0 = out.len() + code.0 - at;
        }
        out.push_str(&text[at..span.start]);
        out.push_str(HIDDEN);
        out.extend(text[span.clone()].matches('\n'));
        while let Some(code) = next.next_if(|code| code.0 < span.end) {
            code.0 = out.len();
        }
        at = span.end;
    }
    for code in next {
        code.0 = out.len() + code.0 - at;
    }
    out.push_str(&text[at..]);
    out
}

/// The screen of the live run `target` names after up to `history` rows of its scrollback, with
/// `exact` and the secret shapes hidden, its colour codes kept and its links dropped, and a hash of
/// it; or why there is none. `cut` counts the top rows left out to keep it under 256 KiB.
pub fn read(
    db: &Connection,
    target: &state::Attach,
    exact: &[String],
    history: usize,
) -> Result<Value, String> {
    let run = state::live_run(db, target)?;
    let snapshot = runtime::snapshot(&run.tmux, &run.session, history, DEADLINE)
        .ok_or_else(|| state::no_pane(&run))?;
    let rows = LINK.replace_all(&snapshot.lines.join("\n"), "").into_owned();
    let (shown, redacted) = redact(&rows, exact)
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
        "earlier": snapshot.earlier.saturating_sub(cut),
        "lines": lines,
        "cut": cut,
        "redacted": redacted,
    });
    let hash: String = Sha256::digest(screen.to_string().as_bytes())[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    screen["hash"] = json!(hash);
    // Left out of the hash: a clear pushes the screen into scrollback, which a redraw does not change.
    screen["scrollback"] = json!(snapshot.scrollback);
    Ok(screen)
}

/// A row without its colour codes.
pub fn plain(row: &str) -> std::borrow::Cow<'_, str> {
    SGR.replace_all(row, "")
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

    #[test]
    fn a_secret_split_by_colour_codes_is_hidden_and_the_codes_after_it_stay() {
        let token = "abcdefghij".to_owned();
        assert_eq!(
            redact("T=\x1b[1mabc\x1b[31mdefghij\x1b[0m ok", &[token]),
            Some(("T=\x1b[1m[redacted]\x1b[31m\x1b[0m ok".to_owned(), 1))
        );
        assert_eq!(
            redact("\x1b[32mghp_\x1b[1mabcdefghijklmnopqrstuvwxyz\x1b[0m", &[]),
            Some(("\x1b[32m[redacted]\x1b[1m\x1b[0m".to_owned(), 1))
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

        #[test]
        fn colour_codes_anywhere_never_change_what_is_hidden(
            secret in "[a-z]{8,12}",
            pieces in prop::collection::vec(".{0,6}|ghp_[a-z]{20}|\\[redacted\\]|\n", 0..10),
            codes in prop::collection::vec((0usize..200, "\x1b\\[[0-9;]{0,6}m"), 0..8),
        ) {
            let mut plain: String = pieces.concat();
            let middle = plain.char_indices().map(|(at, _)| at).find(|at| *at >= plain.len() / 2).unwrap_or(plain.len());
            plain.insert_str(middle, &secret);
            let mut text = plain.clone();
            for (at, code) in &codes {
                let at = text.char_indices().map(|(at, _)| at).chain([text.len()]).nth(at % (text.chars().count() + 1)).unwrap_or(0);
                // Never inside another code, which would make it something else.
                if super::SGR.find_iter(&text).any(|found| found.start() < at && at < found.end()) {
                    continue;
                }
                text.insert_str(at, code);
            }
            let plain = super::SGR.replace_all(&text, "").into_owned();
            let Some((bare, count)) = redact(&plain, std::slice::from_ref(&secret)) else { return Ok(()) };
            let (shown, coloured) = redact(&text, std::slice::from_ref(&secret)).expect("the plain text redacts");
            prop_assert_eq!(super::SGR.replace_all(&shown, "").into_owned(), bare);
            prop_assert_eq!(coloured, count);
            prop_assert_eq!(super::SGR.find_iter(&shown).count(), super::SGR.find_iter(&text).count());
        }
    }
}
