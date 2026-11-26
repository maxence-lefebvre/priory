use priory_runtime::runtime::{command_timeout, output_within};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use std::process::Command;
use std::time::Duration;

const PAGE: usize = 100;

static VIEWER: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// The login `gh` runs as, which a worker's `gh` posts as too. Kept once `gh` answers; a failed
/// ask is asked again by the next caller.
pub(super) fn viewer() -> Option<&'static str> {
    if let Some(login) = VIEWER.get() {
        return Some(login);
    }
    let login = gh::<Login>("user", &[]).ok()?.login;
    Some(VIEWER.get_or_init(|| login))
}

#[derive(Deserialize)]
pub(super) struct Login {
    pub(super) login: String,
}

/// Newest-first pages until one reaches `cursor`: the items after `cursor`, oldest first, and the
/// highest position seen. Without a cursor, only the first page: older history is not published.
pub(super) fn pages<T: DeserializeOwned>(
    path: &str,
    cursor: Option<u64>,
    order: impl Fn(&T) -> u64,
) -> Result<(Vec<T>, Option<u64>), String> {
    let mut items: Vec<T> = Vec::new();
    for page in 1.. {
        let batch: Vec<T> = gh(&format!("{path}per_page={PAGE}&page={page}"), &[])?;
        let last = batch.len() < PAGE
            || cursor.is_none_or(|cursor| batch.iter().any(|item| order(item) <= cursor));
        items.extend(batch);
        if last {
            break;
        }
    }
    let top = items.iter().map(&order).max();
    items.retain(|item| cursor.is_none_or(|cursor| order(item) > cursor));
    items.sort_by_key(&order);
    Ok((items, top))
}

/// The waits before each retry of a call GitHub failed with a 5xx or its GraphQL timeout: a busy
/// repository's query can time out once and answer the next time.
pub(super) const BACKOFF: [Duration; 2] = [Duration::from_secs(1), Duration::from_secs(3)];

pub(super) fn gh<T: DeserializeOwned>(path: &str, fields: &[&str]) -> Result<T, String> {
    gh_within(path, fields, &BACKOFF).map_err(Failed::text)
}

/// What `gh` answers, tried again after each of `waits` while it fails with a transient error.
pub(super) fn gh_within<T: DeserializeOwned>(
    path: &str,
    fields: &[&str],
    waits: &[Duration],
) -> Result<T, Failed> {
    let mut waits = waits.iter();
    loop {
        match call(path, fields) {
            Err(Failed::Transient(error)) => match waits.next() {
                Some(wait) => std::thread::sleep(*wait),
                None => return Err(Failed::Transient(error)),
            },
            answer => return answer,
        }
    }
}

pub(super) enum Failed {
    /// A 5xx, GitHub's GraphQL timeout or a truncated answer: worth another try.
    Transient(String),
    /// Any other failure, a call past `command_timeout` among them: retrying it could hold the
    /// poll for minutes.
    Other(String),
}

impl Failed {
    pub(super) fn text(self) -> String {
        match self {
            Failed::Transient(error) | Failed::Other(error) => error,
        }
    }
}

fn call<T: DeserializeOwned>(path: &str, fields: &[&str]) -> Result<T, Failed> {
    let output = output_within(Command::new("gh").args(["api", path]).args(fields))
        .map_err(|error| Failed::Other(format!("gh api {path}: {error}")))?
        .ok_or_else(|| {
            let timeout = command_timeout().as_secs();
            Failed::Other(format!("gh api {path} timed out after {timeout}s"))
        })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let error = format!("gh api {path} failed: {}", stderr.trim());
        return Err(if transient(&stderr) {
            Failed::Transient(error)
        } else {
            Failed::Other(error)
        });
    }
    serde_json::from_slice(&output.stdout).map_err(|error| {
        let text = format!("gh api {path}: {error}");
        match error.is_eof() {
            true => Failed::Transient(text),
            false => Failed::Other(text),
        }
    })
}

/// Whether `gh`'s `stderr` says GitHub failed on its side: an HTTP 5xx, the GraphQL error it
/// returns when a query runs out of time, or an answer cut short.
fn transient(stderr: &str) -> bool {
    let server = stderr.match_indices("HTTP 5").any(|(at, _)| {
        let code = &stderr.as_bytes()[at + 6..];
        code.len() >= 2
            && code[..2].iter().all(u8::is_ascii_digit)
            && code.get(2).is_none_or(|next| !next.is_ascii_digit())
    });
    server
        || [
            "Something went wrong while executing your query",
            "unexpected end of JSON input",
            "unexpected EOF",
        ]
        .iter()
        .any(|text| stderr.contains(text))
}

#[cfg(test)]
mod tests {
    use super::transient;
    use proptest::prelude::*;

    #[test]
    fn a_5xx_a_graphql_timeout_and_a_cut_answer_are_transient_and_a_4xx_is_not() {
        assert!(transient("unexpected end of JSON input"));
        assert!(transient(
            "Post \"https://api.github.com/graphql\": unexpected EOF"
        ));
        assert!(transient("gh: HTTP 504"));
        assert!(transient(
            "HTTP 502: Bad Gateway (https://api.github.com/graphql)"
        ));
        assert!(transient(
            "GraphQL: Something went wrong while executing your query. This may be the result of a timeout, or it could be a GitHub bug."
        ));
        assert!(!transient("HTTP 401: Bad credentials"));
        assert!(!transient("HTTP 404: Not Found"));
        assert!(!transient("HTTP 5"));
        assert!(!transient("HTTP 5000"));
    }

    proptest! {
        #[test]
        fn any_stderr_is_classified_without_panicking(stderr in ".*") {
            transient(&stderr);
        }
    }
}
