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
    let mut dirs: Vec<PathBuf> = publish
        .folders
        .iter()
        .map(|dirs| folder(cwd, dirs))
        .collect();
    let is_repo = |dir: &PathBuf| read(dir, &["rev-parse", "--git-dir"]).is_some();
    if dirs.len() > 1 && !publish.creates && dirs.iter().any(is_repo) {
        dirs.retain(is_repo);
    }
    let places: Vec<(PathBuf, Vec<Remote>)> = dirs
        .into_iter()
        .map(|dir| {
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
    base: Option<&str>,
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
    let mut range = [commit, "--not", "refs/remotes/origin/HEAD"]
        .map(str::to_owned)
        .to_vec();
    range.extend(base.and_then(|base| tracked(worktree, "origin", base)));
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

/// The commit of `branch` that `remote` is known to hold.
fn tracked(dir: &Path, remote: &str, branch: &str) -> Option<String> {
    let name = format!("refs/remotes/{remote}/{branch}");
    read(dir, &["show-ref", "--verify", "--hash", &name])
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
        Commits::Head { branch, base } => {
            let remote = remotes
                .iter()
                .find(|remote| remote.listed(rules))
                .or(remotes.first())
                .ok_or("Priory cannot read its commits: run it in the repository's clone")?;
            let branch = branch.as_deref().unwrap_or("HEAD");
            let branch = branch.rsplit(':').next().unwrap_or(branch);
            let commit = known(branch)
                .or_else(|| known(&format!("refs/remotes/{}/{branch}", remote.name)))
                .ok_or("its head branch does not resolve in this folder")?;
            let trunk = format!("refs/remotes/{}/HEAD", remote.name);
            let without = match known(&trunk) {
                Some(_) => trunk,
                None => format!("--remotes={}", remote.name),
            };
            let mut args = vec![commit, "--not".to_owned(), without];
            if remote.surely_listed(rules) {
                let base = base.as_deref();
                args.extend(base.and_then(|base| tracked(dir, &remote.name, base)));
            }
            Ok(Some(args))
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
    let (mut commit, mut path, mut header) = ("", String::new(), "");
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
        } else if let Some(names) = line.strip_prefix("diff --git ") {
            // Git prints one path twice, as `a/x b/x`.
            header = names.get(names.len() / 2 + 1..).unwrap_or_default();
        } else if line.starts_with("new file mode ") {
            added_path(header, commit, terms, hits);
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
            path = shown(terms, added_path(name, commit, terms, hits));
        }
    }
}

/// Notes each term in the path `name`, once a commit, and returns it without git's marks.
fn added_path<'n>(name: &'n str, commit: &str, terms: &Terms, hits: &mut Vec<String>) -> &'n str {
    let name = name.trim_end_matches('\t').trim_matches('"');
    let name = name.strip_prefix("b/").unwrap_or(name);
    for (_, label) in screen::labels(name, terms) {
        let hit = format!("rule {label}: commit {commit} a file path");
        if !hits.contains(&hit) {
            hits.push(hit);
        }
    }
    name
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

    #[test]
    fn an_issue_a_merge_and_a_link_in_a_field_hide_no_term() {
        let rules = rules();
        let denied = |command: &str| {
            let none = serde_json::Value::Null;
            let found = priory_factory::publish::calls("Bash", Some(command), &none);
            assert_eq!(found.len(), 1, "{command}");
            screen(&rules, &std::env::temp_dir(), &found[0]).err()
        };
        for command in [
            "gh api -X PATCH repos/o/r/issues/3 -f body=vault9",
            "gh api -X PATCH repos/o/r/pulls/3 -f body='https://github.com/repos/o/x/pulls/1 vault9'",
            "gh pr merge 3 -R o/r --squash --subject vault9",
            "gh pr merge 3 -R o/r -sb vault9",
        ] {
            let reason = denied(command).expect(command);
            assert!(reason.contains("rule host"), "{reason}");
            assert!(!reason.contains("vault9"), "{reason}");
            assert_eq!(denied(&command.replace("o/r", "o/x")), None);
        }
        let unread = "gh api -X PATCH repos/o/r/issues/3 -f body=\"$(notes)\"";
        assert!(denied(unread).is_some());
        assert_eq!(denied(&unread.replace("o/r", "o/x")), None);
    }

    /// Two clones under one folder, `o/r` and `o/x`, each on `child`, whose base `parent` the
    /// remote holds with a term.
    struct Clones(PathBuf);

    impl Clones {
        fn new(name: &str) -> Self {
            let id = format!("priory-private-{name}-{}", std::process::id());
            let clones = Self(std::env::temp_dir().join(id));
            for repo in ["r", "x"] {
                std::fs::create_dir_all(clones.0.join(repo)).unwrap();
                let url = format!("https://github.com/o/{repo}");
                clones.git(repo, "init -q -b main");
                clones.git(repo, &format!("remote add origin {url}"));
                clones.git(repo, "commit -q --allow-empty -m a");
                clones.git(repo, "branch other");
                clones.git(repo, "update-ref refs/remotes/origin/main HEAD");
                let trunk = "refs/remotes/origin/HEAD refs/remotes/origin/main";
                clones.git(repo, &format!("symbolic-ref {trunk}"));
                clones.git(repo, "checkout -q -b parent");
                clones.add(repo, "p", "vault9");
                clones.git(repo, "update-ref refs/remotes/origin/parent HEAD");
                clones.git(repo, "checkout -q -b child");
                clones.add(repo, "b.md", "fine");
            }
            std::fs::create_dir(clones.0.join("plain")).unwrap();
            clones
        }

        fn git(&self, repo: &str, args: &str) {
            let mut command = git(&self.0.join(repo));
            command.args(["-c", "user.name=t", "-c", "user.email=t@t"]);
            runtime::run(command.args(args.split(' '))).unwrap();
        }

        fn add(&self, repo: &str, file: &str, text: &str) {
            std::fs::write(self.0.join(repo).join(file), text).unwrap();
            self.git(repo, &format!("add {file}"));
            self.git(repo, "commit -q -m x");
        }

        /// Why `command`, where `{}` is the folder of both clones, is denied in `repo`.
        fn denial(&self, repo: &str, command: &str) -> Option<String> {
            let command = command.replace("{}", &self.0.display().to_string());
            let none = serde_json::Value::Null;
            let found = priory_factory::publish::calls("Bash", Some(&command), &none);
            found
                .iter()
                .find_map(|call| screen(&rules(), &self.0.join(repo), call).err())
        }
    }

    impl Drop for Clones {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn rules() -> Private {
        Private {
            repos: vec!["o/r".to_owned()],
            terms: vec![priory_factory::private::Term {
                label: "host".to_owned(),
                text: vec!["vault9".to_owned()],
                regex: Vec::new(),
            }],
        }
    }

    #[test]
    fn a_call_is_denied_where_it_goes_to_a_listed_repository_and_only_there() {
        let clones = Clones::new("calls");
        // Each call, then whether a listed and an unlisted clone deny it.
        for (command, listed, unlisted) in [
            (
                "REPO=o/r; gh api -X PATCH repos/$REPO/pulls/3 -f body=vault9",
                true,
                true,
            ),
            (
                "REPO=o/x; gh api -X PATCH repos/$REPO/pulls/3 -f body=vault9",
                false,
                false,
            ),
            (
                "R=o/x; R=o/r; gh api repos/$R/pulls/3 -f body=vault9",
                true,
                true,
            ),
            ("gh api repos/$R/pulls/3 -f body=vault9; R=o/x", true, true),
            (
                "E=repos/o/r/pulls/3; gh api -X PATCH $E -f body=vault9",
                true,
                true,
            ),
            (
                "gh api -X PATCH repos/$(cat f)/pulls/3 -f body=vault9",
                true,
                true,
            ),
            (
                "gh api graphql -f query='query($o:ID!){n}' -F o=\"$O\"",
                false,
                false,
            ),
            (
                "gh api graphql -f query='{s(query:\"vault9\"){n}}'",
                false,
                false,
            ),
            (
                "gh api graphql -f query='mutation{x(b:\"vault9\")}'",
                true,
                true,
            ),
            (
                "Q='mutation{x}'; gh api graphql -f query=\"$Q\" -f b=vault9",
                true,
                true,
            ),
            (
                "gh api graphql -f query='{n}' -F \"$X\" -f b=vault9",
                true,
                true,
            ),
            (
                "gh api graphql -f query=\"$(cat q)\" -f b=vault9",
                true,
                true,
            ),
            ("gh api graphql -F query=@q -f b=vault9", true, true),
            ("git -c user.name=x push origin nosuch", true, false),
            (
                "git -c push.autoSetupRemote=true push origin nosuch",
                true,
                false,
            ),
            ("git -c url.a.insteadOf=b push origin nosuch", true, true),
            (
                "git -c remote.origin.pushurl=b push origin nosuch",
                true,
                true,
            ),
            ("git -c \"$K\" push origin nosuch", true, true),
            ("(cd {}/plain && ls); git push origin HEAD", false, false),
            (
                "(cd {}/plain && ls); gh pr edit 3 -b \"$(notes)\"",
                true,
                false,
            ),
            (
                "git clone -q u d && cd d && git commit -am x && git push",
                true,
                true,
            ),
            ("D={}/x; git -C $D push origin nosuch", false, false),
            ("D={}/r; git -C $D push origin nosuch", true, true),
            ("git -C $D push origin nosuch", true, true),
            ("env -C {}/r git push origin nosuch", true, true),
            ("gh pr create -B parent -t x -b y", false, false),
            ("gh pr create --base=other -t x -b y", true, false),
            ("gh pr create -t x -b y", true, false),
            (
                "git commit -qm x && gh pr create -B parent --fill",
                true,
                false,
            ),
            (
                "gh pr create -B parent --fill && git commit -qm x",
                false,
                false,
            ),
            (
                "gh pr edit 3 --add-project https://github.com/o/x -b vault9",
                true,
                false,
            ),
            (
                "gh pr edit --add-project=https://github.com/o/x/pull/1 -b vault9",
                true,
                false,
            ),
            (
                "gh pr edit https://github.com/o/r/pull/3 -b vault9",
                true,
                true,
            ),
            (
                "gh pr edit 3 -b 'the GH_REPO variable, vault9'",
                true,
                false,
            ),
            ("GH_REPO=o/r gh pr edit 3 -b vault9", true, true),
            ("export GH_REPO=o/r; gh pr edit 3 -b vault9", true, true),
            ("env GH_REPO=o/r gh pr edit 3 -b vault9", true, true),
            ("git push origin HEAD && git commit -m x", false, false),
            ("git commit -m x && git push origin HEAD", true, false),
            (
                "for i in 1 2; do git push; git commit -m x; done",
                true,
                false,
            ),
            ("git stash && git push origin HEAD", false, false),
            ("git checkout -b new && git push origin HEAD", false, false),
            ("git checkout -- b.md && git push origin HEAD", false, false),
            ("git checkout other && git push origin child", false, false),
            ("git checkout other && git push origin HEAD", true, false),
            (
                "git switch -c new other && git push origin HEAD",
                true,
                false,
            ),
            (
                "git checkout -B child other && git push origin child",
                true,
                false,
            ),
            ("git add b.md && gh pr edit 3 -F b.md", false, false),
            ("sed -i s/a/b/ b.md && gh pr edit 3 -F b.md", true, false),
            ("echo \"$(date)\"; gh pr edit 3 --body \"\"", false, false),
            ("gh pr edit 3 --body \"$(notes)\"", true, false),
        ] {
            let denied = |repo| clones.denial(repo, command);
            assert_eq!(
                denied("r").is_some(),
                listed,
                "{command}: {:?}",
                denied("r")
            );
            assert_eq!(
                denied("x").is_some(),
                unlisted,
                "{command}: {:?}",
                denied("x")
            );
        }
    }

    #[test]
    fn a_private_base_and_push_settings_hide_no_commit_and_a_public_base_is_left_out() {
        let clones = Clones::new("base");
        let create = "gh pr create -R o/r -B parent -t x -b y";
        assert_eq!(clones.denial("r", create), None);
        assert!(clones.denial("x", create).is_some());
        for repo in ["r", "x"] {
            clones.git(repo, "checkout -q -b hidden other");
            clones.add(repo, "h", "vault9");
            clones.git(repo, "checkout -q child");
            let matching = "git -c push.default=matching push";
            assert_eq!(clones.denial(repo, matching).is_some(), repo == "r");
            assert_eq!(clones.denial(repo, "git push"), None);
        }
        let stack = |base| {
            let dir = clones.0.join("r");
            screen_stack(&rules(), &dir, "o/r", "child", base, "x", "y").err()
        };
        assert_eq!(stack(Some("parent")), None);
        assert!(stack(None).is_some());
        assert!(stack(Some("gone")).is_some());
    }

    #[test]
    fn an_empty_new_files_path_is_screened_and_a_deleted_ones_is_not() {
        for (repo, denied) in [("r", true), ("x", false)] {
            let clones = Clones::new(&format!("empty-{repo}"));
            let push = "git push origin HEAD";
            clones.add(repo, "a-vault9", "");
            assert_eq!(clones.denial(repo, push).is_some(), denied);
            clones.git(repo, "update-ref refs/remotes/origin/child HEAD");
            clones.git(repo, "rm -q -r .");
            clones.git(repo, "commit -q -m x");
            assert_eq!(clones.denial(repo, push), None);
        }
    }

    #[test]
    fn an_mcp_merge_and_a_deletion_are_screened_by_their_message() {
        let denied = |tool: &str, repo: &str, message: &str| {
            let input = serde_json::json!({ "owner": "o", "repo": repo, "path": "vault9", "message": message });
            let found = priory_factory::publish::calls(tool, None, &input);
            screen(&rules(), &std::env::temp_dir(), &found[0]).is_err()
        };
        let (merge, delete) = ("mcp__g__merge_pull_request", "mcp__g__delete_file");
        for tool in [merge, delete] {
            assert!(denied(tool, "r", "vault9"));
            assert!(!denied(tool, "x", "vault9"));
        }
        assert!(!denied(delete, "r", "fine"));
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
