//! Which tool calls publish to a git remote or a pull request, and what text they hold.

use crate::guard::{TEXT, gh_api, help, moves_head, positional, read, starts};
use serde_json::Value;

/// A call that publishes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Publish {
    /// The call's name with none of its arguments, such as `git push`.
    pub call: &'static str,
    /// The folders its command changes to, in order, or why they cannot be read.
    pub dirs: Vec<String>,
    /// The repository it names, if any.
    pub repo: Option<String>,
    /// Its repository cannot be read, so it counts as a listed one.
    pub anywhere: bool,
    pub commits: Commits,
    /// Text the call holds, each with the place a refusal names.
    pub texts: Vec<(&'static str, String)>,
    /// Files whose text it publishes: a missing one refuses the call when it is `needed`.
    pub files: Vec<File>,
    /// Why Priory cannot read what it would publish.
    pub unread: Option<&'static str>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct File {
    pub path: String,
    pub needed: bool,
    /// How many of the call's `dirs` come before it, since a later `cd` does not move it.
    pub dirs: usize,
}

/// The commits a call publishes.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum Commits {
    #[default]
    None,
    /// What a push sends: these revisions, or every branch and tag.
    Push {
        remote: Option<String>,
        revisions: Vec<String>,
        every: bool,
    },
    /// What a pull request from this branch, or the current one, shows.
    Head(Option<String>),
}

/// Programs that print the files they are given, as in `--body "$(cat body.md)"`.
const PRINTS: &[&str] = &["cat", "head", "tail", "sed", "awk", "jq"];

const STDIN: &str = "its text comes from stdin, which Priory cannot read: write it to a file in an earlier call and pass the file";
const MOVED: &str = "the same call also changes the branch's commits, which Priory cannot read before they exist: publish in a call of its own";
const NO_DIR: &str = "Priory cannot tell which folder it runs in: use a literal path";

/// Every call among those `tool` makes that publishes. `command` is a shell tool's command and
/// `input` any tool's arguments.
pub fn calls(tool: &str, command: Option<&str>, input: &Value) -> Vec<Publish> {
    if let Some(name) = tool.strip_prefix("mcp__") {
        return mcp(name, input).into_iter().collect();
    }
    let Some(command) = command else {
        return Vec::new();
    };
    let commands = read(command);
    let mut dirs: Vec<String> = Vec::new();
    let mut lost = false;
    let mut files = Vec::new();
    let mut found = Vec::new();
    for words in &commands {
        match words[0].as_str() {
            "cd" | "pushd" => match positional(words)[..] {
                [dir] if dir != "-" && !dir.contains(['$', '~', '*']) && !dir.is_empty() => {
                    dirs.push(dir.to_owned())
                }
                _ => lost = true,
            },
            "popd" => lost = true,
            program if PRINTS.contains(&program) => {
                files.extend(words[1..].iter().map(|path| File {
                    path: path.clone(),
                    needed: false,
                    dirs: dirs.len(),
                }));
            }
            _ => {}
        }
        if let Some(mut publish) = push(words).or_else(|| pull(words)).or_else(|| api(words)) {
            if lost {
                publish.unread = Some(NO_DIR);
            }
            let before = dirs.len();
            for file in &mut publish.files {
                file.dirs += before + publish.dirs.len();
            }
            publish.dirs.splice(0..0, dirs.iter().cloned());
            found.push(publish);
        }
    }
    // Text elsewhere in the call may be what it publishes: a heredoc, a variable or piped output.
    let around: Vec<(&'static str, String)> = commands
        .iter()
        .flat_map(|words| match words[0].as_str() {
            TEXT => vec![("inline text", words[1].clone())],
            "echo" | "printf" | "export" => vec![("inline text", words[1..].join(" "))],
            _ => Vec::new(),
        })
        .collect();
    let redirected = [
        "GH_REPO",
        "GH_HOST",
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_CONFIG",
    ]
    .iter()
    .any(|name| command.contains(name));
    for publish in &mut found {
        publish.anywhere |= redirected;
        if matches!(publish.commits, Commits::Push { .. }) {
            if moves_head(command) {
                publish.unread = Some(MOVED);
            }
        } else {
            publish.texts.extend(around.iter().cloned());
            publish.files.extend(files.iter().cloned());
        }
    }
    found
}

fn mcp(name: &str, input: &Value) -> Option<Publish> {
    let (_, short) = name.rsplit_once("__")?;
    let call = match short {
        "create_pull_request" => "create_pull_request",
        "update_pull_request" => "update_pull_request",
        "push_files" => "push_files",
        "create_or_update_file" => "create_or_update_file",
        _ => return None,
    };
    let repo = input["owner"].as_str().zip(input["repo"].as_str());
    let mut texts = Vec::new();
    let Value::Object(fields) = input else {
        return None;
    };
    for (key, value) in fields {
        if key != "owner" && key != "repo" {
            strings(value, &mut texts);
        }
    }
    Some(Publish {
        call,
        repo: repo.map(|(owner, repo)| format!("{owner}/{repo}")),
        anywhere: repo.is_none(),
        texts,
        ..Publish::default()
    })
}

fn strings(value: &Value, texts: &mut Vec<(&'static str, String)>) {
    match value {
        Value::String(text) => texts.push(("argument", text.clone())),
        Value::Array(values) => values.iter().for_each(|value| strings(value, texts)),
        Value::Object(fields) => fields.values().for_each(|value| strings(value, texts)),
        _ => {}
    }
}

/// `git push`, unless it is a dry run, asks for help or only deletes.
fn push(words: &[String]) -> Option<Publish> {
    if words[0] != "git" || positional(words).first() != Some(&"push") || help(words) {
        return None;
    }
    let mut publish = Publish {
        call: "git push",
        ..Publish::default()
    };
    let mut args = words[1..].iter();
    while let Some(arg) = args.next().filter(|arg| *arg != "push") {
        match arg.as_str() {
            "-C" => publish.dirs.extend(args.next().cloned()),
            "-c" | "--config-env" | "--namespace" => {
                args.next();
                publish.anywhere = true;
            }
            "--git-dir" | "--work-tree" => {
                args.next();
                publish.unread = Some(NO_DIR);
            }
            _ if arg.starts_with("--git-dir=") || arg.starts_with("--work-tree=") => {
                publish.unread = Some(NO_DIR)
            }
            _ if arg.starts_with("--config-env=") => publish.anywhere = true,
            _ => {}
        }
    }
    let (mut remote, mut specs) = (None, Vec::new());
    let (mut every, mut deletes) = (false, false);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--dry-run" => return None,
            "--all" | "--branches" | "--mirror" | "--tags" => every = true,
            "--delete" => deletes = true,
            "--repo" => remote = args.next().cloned(),
            "--push-option" | "--receive-pack" | "--exec" => {
                args.next();
            }
            _ if arg.starts_with("--repo=") => remote = Some(arg["--repo=".len()..].to_owned()),
            _ if arg.starts_with("--") => {}
            _ if arg.starts_with('-') && arg.len() > 1 => {
                for (at, flag) in arg[1..].char_indices() {
                    match flag {
                        'n' => return None,
                        'd' => deletes = true,
                        'o' => {
                            if at == arg.len() - 2 {
                                args.next();
                            }
                            break;
                        }
                        _ => {}
                    }
                }
            }
            _ => specs.push(arg.as_str()),
        }
    }
    if deletes {
        return None;
    }
    let mut specs = specs.into_iter();
    let remote = specs.next().map(str::to_owned).or(remote);
    let mut revisions = Vec::new();
    let mut named = false;
    for spec in specs.filter(|spec| *spec != "tag") {
        named = true;
        let source = spec.trim_start_matches('+');
        let source = source.split_once(':').map_or(source, |(source, _)| source);
        if spec.contains('*') || spec.trim_start_matches('+') == ":" {
            every = true;
        } else if !source.is_empty() {
            revisions.push(source.to_owned());
        }
    }
    if named && revisions.is_empty() && !every {
        return None;
    }
    publish.commits = Commits::Push {
        remote,
        revisions,
        every,
    };
    Some(publish)
}

/// `gh pr create` and `gh pr edit`.
fn pull(words: &[String]) -> Option<Publish> {
    let create =
        starts(words, &["gh", "pr", "create"], &[]) || starts(words, &["gh", "pr", "new"], &[]);
    if help(words) || !(create || starts(words, &["gh", "pr", "edit"], &[])) {
        return None;
    }
    let mut publish = Publish {
        call: if create { "gh pr create" } else { "gh pr edit" },
        ..Publish::default()
    };
    let mut head = None;
    let mut args = words[1..].iter();
    while let Some(arg) = args.next() {
        let (name, attached) = option(arg);
        let mut value = || attached.clone().or_else(|| args.next().cloned());
        match name.as_str() {
            "-R" | "--repo" => publish.repo = value(),
            "-F" | "--body-file" | "-T" | "--template" => file(&mut publish, value()),
            "-H" | "--head" => head = value(),
            "-t" | "--title" => publish.texts.extend(value().map(|text| ("PR title", text))),
            "-b" | "--body" => publish.texts.extend(value().map(|text| ("PR body", text))),
            _ => {
                if !create && arg.contains("github.com/") && publish.repo.is_none() {
                    publish.repo = Some(arg.clone());
                }
                publish.texts.push(("argument", arg.clone()));
            }
        }
    }
    if create {
        publish.commits = Commits::Head(head);
    }
    Some(publish)
}

/// A gh option and the value attached to it. The value options of `gh pr` end a cluster such as
/// `-dF`, after its boolean letters.
fn option(arg: &str) -> (String, Option<String>) {
    if arg.starts_with("--") {
        return match arg.split_once('=') {
            Some((name, value)) => (name.to_owned(), Some(value.to_owned())),
            None => (arg.to_owned(), None),
        };
    }
    let Some(cluster) = arg.strip_prefix('-') else {
        return (arg.to_owned(), None);
    };
    let rest = cluster.trim_start_matches(['d', 'e', 'f', 'w']);
    match rest.chars().next() {
        Some(flag) if "RFTHtb".contains(flag) => {
            let value = rest[1..].strip_prefix('=').unwrap_or(&rest[1..]);
            (
                format!("-{flag}"),
                (!value.is_empty()).then(|| value.to_owned()),
            )
        }
        _ => (arg.to_owned(), None),
    }
}

fn file(publish: &mut Publish, path: Option<String>) {
    match path {
        Some(path) if path == "-" => publish.unread = Some(STDIN),
        Some(path) => publish.files.push(File {
            path,
            needed: true,
            dirs: 0,
        }),
        None => {}
    }
}

/// `gh api` writes to a repository's pull requests, files or git data, or to GraphQL.
fn api(words: &[String]) -> Option<Publish> {
    if words[0] != "gh" || gh_api(words, &|_, _| None).is_none() {
        return None;
    }
    let mut publish = Publish {
        call: "gh api",
        ..Publish::default()
    };
    let endpoint = words[1..].iter().find_map(|word| {
        let path = word.trim_start_matches('/');
        let path = path.find("repos/").map_or(path, |at| &path[at..]);
        let parts: Vec<&str> = path.split(['/', '?']).collect();
        match parts[..] {
            [
                "repos",
                owner,
                repo,
                "pulls" | "contents" | "git" | "merges",
                ..,
            ] => Some(Some(format!("{owner}/{repo}"))),
            ["graphql"] => Some(None),
            _ => None,
        }
    })?;
    publish.anywhere = endpoint.is_none();
    publish.repo = endpoint.filter(|repo| !repo.contains('{'));
    let mut args = words[1..].iter();
    while let Some(arg) = args.next() {
        let path = match arg.strip_prefix("--input") {
            Some("") => args.next().cloned(),
            Some(rest) if rest.starts_with('=') => Some(rest[1..].to_owned()),
            _ => arg.split_once("=@").map(|(_, path)| path.to_owned()),
        };
        match path {
            Some(path) => file(&mut publish, Some(path)),
            None => publish.texts.push(("argument", arg.clone())),
        }
    }
    Some(publish)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn one(command: &str) -> Publish {
        let mut found = calls("Bash", Some(command), &Value::Null);
        assert_eq!(found.len(), 1, "{command}: {found:?}");
        found.remove(0)
    }

    #[test]
    fn a_push_names_its_remote_and_the_revisions_it_sends() {
        let sent = |command: &str| match one(command).commits {
            Commits::Push {
                remote,
                revisions,
                every,
            } => (remote, revisions.join(","), every),
            other => panic!("{other:?}"),
        };
        let origin = || Some("origin".to_owned());
        assert_eq!(sent("git push"), (None, String::new(), false));
        assert_eq!(
            sent("git -C x push -u origin HEAD:refs/heads/a +b"),
            (origin(), "HEAD,b".to_owned(), false)
        );
        assert_eq!(
            sent("bash -c 'git push -o ci.skip origin --all'"),
            (origin(), String::new(), true)
        );
        assert_eq!(one("cd a && git -C b push").dirs, ["a", "b"]);
        for free in [
            "git push -n",
            "git push --dry-run origin a",
            "git push origin :gone",
            "git push --delete origin gone",
            "git push --help",
            "gh pr view 3",
            "gh api repos/o/r/pulls/3",
            "echo 'git push'",
        ] {
            assert_eq!(calls("Bash", Some(free), &Value::Null), [], "{free}");
        }
        assert_eq!(one("git commit -m x && git push").unread, Some(MOVED));
        assert_eq!(one("cd $D && git push").unread, Some(NO_DIR));
    }

    #[test]
    fn a_pull_requests_text_is_read_wherever_the_call_holds_it() {
        let holds = |command: &str| {
            one(command)
                .texts
                .iter()
                .any(|(_, text)| text.contains("vault9"))
        };
        for command in [
            "gh pr create -t vault9 -b x",
            "gh -R o/r pr edit 3 --body='a vault9'",
            "gh pr edit 3 -b\"$(cat <<'EOF'\nvault9\nEOF\n)\"",
            "BODY='vault9'; gh pr edit 3 -b \"$BODY\"",
            "echo vault9 > b; gh pr edit 3 -F b",
            "gh api -X PATCH repos/o/r/pulls/3 -f body=vault9",
        ] {
            assert!(holds(command), "{command}");
        }
        assert!(!holds("gh pr edit 3 -F /vault9/body.md"));
        assert_eq!(one("gh pr edit 3 -dF body.md").files[0].path, "body.md");
        assert_eq!(one("cat b | gh pr edit 3 -F -").unread, Some(STDIN));
        let input = serde_json::json!({ "owner": "o", "repo": "r", "body": "vault9" });
        let found = calls("mcp__github__update_pull_request", None, &input);
        assert_eq!(found[0].repo.as_deref(), Some("o/r"));
        assert_eq!(found[0].texts, [("argument", "vault9".to_owned())]);
    }

    proptest! {
        #[test]
        fn reading_a_call_never_panics(
            words in prop::collection::vec(
                "git|gh|pr|api|create|edit|push|cd|-[a-zA-Z]{0,3}|--[a-z-]{0,8}=?|repos/o/r/pulls|\\PC{0,6}",
                0..14,
            ),
        ) {
            calls("Bash", Some(&words.join(" ")), &Value::Null);
        }
    }
}
