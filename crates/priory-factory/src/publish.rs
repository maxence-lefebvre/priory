//! Which tool calls publish to a git remote or a pull request, and what text they hold.

use crate::guard::{
    LITERAL, OPAQUE, PRINTED, SCOPE, SET, TEXT, VARIABLE, WRITES, dry_run, gh_request, help,
    moves_head, positional, read, starts,
};
use serde_json::Value;

/// A call that publishes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Publish {
    /// The call's name with none of its arguments, such as `git push`.
    pub call: &'static str,
    /// Each folder it may run in, as the folders its command changes to, in order.
    pub folders: Vec<Vec<String>>,
    /// Priory cannot tell which folder it runs in.
    pub lost: bool,
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
    /// Each folder its path may be relative to, as `Publish::folders` lists them.
    pub folders: Vec<Vec<String>>,
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
const OUTPUT: &str = "its text holds a variable or a command's output, which Priory cannot read: write it to a file in an earlier call and pass the file";
const WRITTEN: &str = "the same call writes a file it publishes, which Priory cannot read before it exists: write the file in an earlier call";
const REVISION: &str = "what it pushes holds a variable or a command's output: name the branch";

/// The most folders a call may run in before Priory gives up on them.
const FOLDERS: usize = 8;

/// Programs that leave the files they are given as they are.
const READS: &[&str] = &[
    "cat", "head", "tail", "ls", "test", "[", "[[", "grep", "rg", "wc", "diff", "echo", "printf",
    "rm", "gh",
];

/// The words of a call whose text Priory cannot read.
struct Unresolved {
    words: Vec<String>,
    /// A shell may run a script that expands, so no word of the call can be read.
    script: bool,
}

impl Unresolved {
    fn of(commands: &[Vec<String>]) -> Self {
        let noted = |note: &str| -> Vec<String> {
            commands
                .iter()
                .filter(|words| words[0] == note)
                .map(|words| words[1].clone())
                .collect()
        };
        let opaque = noted(OPAQUE);
        let variables = noted(VARIABLE);
        let script = opaque
            .iter()
            .chain(&variables)
            .any(|word| word.contains("gh ") && (word.contains("pr ") || word.contains("api ")));
        let literal = |name: &str| {
            let mut set = false;
            for words in commands {
                let named = |word: &String| {
                    word.strip_prefix(name)
                        .is_some_and(|rest| rest.is_empty() || rest.starts_with(['=', '+']))
                };
                if words[0] == TEXT {
                    if !named(&words[1]) {
                        continue;
                    }
                    let lasts = words.get(2).is_some_and(|word| word == SET);
                    if !lasts || opaque.contains(&words[1]) || variables.contains(&words[1]) {
                        return false;
                    }
                    set = true;
                } else if !words[0].starts_with('\0') && words.iter().any(named) {
                    return false;
                }
            }
            set
        };
        let unset = variables.iter().filter(|word| {
            !names(word).is_some_and(|names| names.iter().all(|name| literal(name)))
        });
        Self {
            words: unset.chain(&opaque).cloned().collect(),
            script,
        }
    }

    fn holds(&self, word: &str) -> bool {
        self.script || self.words.iter().any(|unresolved| unresolved == word)
    }
}

/// The variables `word` names as `$NAME` or `${NAME}`, or none when it expands another way.
fn names(word: &str) -> Option<Vec<&str>> {
    let mut names = Vec::new();
    for part in word.split('$').skip(1) {
        let (part, braced) = match part.strip_prefix('{') {
            Some(rest) => (rest, true),
            None => (part, false),
        };
        let end = part
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(part.len());
        let (name, rest) = part.split_at(end);
        if name.is_empty()
            || name.starts_with(|c: char| c.is_ascii_digit())
            || (braced && !rest.starts_with('}'))
        {
            return None;
        }
        names.push(name);
    }
    Some(names)
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

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
    let unresolved = Unresolved::of(&commands);
    let mut folders: Vec<Vec<String>> = vec![Vec::new()];
    let mut leading = true;
    let mut lost = false;
    let mut files = Vec::new();
    let mut found: Vec<(usize, Publish)> = Vec::new();
    for (at, words) in commands.iter().enumerate() {
        match words[0].as_str() {
            SCOPE => leading = false,
            "cd" | "pushd" => match positional(words)[..] {
                [dir]
                    if dir != "-"
                        && !dir.contains(['$', '~', '*'])
                        && !dir.is_empty()
                        && !unresolved.holds(dir) =>
                {
                    let mut moved = folders.clone();
                    moved
                        .iter_mut()
                        .for_each(|folder| folder.push(dir.to_owned()));
                    if leading {
                        folders = moved;
                    } else {
                        moved.retain(|folder| !folders.contains(folder));
                        folders.extend(moved);
                    }
                    if folders.len() > FOLDERS {
                        lost = true;
                        folders.truncate(FOLDERS);
                    }
                }
                _ => lost = true,
            },
            "popd" => lost = true,
            program if program == PRINTED || PRINTS.contains(&program) => {
                files.extend(words[1..].iter().map(|path| File {
                    path: path.clone(),
                    needed: program == PRINTED,
                    folders: folders.clone(),
                }));
            }
            _ => {}
        }
        let publish = push(words, &unresolved)
            .or_else(|| pull(words, &unresolved))
            .or_else(|| api(words, &unresolved));
        if let Some(mut publish) = publish {
            publish.lost |= lost;
            let own = publish.folders.pop().unwrap_or_default();
            publish.folders = folders.clone();
            for folder in &mut publish.folders {
                folder.extend(own.iter().cloned());
            }
            for file in &mut publish.files {
                file.folders = folders.clone();
            }
            found.push((at, publish));
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
    let heredoc_expands = commands
        .iter()
        .any(|words| words[0] == TEXT && unresolved.holds(&words[1]));
    for (at, publish) in &mut found {
        publish.anywhere |= redirected;
        if matches!(publish.commits, Commits::Push { .. }) {
            if moves_head(command) {
                publish.unread = Some(MOVED);
            }
            continue;
        }
        publish.texts.extend(around.iter().cloned());
        publish.files.extend(files.iter().cloned());
        for file in publish.files.iter_mut().filter(|file| file.needed) {
            let name = file_name(&file.path).to_owned();
            for (place, words) in commands.iter().enumerate() {
                let writes =
                    words[0] == WRITES && (words[1].is_empty() || file_name(&words[1]) == name);
                let literal = words.get(2).is_some_and(|word| word == LITERAL);
                let mentions = place < *at
                    && !words[0].starts_with('\0')
                    && !READS.contains(&words[0].as_str())
                    && words[1..].iter().any(|word| file_name(word) == name);
                if writes && literal && !heredoc_expands {
                    file.needed = false;
                } else if writes || mentions {
                    publish.unread = Some(WRITTEN);
                }
            }
        }
    }
    found.into_iter().map(|(_, publish)| publish).collect()
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
        folders: vec![Vec::new()],
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
fn push(words: &[String], unresolved: &Unresolved) -> Option<Publish> {
    if words[0] != "git" || positional(words).first() != Some(&"push") || help(words) {
        return None;
    }
    let mut publish = Publish {
        call: "git push",
        ..Publish::default()
    };
    let mut own = Vec::new();
    let mut args = words[1..].iter();
    while let Some(arg) = args.next().filter(|arg| *arg != "push") {
        match arg.as_str() {
            "-C" => {
                let dir = args.next();
                publish.lost |= dir.is_some_and(|dir| unresolved.holds(dir));
                own.extend(dir.cloned());
            }
            "-c" | "--config-env" | "--namespace" => {
                args.next();
                publish.anywhere = true;
            }
            "--git-dir" | "--work-tree" => {
                args.next();
                publish.lost = true;
            }
            _ if arg.starts_with("--git-dir=") || arg.starts_with("--work-tree=") => {
                publish.lost = true;
            }
            _ if arg.starts_with("--config-env=") => publish.anywhere = true,
            _ => {}
        }
    }
    publish.folders = vec![own];
    if dry_run(args.as_slice()) {
        return None;
    }
    if args.as_slice().iter().any(|arg| unresolved.holds(arg)) {
        publish.unread = Some(REVISION);
    }
    let (mut remote, mut specs) = (None, Vec::new());
    let (mut every, mut deletes) = (false, false);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--all" | "--branches" | "--mirror" | "--tags" => every = true,
            "--delete" => deletes = true,
            "--no-delete" => deletes = false,
            "--repo" => remote = args.next().cloned(),
            "--push-option" | "--receive-pack" | "--exec" => {
                args.next();
            }
            _ if arg.starts_with("--repo=") => remote = Some(arg["--repo=".len()..].to_owned()),
            _ if arg.starts_with("--") => {}
            _ if arg.starts_with('-') && arg.len() > 1 => {
                for (at, flag) in arg[1..].char_indices() {
                    match flag {
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
    if deletes && publish.unread.is_none() {
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
    if named && revisions.is_empty() && !every && publish.unread.is_none() {
        return None;
    }
    publish.commits = Commits::Push {
        remote,
        revisions,
        every,
    };
    Some(publish)
}

/// `gh pr create`, `gh pr edit` and `gh pr merge`.
fn pull(words: &[String], unresolved: &Unresolved) -> Option<Publish> {
    let create =
        starts(words, &["gh", "pr", "create"], &[]) || starts(words, &["gh", "pr", "new"], &[]);
    let call = if create {
        "gh pr create"
    } else if starts(words, &["gh", "pr", "edit"], &[]) {
        "gh pr edit"
    } else if starts(words, &["gh", "pr", "merge"], &[]) {
        "gh pr merge"
    } else {
        return None;
    };
    if help(words) {
        return None;
    }
    let switches: &[char] = match call {
        "gh pr merge" => &['d', 'm', 'r', 's'],
        _ => &['d', 'e', 'f', 'w'],
    };
    let mut publish = Publish {
        call,
        ..Publish::default()
    };
    let mut head = None;
    let mut args = words[1..].iter();
    while let Some(arg) = args.next() {
        let (name, attached) = option(arg, switches);
        let next = args.as_slice().first().filter(|_| attached.is_none());
        let text = matches!(
            name.as_str(),
            "-F" | "--body-file"
                | "-T"
                | "--template"
                | "-t"
                | "--title"
                | "--subject"
                | "-b"
                | "--body"
        );
        if text && (unresolved.holds(arg) || next.is_some_and(|next| unresolved.holds(next))) {
            publish.unread = Some(OUTPUT);
        }
        let mut value = || attached.clone().or_else(|| args.next().cloned());
        match name.as_str() {
            "-R" | "--repo" => publish.repo = value(),
            "-F" | "--body-file" | "-T" | "--template" => file(&mut publish, value()),
            "-H" | "--head" => head = value(),
            "-t" | "--title" | "--subject" => {
                publish.texts.extend(value().map(|text| ("PR title", text)))
            }
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
/// `-dF`, after its `switches`.
fn option(arg: &str, switches: &[char]) -> (String, Option<String>) {
    if arg.starts_with("--") {
        return match arg.split_once('=') {
            Some((name, value)) => (name.to_owned(), Some(value.to_owned())),
            None => (arg.to_owned(), None),
        };
    }
    let Some(cluster) = arg.strip_prefix('-') else {
        return (arg.to_owned(), None);
    };
    let rest = cluster.trim_start_matches(switches);
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
            folders: Vec::new(),
        }),
        None => {}
    }
}

/// `gh api` writes to a repository's pull requests, issues, files or git data, or to GraphQL.
fn api(words: &[String], unresolved: &Unresolved) -> Option<Publish> {
    if words[0] != "gh" {
        return None;
    }
    let request = gh_request(words)?;
    let mut publish = Publish {
        call: "gh api",
        ..Publish::default()
    };
    let parts: Vec<&str> = request.endpoint.split('/').collect();
    let holder = match parts[..] {
        [
            "repos",
            owner,
            repo,
            "pulls" | "issues" | "contents" | "git" | "merges",
            ..,
        ] => Some(format!("{owner}/{repo}")),
        ["graphql"] | ["repositories", _, _, ..] => None,
        // The repository may be what expands.
        _ if unresolved.holds(&request.raw) => None,
        _ => return None,
    };
    publish.anywhere = holder.is_none();
    publish.repo = holder.filter(|repo| !repo.contains('{'));
    if words[1..]
        .iter()
        .any(|word| *word != request.raw && unresolved.holds(word))
    {
        publish.unread = Some(OUTPUT);
    }
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
        assert_eq!(one("cd a && git -C b push").folders, [["a", "b"]]);
        for free in [
            "git push -n",
            "git push --dry-run origin a",
            "git push origin :gone",
            "git push --delete origin gone",
            "git push --help",
            "git push --no-dry-run -n",
            "gh pr view 3",
            "gh api repos/o/r/pulls/3",
            "echo 'git push'",
        ] {
            assert_eq!(calls("Bash", Some(free), &Value::Null), [], "{free}");
        }
        assert_eq!(one("git commit -m x && git push").unread, Some(MOVED));
        assert!(one("cd $D && git push").lost);
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

    #[test]
    fn a_cd_that_may_not_last_keeps_the_folder_before_it() {
        for (command, folders) in [
            ("cd a && cd b; git push", "a/b"),
            ("(cd a && ls); git push", ",a"),
            ("bash -c 'cd a'; git push", ",a"),
            ("x=$(cd a && pwd); git push", ",a"),
            ("cd a | cat; git push", ",a"),
            ("cd a & git push", ",a"),
            ("cd a || true; git push", ",a"),
            ("false && cd a; git push", ",a"),
            ("if false; then cd a; fi; git push", ",a"),
            ("f() { cd a; }; git push", ",a"),
            ("{ cd a; } | cat; git push", ",a"),
            ("env cd a; git push", ",a"),
            ("cd a && gh pr edit 3 -b \"$(cat <<'E'\nx\nE\n)\"", "a"),
        ] {
            let found: Vec<String> = one(command)
                .folders
                .iter()
                .map(|folder| folder.join("/"))
                .collect();
            assert_eq!(found.join(","), folders, "{command}");
        }
        assert!(one("(cd a; cd b; cd c; cd d); git push").lost);
    }

    #[test]
    fn a_text_that_is_not_in_the_call_or_a_file_is_unread() {
        for (command, why) in [
            ("gh pr edit 3 -b \"$(notes)\"", Some(OUTPUT)),
            ("gh pr edit 3 --title=`notes`", Some(OUTPUT)),
            ("gh pr edit 3 -b \"$VAR\"", Some(OUTPUT)),
            ("VAR=$(notes); gh pr edit 3 -b \"$VAR\"", Some(OUTPUT)),
            ("VAR=x gh pr edit 3 -b \"$VAR\"", Some(OUTPUT)),
            ("VAR=x; read VAR; gh pr edit 3 -b $VAR", Some(OUTPUT)),
            ("bash -c \"gh pr edit 3 -b '$VAR'\"", Some(OUTPUT)),
            ("gh pr edit 3 -b \"$(cat f | tr a b)\"", Some(OUTPUT)),
            ("gh pr edit 3 -b \"$(cat <<E\n$VAR\nE\n)\"", Some(OUTPUT)),
            ("gh pr merge 3 -sb \"$(notes)\"", Some(OUTPUT)),
            (
                "gh api repos/o/r/issues/3 -f body=\"$(notes)\"",
                Some(OUTPUT),
            ),
            ("gen > b.md; gh pr edit 3 -F ./b.md", Some(WRITTEN)),
            ("cp n b.md; gh pr edit 3 -F b.md", Some(WRITTEN)),
            ("(gen) > b.md; gh pr edit 3 -F b.md", Some(WRITTEN)),
            ("gen > $F; gh pr edit 3 -F b.md", Some(WRITTEN)),
            (
                "cat > b.md <<E\n$(gen)\nE\ngh pr edit 3 -F b.md",
                Some(WRITTEN),
            ),
            ("gen > f; gh pr edit 3 -b \"$(cat f)\"", Some(WRITTEN)),
            ("git push origin \"$(branch)\"", Some(REVISION)),
            ("gh pr edit 3 -b '$VAR and `x`'", None),
            ("gh pr edit 3 -b \"costs \\$5\"", None),
            ("gh pr edit 3 -b \"$(cat f)\"", None),
            ("VAR=x; gh pr edit 3 -b \"${VAR}s\"", None),
            (
                "cat > b.md <<'E'\n$x\nE\ngh pr edit 3 -F b.md && rm b.md",
                None,
            ),
            ("cat b.md; gh pr edit $N -F b.md", None),
            ("gh api repos/o/r/pulls/$N -f body=x", None),
        ] {
            assert_eq!(one(command).unread, why, "{command}");
        }
        let printed = one("gh pr edit 3 -b \"$(cat f)\"");
        assert!(printed.files.iter().any(|file| file.needed));
    }

    #[test]
    fn a_github_write_goes_where_its_endpoint_says() {
        let link =
            "gh api -f body=https://github.com/repos/o/other/pulls/1 -X PATCH repos/o/r/pulls/3";
        assert_eq!(one(link).repo.as_deref(), Some("o/r"));
        let issue = one("gh api -X PATCH repos/o/r/issues/3 -f body=vault9");
        assert_eq!(issue.repo.as_deref(), Some("o/r"));
        assert!(one("gh api \"$E\" -f body=x").anywhere);
        let label = "gh api repos/o/r/labels -f name=x";
        assert_eq!(calls("Bash", Some(label), &Value::Null), []);
        let merge = one("gh pr merge 3 -R o/r -sd --subject vault9 -b x");
        assert_eq!(merge.repo.as_deref(), Some("o/r"));
        assert!(merge.texts.contains(&("PR title", "vault9".to_owned())));
        one("git push -n --no-dry-run");
        one("git push --dry-run --no-dry-run");
        one("git push -d --no-delete origin a");
    }

    proptest! {
        #[test]
        fn reading_a_call_never_panics(
            words in prop::collection::vec(
                "git|gh|pr|api|create|edit|merge|push|cd|\\(|\\)|\\$\\(|\\$[A-Z]|[;|&>`]|-[a-zA-Z]{0,3}|--[a-z-]{0,8}=?|repos/o/r/pulls|\\PC{0,6}",
                0..14,
            ),
        ) {
            calls("Bash", Some(&words.join(" ")), &Value::Null);
        }
    }
}
