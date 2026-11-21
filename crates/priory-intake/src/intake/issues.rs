use super::gh::{Login, pages};
use super::pulls::{
    comment, issue_comments, open_pulls, pull_requests, reviews_and_checks, thread_comment,
    thread_comments,
};
use super::{
    CHECKED, Cursors, ISSUE_CLOSED, ISSUE_COMMENTED, LABELED, OPENED, Observed, PR_CLOSED,
    PR_LABELED, PR_OPENED, Polled, REVIEWED, THREAD_COMMENTED,
};
use serde::Deserialize;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Deserialize)]
struct Label {
    name: String,
}

#[derive(Deserialize)]
struct Issue {
    id: u64,
    number: u64,
    title: String,
    html_url: String,
    user: Option<Login>,
    created_at: String,
    pull_request: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct IssueEvent {
    id: u64,
    event: String,
    created_at: String,
    actor: Option<Login>,
    label: Option<Label>,
    issue: Issue,
}

/// One poll of `repo`'s streams. Pull request events need a GraphQL query for their branches.
pub(super) fn fetch_repo(repo: &str, cursors: &Cursors) -> Result<Vec<Polled>, String> {
    let cursor = |kind| cursors.get(&(repo.to_string(), kind)).copied();
    let (events, events_top): (Vec<IssueEvent>, _) = pages(
        &format!("repos/{repo}/issues/events?"),
        cursor(LABELED),
        |event: &IssueEvent| event.id,
    )?;
    let (issues, issues_top): (Vec<Issue>, _) = pages(
        &format!("repos/{repo}/issues?state=all&sort=created&direction=desc&"),
        cursor(OPENED),
        |issue: &Issue| issue.number,
    )?;
    let numbers: BTreeSet<u64> = events
        .iter()
        .map(|event| &event.issue)
        .chain(&issues)
        .filter(|issue| issue.pull_request.is_some())
        .map(|issue| issue.number)
        .collect();
    let pulls = pull_requests(repo, &numbers)?;
    let mut last_close = BTreeMap::new();
    for event in events.iter().filter(|event| event.event == "closed") {
        last_close.insert(event.issue.number, event.id);
    }
    let source = |kind, baseline, issue: &Issue, id: u64, actor: Option<&Login>, time: &str| {
        let mut data = json!({
            "repo": repo,
            "title": issue.title,
            "url": issue.html_url,
            "actor": actor.map(|actor| &actor.login),
            "time": time,
            "github_id": id,
        });
        let number = issue.number;
        if issue.pull_request.is_none() {
            data["issue"] = json!(number);
        } else {
            let pull = pulls.get(&number);
            data["pull_request"] = json!(number);
            data["head_branch"] = json!(pull.map(|pull| &pull.head_ref_name));
            data["head_sha"] = json!(pull.map(|pull| &pull.head_ref_oid));
            data["base_branch"] = json!(pull.map(|pull| &pull.base_ref_name));
        }
        Observed {
            kind,
            name: String::new(),
            repo: repo.to_string(),
            number,
            label: None,
            baseline,
            data,
        }
    };
    let labeled = events
        .iter()
        .filter_map(|event| {
            let pull_request = event.issue.pull_request.is_some();
            let kind = match event.event.as_str() {
                "labeled" if pull_request => PR_LABELED,
                "labeled" => LABELED,
                "closed" if pull_request => PR_CLOSED,
                "closed" => ISSUE_CLOSED,
                _ => return None,
            };
            let mut observed = source(
                kind,
                cursor(LABELED).is_none(),
                &event.issue,
                event.id,
                event.actor.as_ref(),
                &event.created_at,
            );
            observed.name = format!("github:{repo}:issues/events:{}", event.id);
            if kind == PR_CLOSED {
                // The pull request's state is today's: only its last close can be the merge.
                let last = last_close.get(&event.issue.number) == Some(&event.id);
                let pull = pulls.get(&event.issue.number);
                observed.data["merged"] = json!(pull.map(|pull| pull.merged && last));
            } else if kind != ISSUE_CLOSED {
                observed.label = event.label.as_ref().map(|label| label.name.clone());
                observed.data["label"] = json!(observed.label);
            }
            Some(observed)
        })
        .collect();
    let opened = issues
        .iter()
        .map(|issue| {
            let kind = if issue.pull_request.is_none() {
                OPENED
            } else {
                PR_OPENED
            };
            let mut observed = source(
                kind,
                cursor(OPENED).is_none(),
                issue,
                issue.id,
                issue.user.as_ref(),
                &issue.created_at,
            );
            observed.name = format!("github:{repo}:issues:{}:opened", issue.id);
            observed
        })
        .collect();
    let open = open_pulls(repo)?;
    // The open list is the later snapshot of a pull request both queries read.
    let closed = pulls
        .values()
        .filter(|pull| !open.iter().any(|other| other.number == pull.number));
    let watched: BTreeMap<u64, _> = open
        .iter()
        .chain(closed)
        .map(|pull| (pull.number, pull))
        .collect();
    let (mut reviews, checks) = reviews_and_checks(repo, watched.values().copied(), cursor);
    for found in thread_comments(repo)? {
        let number = found["pull_request_url"]
            .as_str()
            .and_then(|url| url.rsplit('/').next()?.parse().ok());
        let Some(pull) = number.and_then(|number| watched.get(&number)) else {
            continue;
        };
        let Some(data) = thread_comment(&found) else {
            continue;
        };
        let mut observed = comment(THREAD_COMMENTED, repo, pull.number, &pull.branches(), data);
        observed.baseline = cursor(REVIEWED).is_none();
        reviews.push(observed);
    }
    for found in issue_comments(repo)? {
        let number = found["issue_url"]
            .as_str()
            .and_then(|url| url.rsplit('/').next()?.parse::<u64>().ok());
        let on_pull = found["html_url"]
            .as_str()
            .is_some_and(|url| url.contains("/pull/"));
        let (Some(number), false) = (number, on_pull || found["user"]["type"] == "Bot") else {
            continue;
        };
        reviews.push(Observed {
            kind: ISSUE_COMMENTED,
            name: format!("github:{repo}:issues/comments:{}", found["id"]),
            repo: repo.to_string(),
            number,
            label: None,
            baseline: cursor(REVIEWED).is_none(),
            data: json!({
                "repo": repo,
                "issue": number,
                "url": found["html_url"],
                "actor": found["user"]["login"],
                "body": found["body"],
                "time": found["created_at"],
                "github_id": found["id"],
            }),
        });
    }
    let order = |event: &Observed| {
        let time = event.data["time"].as_str().unwrap_or_default().to_owned();
        (time, event.data["github_id"].as_u64())
    };
    reviews.sort_by_key(order);
    let top = |events: &[Observed]| {
        events
            .iter()
            .filter_map(|event| event.data["github_id"].as_u64())
            .max()
    };
    // Reviews and checks first: a crash after a close would end their polling.
    Ok(vec![
        Polled {
            stream: (repo.to_string(), REVIEWED),
            top: top(&reviews),
            events: reviews,
        },
        Polled {
            stream: (repo.to_string(), CHECKED),
            top: top(&checks),
            events: checks,
        },
        Polled {
            stream: (repo.to_string(), LABELED),
            events: labeled,
            top: events_top,
        },
        Polled {
            stream: (repo.to_string(), OPENED),
            events: opened,
            top: issues_top,
        },
    ])
}
