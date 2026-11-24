//! Keeps the factory's private terms out of what a call publishes to a repository it lists. A
//! refusal names each rule's label and location, never the term.

use crate::screen;
use priory_factory::private::Private;
use priory_factory::publish::{Commits, Publish};
use priory_runtime::runtime::{self, git};
use regex::Regex;
use std::path::{Path, PathBuf};

/// The most locations a refusal names.
const MOST: usize = 20;
/// The most of a published file that is read.
const FILE: u64 = 4 * 1024 * 1024;

/// The hook's line, which asks for the factory's terms: `{"private": {}}`.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Line {
    #[serde(rename = "private")]
    _private: Nothing,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Nothing {}

const NO_DIR: &str = "Priory cannot tell which folder it runs in: use a literal path";
const GONE: &str = "a file it publishes cannot be read: write it in an earlier call, and name it with a literal path";
/// What a refusal calls the people a commit names.
const PEOPLE: [&str; 2] = ["author", "committer"];

type Terms = [(String, Regex)];

/// Refuses `publish`, a call made in `cwd`, when it would put a private term on a listed
/// repository, or when Priory cannot read what it would put there.
pub fn screen(rules: &Private, cwd: &Path, publish: &Publish) -> Result<(), String> {
    if rules.repos.is_empty() || rules.terms.is_empty() {
        return Ok(());
    }
    let unread = |why: &str| {
        format!(
            "Priory denied `{}`: it may publish to a repository the factory keeps private terms out of, and {why}.",
            publish.call
        )
    };
    if publish.lost && publish.repo.is_none() {
        return Err(unread(NO_DIR));
    }
    let places: Vec<(PathBuf, Vec<Remote>)> = publish
        .folders
        .iter()
        .map(|dirs| {
            let dir = folder(cwd, dirs);
            let remotes = remotes(&dir);
            (dir, remotes)
        })
        .filter(|(dir, remotes)| listed(rules, publish, dir, remotes))
        .collect();
    if places.is_empty() {
        return Ok(());
    }
    if let Some(why) = publish.unread {
        return Err(unread(why));
    }
    let terms = rules.compile()?;
    let mut hits = Vec::new();
    for (place, text) in &publish.texts {
        note(&mut hits, &terms, place, text);
    }
    for file in &publish.files {
        let mut unreadable = Some(GONE.to_owned());
        for dirs in &file.folders {
            match contents(&folder(cwd, dirs).join(&file.path)) {
                Ok(text) => {
                    unreadable = None;
                    note(&mut hits, &terms, &shown(&terms, &file.path), &text);
                }
                Err(why) => unreadable = unreadable.map(|_| why),
            }
        }
        if let Some(why) = unreadable.filter(|_| file.needed) {
            return Err(unread(&why));
        }
    }
    for (dir, remotes) in &places {
        let range = revisions(rules, publish, dir, remotes).map_err(|why| unread(&why))?;
        if let Some(range) = range {
            commits(dir, &range, &terms, &mut hits)
                .map_err(|_| unread("git could not list the commits it publishes"))?;
        }
    }
    refused(&format!("`{}`", publish.call), hits)
}

/// Refuses the pull requests of a stack whose top is `commit`, as `screen` does, before any push.
pub(crate) fn screen_stack(
    rules: &Private,
    worktree: &Path,
    repo: &str,
    commit: &str,
    title: &str,
    body: &str,
) -> Result<(), String> {
    if !rules.lists(repo) {
        return Ok(());
    }
    let terms = rules.compile()?;
    let mut hits = Vec::new();
    note(&mut hits, &terms, "PR title", title);
    note(&mut hits, &terms, "PR body", body);
    let range = [commit, "--not", "refs/remotes/origin/HEAD"].map(str::to_owned);
    commits(worktree, &range, &terms, &mut hits)
        .map_err(|_| "git could not list the commits the pull request publishes".to_owned())?;
    refused("the pull request", hits)
}

fn refused(what: &str, hits: Vec<String>) -> Result<(), String> {
    if hits.is_empty() {
        return Ok(());
    }
    let more = match hits.len().saturating_sub(MOST) {
        0 => String::new(),
        more => format!("; and {more} more"),
    };
    let person = hits
        .iter()
        .any(|hit| PEOPLE.iter().any(|person| hit.ends_with(person)));
    let reset = match person {
        true => {
            " To change who made a commit, set user.name and user.email, then run `git commit --amend --reset-author`, in a rebase for an older commit."
        }
        false => "",
    };
    Err(format!(
        "Priory denied {what}: it would publish private terms to a repository the factory keeps them out of: {}{more}.{reset} Rewrite that text without them, amend or rebase the commits that hold them, then try again. Do not publish it another way.",
        hits[..hits.len().min(MOST)].join("; ")
    ))
}

fn note(hits: &mut Vec<String>, terms: &Terms, place: &str, text: &str) {
    for (line, label) in screen::labels(text, terms) {
        hits.push(format!("rule {label}: {place} line {line}"));
    }
}

/// `path`, unless a term is in it.
fn shown(terms: &Terms, path: &str) -> String {
    match screen::labels(path, terms).is_empty() {
        true => path.to_owned(),
        false => "a file whose path holds a term".to_owned(),
    }
}

fn folder(cwd: &Path, dirs: &[String]) -> PathBuf {
    dirs.iter().fold(cwd.to_owned(), |dir, next| dir.join(next))
}

fn contents(path: &Path) -> Result<String, String> {
    let found = std::fs::metadata(path).map_err(|_| GONE.to_owned())?;
    if !found.is_file() {
        return Err("it publishes something that is not a regular file".to_owned());
    }
    if found.len() > FILE {
        return Err("a file it publishes is over 4 MiB".to_owned());
    }
    let bytes = std::fs::read(path).map_err(|_| GONE.to_owned())?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn read(dir: &Path, args: &[&str]) -> Option<String> {
    runtime::run(git(dir).args(args))
        .ok()
        .map(|out| out.trim().to_owned())
}

/// The GitHub `owner/name` at the end of a remote's URL, or the text itself when it is one.
fn slug(url: &str) -> Option<String> {
    let url = url.trim_end_matches('/');
    let url = url.strip_suffix(".git").unwrap_or(url);
    let mut parts = url.rsplit(['/', ':']);
    let (name, owner) = (parts.next()?, parts.next()?);
    let slug = format!("{owner}/{name}");
    runtime::check_repo(&slug).ok().map(|()| slug)
}

struct Remote {
    name: String,
    urls: Vec<String>,
}

impl Remote {
    fn listed(&self, rules: &Private) -> bool {
        self.urls.iter().any(|url| url_listed(rules, url))
    }

    /// Whether every URL names a listed repository, so what it holds is public already.
    fn surely_listed(&self, rules: &Private) -> bool {
        !self.urls.is_empty()
            && self
                .urls
                .iter()
                .all(|url| slug(url).is_some_and(|slug| rules.lists(&slug)))
    }
}

/// A URL Priory cannot read counts as a listed one.
fn url_listed(rules: &Private, url: &str) -> bool {
    slug(url).is_none_or(|slug| rules.lists(&slug))
}

/// The remotes of `dir`, each with its URLs as written and as git resolves them for a push.
fn remotes(dir: &Path) -> Vec<Remote> {
    let names = read(dir, &["remote"]).unwrap_or_default();
    names
        .lines()
        .map(|name| {
            let urls = [
                read(dir, &["config", "--get-all", &format!("remote.{name}.url")]),
                read(dir, &["remote", "get-url", "--push", "--all", name]),
            ];
            Remote {
                name: name.to_owned(),
                urls: urls
                    .into_iter()
                    .flatten()
                    .flat_map(|out| out.lines().map(str::to_owned).collect::<Vec<_>>())
                    .collect(),
            }
        })
        .collect()
}

/// Whether `publish` writes to a listed repository, or to one Priory cannot name.
fn listed(rules: &Private, publish: &Publish, dir: &Path, remotes: &[Remote]) -> bool {
    if publish.anywhere {
        return true;
    }
    if let Some(repo) = &publish.repo {
        let path = repo
            .split_once("/pull/")
            .map_or(repo.as_str(), |(path, _)| path);
        return url_listed(rules, path);
    }
    if let Commits::Push { remote, .. } = &publish.commits {
        let name = remote.clone().unwrap_or_else(|| default_remote(dir));
        return match remotes.iter().find(|remote| remote.name == name) {
            Some(remote) => remote.listed(rules),
            None => url_listed(rules, &name),
        };
    }
    remotes.is_empty() || remotes.iter().any(|remote| remote.listed(rules))
}

/// The remote a push that names none goes to.
fn default_remote(dir: &Path) -> String {
    let branch = read(dir, &["symbolic-ref", "--short", "HEAD"]).unwrap_or_default();
    [
        format!("branch.{branch}.pushRemote"),
        "remote.pushDefault".to_owned(),
        format!("branch.{branch}.remote"),
    ]
    .iter()
    .find_map(|key| read(dir, &["config", "--get", key]))
    .unwrap_or_else(|| "origin".to_owned())
}

/// The `git log` arguments that list the commits `publish` would make public, if any.
fn revisions(
    rules: &Private,
    publish: &Publish,
    dir: &Path,
    remotes: &[Remote],
) -> Result<Option<Vec<String>>, String> {
    let known = |revision: &str| {
        let commit = format!("{revision}^{{commit}}");
        read(
            dir,
            &[
                "rev-parse",
                "--verify",
                "--quiet",
                "--end-of-options",
                &commit,
            ],
        )
    };
    match &publish.commits {
        Commits::None => Ok(None),
        Commits::Push {
            remote,
            revisions,
            every,
        } => {
            let name = remote.clone().unwrap_or_else(|| default_remote(dir));
            let remote = remotes
                .iter()
                .find(|remote| remote.name == name || remote.urls.contains(&name))
                .ok_or("its remote is not one `git remote` lists: push to a named remote")?;
            let config = |key: &str| read(dir, &["config", "--get-all", key]).unwrap_or_default();
            // Config can send other branches than the current one.
            let widened = revisions.is_empty()
                && (config("push.default") == "matching"
                    || !config(&format!("remote.{name}.push")).is_empty()
                    || config(&format!("remote.{name}.mirror")) == "true");
            let mut args = Vec::new();
            if *every || widened {
                args.extend(["--branches", "--tags"].map(str::to_owned));
            }
            for revision in revisions {
                args.push(known(revision).ok_or("a revision it pushes does not resolve")?);
            }
            if args.is_empty() {
                args.push(known("HEAD").ok_or("the branch it pushes has no commit")?);
            }
            args.push("--not".to_owned());
            args.extend(
                remotes
                    .iter()
                    .filter(|other| other.name == remote.name || other.surely_listed(rules))
                    .map(|other| format!("--remotes={}", other.name)),
            );
            Ok(Some(args))
        }
        Commits::Head(head) => {
            let base = remotes
                .iter()
                .find(|remote| remote.listed(rules))
                .or(remotes.first())
                .ok_or("Priory cannot read its commits: run it in the repository's clone")?;
            let branch = head.as_deref().unwrap_or("HEAD");
            let branch = branch.rsplit(':').next().unwrap_or(branch);
            let commit = known(branch)
                .or_else(|| known(&format!("refs/remotes/{}/{branch}", base.name)))
                .ok_or("its head branch does not resolve in this folder")?;
            let trunk = format!("refs/remotes/{}/HEAD", base.name);
            let without = match known(&trunk) {
                Some(_) => trunk,
                None => format!("--remotes={}", base.name),
            };
            Ok(Some(vec![commit, "--not".to_owned(), without]))
        }
    }
}

/// Notes each term in the messages, paths and added lines of the commits in `range`. Each commit
/// counts, since one a later commit undoes is public too. A merge shows only what it resolved.
fn commits(
    dir: &Path,
    range: &[String],
    terms: &Terms,
    hits: &mut Vec<String>,
) -> Result<(), String> {
    // The daemon runs this outside a run's jail, so no merge driver of the repository may run.
    let drivers = read(
        dir,
        &[
            "config",
            "-z",
            "--name-only",
            "--get-regexp",
            r"^merge\..*\.driver$",
        ],
    )
    .unwrap_or_default();
    let drivers: Vec<&str> = drivers.split_terminator('\0').collect();
    let log = |format: &str, options: &[&str]| {
        let mut command = git(dir);
        for (at, driver) in drivers.iter().enumerate() {
            command
                .env(format!("GIT_CONFIG_KEY_{at}"), driver)
                .env(format!("GIT_CONFIG_VALUE_{at}"), "false");
        }
        command.env("GIT_CONFIG_COUNT", drivers.len().to_string());
        runtime::run(
            command
                .args(["-c", "core.quotePath=false", "-c", "log.showRoot=true"])
                .args([
                    "-c",
                    "log.showSignature=false",
                    "-c",
                    "merge.renormalize=false",
                ])
                .args(["log", "--encoding=UTF-8", "--no-color", "--no-notes"])
                .args(["--no-relative", format])
                .args(options)
                .args(range)
                .arg("--"),
        )
    };
    let format = "--format=%x00%h%n%an <%ae>%n%cn <%ce>%n%B";
    let messages = log(format, &["--no-patch"])?;
    for message in messages.split('\0').skip(1) {
        let mut parts = message.splitn(4, '\n');
        let mut part = || parts.next().unwrap_or_default();
        let commit = part();
        for person in PEOPLE {
            for (_, label) in screen::labels(part(), terms) {
                hits.push(format!("rule {label}: commit {commit} {person}"));
            }
        }
        let text = part();
        note(hits, terms, &format!("commit {commit} message"), text);
    }
    let patch = log(
        "--format=%x00%h",
        &[
            "--patch",
            "--unified=0",
            "--inter-hunk-context=0",
            "--text",
            "--no-ext-diff",
            "--no-textconv",
            "--no-renames",
            "--submodule=short",
            "--diff-merges=remerge",
            "--src-prefix=a/",
            "--dst-prefix=b/",
        ],
    )?;
    added(&patch, terms, hits);
    Ok(())
}

/// Notes each term in the paths and added lines of `patch`, as `commits` has git print it.
fn added(patch: &str, terms: &Terms, hits: &mut Vec<String>) {
    let (mut commit, mut path) = ("", String::new());
    let (mut old, mut new, mut at) = (0, 0, 0);
    for line in patch.split('\n') {
        if old > 0 || new > 0 {
            if let Some(text) = line.strip_prefix('+') {
                for (_, label) in screen::labels(text, terms) {
                    hits.push(format!("rule {label}: commit {commit} {path}:{at}"));
                }
                (new, at) = (new - new.min(1), at + 1);
            } else if line.starts_with('-') {
                old -= old.min(1);
            }
            continue;
        }
        if let Some(next) = line.strip_prefix('\0') {
            commit = next;
        } else if let Some(hunk) = line.strip_prefix("@@ ") {
            let mut ranges = hunk.split(' ').map(|range| {
                let range = range.get(1..).unwrap_or_default();
                let (start, count) = range.split_once(',').unwrap_or((range, "1"));
                (start.parse().unwrap_or(0), count.parse().unwrap_or(0))
            });
            (_, old) = ranges.next().unwrap_or((0, 0));
            (at, new) = ranges.next().unwrap_or((0usize, 0usize));
        } else if let Some(name) = line
            .strip_prefix("+++ ")
            .filter(|name| *name != "/dev/null")
        {
            let name = name.trim_end_matches('\t').trim_matches('"');
            let name = name.strip_prefix("b/").unwrap_or(name);
            path = shown(terms, name);
            for (_, label) in screen::labels(name, terms) {
                hits.push(format!("rule {label}: commit {commit} a file path"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn only_added_lines_and_new_paths_count() {
        let terms = [("host".to_owned(), Regex::new("vault9").unwrap())];
        let patch = "\0abc\n\ndiff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -4 +4,2 @@ vault9\n-vault9\n+++ vault9\n+ok\ndiff --git a/vault9 b/vault9\n--- /dev/null\n+++ b/vault9\n@@ -0,0 +1 @@\n+vault9\n\\ No newline at end of file\n";
        let mut hits = Vec::new();
        added(patch, &terms, &mut hits);
        assert_eq!(
            hits,
            [
                "rule host: commit abc x:4",
                "rule host: commit abc a file path",
                "rule host: commit abc a file whose path holds a term:1"
            ]
        );
    }

    #[test]
    fn a_url_names_its_repository_and_an_unreadable_one_counts_as_listed() {
        for url in [
            "https://github.com/O/r.git",
            "git@github.com:o/r.git",
            "ssh://git@github.com/o/r/",
            "o/r",
        ] {
            assert!(slug(url).unwrap().eq_ignore_ascii_case("o/r"), "{url}");
        }
        assert_eq!(slug("r"), None);
    }

    proptest! {
        #[test]
        fn reading_a_patch_never_panics(
            lines in prop::collection::vec("@@ -?[0-9,é]{0,4} \\+?[0-9,]{0,4} @@|\\+\\+\\+ \\PC{0,6}|[-+\0]\\PC{0,6}|\\PC{0,6}", 0..20),
        ) {
            let terms = [("host".to_owned(), Regex::new("vault9").unwrap())];
            added(&lines.join("\n"), &terms, &mut Vec::new());
        }
    }
}
