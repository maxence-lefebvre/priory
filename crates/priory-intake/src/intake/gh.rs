use serde::Deserialize;
use serde::de::DeserializeOwned;
use std::process::Command;

const PAGE: usize = 100;

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

pub(super) fn gh<T: DeserializeOwned>(path: &str, fields: &[&str]) -> Result<T, String> {
    let output = Command::new("gh")
        .args(["api", path])
        .args(fields)
        .output()
        .map_err(|error| format!("gh api {path}: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("gh api {path} failed: {}", stderr.trim()));
    }
    serde_json::from_slice(&output.stdout).map_err(|error| format!("gh api {path}: {error}"))
}
