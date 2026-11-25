//! Which tool calls are irreversible: built-in guards on shell commands and MCP tools, and the
//! ones a factory adds under `[guard]`. Matching reads the parsed command, so wrappers such as
//! `bash -c`, `env` or `sudo` and options such as `gh -R` do not hide it.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// `[guard]` in `factory.toml`: the irreversible calls the factory adds to the built-in ones.
#[derive(Debug, Default, Clone, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Guards {
    /// Shell commands, each a guard of its own name.
    #[serde(default)]
    pub commands: Vec<CommandGuard>,
    /// MCP tools by full name, such as `mcp__stripe__create_refund`, or a prefix ending in `*`.
    #[serde(default)]
    pub tools: Vec<String>,
}

/// A shell command that is irreversible: its program, then the first words after its options.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CommandGuard {
    /// The guard's name, which a workflow's `guard.allow` names.
    pub name: String,
    /// The program and the words it starts with, such as `["fly", "deploy"]`.
    pub run: Vec<String>,
    /// When set, the command matches only with one of these options, such as `--prod`.
    #[serde(default)]
    pub flags: Vec<String>,
}

/// A guarded call: the guard it matched and the command or tool that matched it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub guard: String,
    pub call: String,
}

/// The built-in command guards: name, program and leading words, and options one of which must be
/// present.
const COMMANDS: &[(&str, &[&str], &[&str])] = &[
    ("gh-pr-merge", &["gh", "pr", "merge"], &[]),
    (
        "gh-pr-approve",
        &["gh", "pr", "review"],
        &["--approve", "-a"],
    ),
    ("gh-pr-comment", &["gh", "pr", "comment"], &[]),
    ("gh-issue-comment", &["gh", "issue", "comment"], &[]),
    ("linear-write", &["linear", "issue", "create"], &[]),
    ("linear-write", &["linear", "issue", "update"], &[]),
    ("linear-write", &["linear", "issue", "delete"], &[]),
    ("linear-write", &["linear", "issue", "comment"], &[]),
    ("slack-write", &["slack", "chat", "send"], &[]),
];

/// MCP tools that are irreversible whatever their server is named: `mcp__<server>__<tool>`.
const TOOLS: &[&str] = &[
    "add_issue_comment",
    "send_message",
    "slack_send_message",
    "schedule_message",
    "slack_schedule_message",
    "save_comment",
    "create_comment",
    "save_issue",
    "create_issue",
    "update_issue",
];

/// MCP tools that a command guard covers. The guard cannot read a tool's arguments, so any review
/// counts as an approval and any file write as a push to the default branch.
const NAMED_TOOLS: &[(&[&str], &str)] = &[
    (
        &["merge_pull_request", "enable_pr_auto_merge"],
        "gh-pr-merge",
    ),
    (
        &[
            "pull_request_review_write",
            "create_pull_request_review",
            "create_and_submit_pull_request_review",
            "submit_pending_pull_request_review",
        ],
        "gh-pr-approve",
    ),
    (
        &["push_files", "create_or_update_file", "delete_file"],
        "git-push-default",
    ),
];

/// Every built-in guard's name, for `check` and `--explain`.
pub const BUILT_IN: &[&str] = &[
    "gh-pr-merge",
    "gh-pr-approve",
    "gh-pr-comment",
    "gh-issue-comment",
    "gh-api-write",
    "git-push-default",
    "git-push-force",
    "slack-write",
    "linear-write",
    "mcp-write",
    SIGN_IN,
    PERSON,
    UNREADABLE,
];

/// The guards only a person's approval grants: never `autonomous` nor a workflow's `guard.allow`.
pub const NEED_APPROVAL: &[&str] = &["gh-pr-merge", "gh-pr-approve"];

/// The guard on an MCP server's `authenticate` and `complete_authentication` tools, which only an
/// agent with `connectorSignIn` passes.
pub const SIGN_IN: &str = "connector-sign-in";

/// The MCP tools that sign in to a connector.
pub const SIGN_IN_TOOLS: &[&str] = &["authenticate", "complete_authentication"];

/// The guard on Priory's own person verbs, which nothing grants to a run.
pub const PERSON: &str = "priory-person";

/// The guard on a call that publishes one of the factory's private terms, which nothing grants.
pub const PRIVATE: &str = "private-terms";

/// The guard on a script nested too deep for the guard to read, which nothing grants.
pub const UNREADABLE: &str = "unreadable";

/// The `priory` verbs only a person runs, as an answer approves a step. `report pick`,
/// `memories accept`, `memories drop`, `batch edit`, `score --calibrate` and `mcp --outside` are too.
pub const PERSON_VERBS: &[&str] = &[
    "answer",
    "dismiss",
    "deliver",
    "resume",
    "revive",
    "cancel",
    "release",
    "hold",
    "move",
    "say",
    "pause",
    "unpause",
    "snooze",
    "seen",
    "subscribe",
    "unsubscribe",
    "give",
    "eval",
    "bench",
    "label",
    "upgrade",
];

/// Whether `args`, the arguments after `priory`, run a person verb.
pub fn person_verb(args: &[String]) -> bool {
    let mut words = vec!["priory".to_owned()];
    words.extend_from_slice(args);
    match positional(&words)[..] {
        ["report", "pick", ..]
        | ["memories", "accept" | "drop", ..]
        | ["batch", "edit", ..]
        | ["patterns", "close" | "dismiss" | "capture", ..] => true,
        ["score", ..] => args
            .iter()
            .any(|arg| arg == "--calibrate" || arg.starts_with("--calibrate=")),
        // A person's own session serves it, and its `answer` and `dismiss` decide asks.
        ["mcp", ..] => args.iter().any(|arg| arg == "--outside"),
        [verb, ..] => PERSON_VERBS.contains(&verb),
        [] => false,
    }
}

/// The hits of the `priory-person` and `unreadable` guards alone in the shell command `command`,
/// for a person's own session: every other guard is that person's business.
pub fn person(command: &str) -> Vec<Hit> {
    let no_git = |_: Option<&str>, _: &[&str]| None;
    Guards::default()
        .hits("Bash", Some(command), &no_git)
        .into_iter()
        .filter(|hit| hit.guard == PERSON || hit.guard == UNREADABLE)
        .collect()
}

/// Runs git with these arguments, in the `-C` directory if any, and returns its trimmed output.
pub type Git<'a> = &'a dyn Fn(Option<&str>, &[&str]) -> Option<String>;

impl Guards {
    /// Every guard's name: the built-in ones, then the factory's commands and tools.
    pub fn names(&self) -> Vec<String> {
        BUILT_IN
            .iter()
            .map(|name| name.to_string())
            .chain(self.commands.iter().map(|command| command.name.clone()))
            .chain(self.tools.iter().cloned())
            .collect()
    }

    /// Every guard that the call of `tool` matches, one per command it runs. `command` is a shell
    /// tool's command.
    pub fn hits(&self, tool: &str, command: Option<&str>, git: Git) -> Vec<Hit> {
        if let Some(name) = tool.strip_prefix("mcp__") {
            return self.tool(tool, name).into_iter().collect();
        }
        let Some(command) = command else {
            return Vec::new();
        };
        commands(command)
            .iter()
            .filter_map(|words| self.command(words, git))
            .collect()
    }

    /// A factory tool guard is named by its pattern as written, so `guard.allow` can name it.
    fn tool(&self, tool: &str, name: &str) -> Option<Hit> {
        let (server, short) = name.rsplit_once("__")?;
        if server == "priory" {
            return None;
        }
        if SIGN_IN_TOOLS.contains(&short) {
            return Some(Hit {
                guard: SIGN_IN.to_owned(),
                call: tool.to_owned(),
            });
        }
        let built_in = NAMED_TOOLS
            .iter()
            .find(|(names, _)| names.contains(&short))
            .map(|(_, guard)| *guard)
            .or_else(|| TOOLS.contains(&short).then_some("mcp-write"));
        let added = self
            .tools
            .iter()
            .find(|pattern| match pattern.strip_suffix('*') {
                Some(prefix) => tool.starts_with(prefix),
                None => *pattern == tool,
            });
        let guard = match (built_in, added) {
            (Some(name), _) if NEED_APPROVAL.contains(&name) => name.to_owned(),
            (_, Some(pattern)) => pattern.clone(),
            (Some(name), None) => name.to_owned(),
            (None, None) => return None,
        };
        Some(Hit {
            guard,
            call: tool.to_owned(),
        })
    }

    fn command(&self, words: &[String], git: Git) -> Option<Hit> {
        let hit = |guard: &str| {
            Some(Hit {
                guard: guard.to_owned(),
                call: words.join(" "),
            })
        };
        if words[0] == TOO_DEEP {
            return Some(Hit {
                guard: UNREADABLE.to_owned(),
                call: format!("a script nested more than {DEPTH} levels deep"),
            });
        }
        if words[0] == UNSPLIT {
            return Some(Hit {
                guard: UNREADABLE.to_owned(),
                call: "an env -S string that expands a variable or that env refuses".to_owned(),
            });
        }
        if help(words) {
            return None;
        }
        let built_in = COMMANDS
            .iter()
            .find(|(_, run, flags)| starts(words, run, flags))
            .map(|(name, _, _)| *name)
            .or_else(|| match words[0].as_str() {
                "gh" => gh_api(words, git),
                "git" => git_push(words, git),
                "curl" | "wget" => curl(words),
                "priory" => person_verb(&words[1..]).then_some(PERSON),
                _ => None,
            });
        // A factory guard must not rename a merge or an approval, which only a person grants.
        if let Some(name) = built_in.filter(|name| NEED_APPROVAL.contains(name) || *name == PERSON)
        {
            return hit(name);
        }
        let added = self
            .commands
            .iter()
            .find(|guard| starts(words, &guard.run, &guard.flags));
        match added {
            Some(guard) => hit(&guard.name),
            None => built_in.and_then(hit),
        }
    }
}

/// Whether `words` asks for the program's help. A help word after an option may be that option's
/// value, as pflag and git take it, so it counts only after a word that is no option.
pub(crate) fn help(words: &[String]) -> bool {
    let short = matches!(words[0].as_str(), "gh" | "git");
    words
        .iter()
        .take_while(|word| *word != "--")
        .collect::<Vec<_>>()
        .windows(2)
        .any(|pair| {
            let is_help = pair[1] == "--help" || (short && pair[1] == "-h");
            is_help && !pair[0].starts_with('-')
        })
}

/// Whether `words` runs `run[0]` with positional words starting with the rest of `run`, and holds
/// one of `flags` if any.
pub(crate) fn starts<S: AsRef<str>>(words: &[String], run: &[S], flags: &[S]) -> bool {
    let Some((program, rest)) = run.split_first() else {
        return false;
    };
    if words[0] != program.as_ref() {
        return false;
    }
    let positional = positional(words);
    let leads = positional.len() >= rest.len()
        && rest
            .iter()
            .zip(&positional)
            .all(|(want, got)| want.as_ref() == *got);
    let flagged = flags.is_empty()
        || words[1..].iter().any(|word| {
            flags.iter().any(|flag| {
                let flag = flag.as_ref();
                word == flag
                    || word
                        .strip_prefix(flag)
                        .is_some_and(|rest| rest.starts_with('='))
                    || clustered(word, flag)
            })
        });
    leads && flagged
}

/// Whether the short `flag` is a letter in `word`'s cluster, as `-a` in `-ab`. A letter after one
/// that takes a value is that value's, which the guard cannot tell, so it counts too.
fn clustered(word: &str, flag: &str) -> bool {
    let (Some(cluster), Some(letter)) = (word.strip_prefix('-'), flag.strip_prefix('-')) else {
        return false;
    };
    let mut letters = letter.chars();
    let (Some(letter), None) = (letters.next(), letters.next()) else {
        return false;
    };
    !cluster.starts_with('-')
        && cluster
            .chars()
            .take_while(char::is_ascii_alphanumeric)
            .any(|c| c == letter)
}

/// Options of a program that take the next word as their value.
pub(crate) fn takes_value(program: &str, option: &str) -> bool {
    let options: &[&str] = match program {
        "gh" => &["-R", "--repo"],
        "git" => &["-C", "-c", "--git-dir", "--work-tree", "--namespace"],
        _ => &[],
    };
    options.contains(&option)
}

/// The arguments after the program that are no option nor an option's value.
pub(crate) fn positional(words: &[String]) -> Vec<&str> {
    let mut positional = Vec::new();
    let mut args = words[1..].iter();
    while let Some(arg) = args.next() {
        if arg == "--" {
            positional.extend(args.map(String::as_str));
            break;
        }
        if !arg.starts_with('-') || arg == "-" {
            positional.push(arg.as_str());
        } else if takes_value(&words[0], arg) {
            args.next();
        }
    }
    positional
}

/// A `gh api` call that writes: a method other than GET, or a field or body, which makes it a POST.
pub(crate) struct Request {
    method: Option<String>,
    pub fields: Vec<String>,
    /// Its body, or a field, comes from a file or stdin.
    pub opaque: bool,
    /// The endpoint argument as written.
    pub raw: String,
    /// The endpoint's path, with no host, query or leading slash.
    pub endpoint: String,
}

pub(crate) fn gh_request(words: &[String]) -> Option<Request> {
    if positional(words).first() != Some(&"api") {
        return None;
    }
    let mut method = None;
    let mut fields = Vec::new();
    let mut opaque = false;
    let mut endpoint = None;
    let mut api = false;
    let mut args = words[1..].iter();
    while let Some(arg) = args.next() {
        let (option, attached) = match arg.split_once('=') {
            Some((option, value)) if arg.starts_with("--") => (option, Some(value.to_owned())),
            _ => (arg.as_str(), None),
        };
        let mut value = || attached.clone().or_else(|| args.next().cloned());
        match option {
            "-X" | "--method" => method = value(),
            "-f" | "-F" | "--field" | "--raw-field" => fields.extend(value()),
            "--input" => opaque = value().is_some(),
            "-R" | "--repo" | "-H" | "--header" | "-q" | "--jq" | "-t" | "--template"
            | "--hostname" | "-p" | "--preview" | "--cache" => drop(value()),
            _ if arg.starts_with("-X") && arg.len() > 2 => method = Some(arg[2..].to_owned()),
            _ if (arg.starts_with("-f") || arg.starts_with("-F")) && arg.len() > 2 => {
                fields.push(arg[2..].to_owned())
            }
            _ if arg.starts_with('-') => {}
            _ if !api => api = true,
            _ => endpoint = endpoint.or_else(|| Some(arg.clone())),
        }
    }
    opaque |= fields.iter().any(|field| field.contains("=@"));
    let writes = match &method {
        Some(method) => !method.eq_ignore_ascii_case("GET"),
        None => opaque || !fields.is_empty(),
    };
    if !writes {
        return None;
    }
    let raw = endpoint.unwrap_or_default();
    let endpoint = raw.split_once("://").map_or(raw.as_str(), |(_, rest)| {
        rest.split_once('/').map_or("", |(_, path)| path)
    });
    let endpoint = endpoint.trim_start_matches('/');
    let endpoint = endpoint.strip_prefix("api/v3/").unwrap_or(endpoint);
    let endpoint = endpoint.split('?').next().unwrap_or_default();
    let endpoint = if endpoint == "api/graphql" {
        "graphql"
    } else {
        endpoint
    }
    .to_owned();
    Some(Request {
        method,
        fields,
        opaque,
        raw,
        endpoint,
    })
}

/// `gh api` that writes. A merge or an approving review is named as such, and so is one whose body
/// Priory cannot read.
fn gh_api(words: &[String], git: Git) -> Option<&'static str> {
    let Request {
        method,
        fields,
        opaque,
        endpoint,
        ..
    } = gh_request(words)?;
    let parts: Vec<&str> = endpoint.split('/').collect();
    let pull = |tail: &str| {
        matches!(parts.as_slice(), ["repos", _, _, "pulls", number, first, ..]
            if number.parse::<u64>().is_ok() && *first == tail)
    };
    if pull("merge") || matches!(parts.as_slice(), ["repos", _, _, "merges"]) {
        return Some("gh-pr-merge");
    }
    if let ["repos", _, _, "git", "refs", "heads", branch @ ..] = parts.as_slice() {
        let deletes = method
            .as_deref()
            .is_some_and(|m| m.eq_ignore_ascii_case("DELETE"));
        if deletes || fields.iter().any(|field| field == "force=true") {
            return Some("git-push-force");
        }
        if is_default(git, None, &branch.join("/")) {
            return Some("git-push-default");
        }
    }
    if pull("reviews") && (opaque || fields.iter().any(|field| field == "event=APPROVE")) {
        return Some("gh-pr-approve");
    }
    if endpoint == "graphql" {
        // Only the query names the operation; the other fields are its variables.
        let query = fields
            .iter()
            .find_map(|field| field.strip_prefix("query="))
            .unwrap_or_default();
        if opaque
            || query.contains("mergePullRequest")
            || query.contains("enablePullRequestAutoMerge")
        {
            return Some("gh-pr-merge");
        }
        let approve = approves(query) || fields.iter().any(|field| field.ends_with("=APPROVE"));
        if query.contains("PullRequestReview") && approve {
            return Some("gh-pr-approve");
        }
    }
    Some("gh-api-write")
}

/// Whether `text` holds the word `APPROVE`, not `APPROVED`.
fn approves(text: &str) -> bool {
    text.match_indices("APPROVE").any(|(at, word)| {
        !text[at + word.len()..].starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_')
    })
}

/// Whether `command` also runs a git command that moves the branch's commit, so an approval given
/// at the commit before it does not cover what follows.
pub fn moves_head(command: &str) -> bool {
    const MOVES: &[&str] = &[
        "commit",
        "merge",
        "rebase",
        "cherry-pick",
        "am",
        "reset",
        "pull",
        "revert",
        "checkout",
        "switch",
        "stash",
        "update-ref",
    ];
    commands(command).iter().any(|words| {
        words[0] == "git"
            && positional(words)
                .first()
                .is_some_and(|sub| MOVES.contains(sub))
    })
}

/// Whether a call opens a pull request, pushes, or ends or sleeps the run: the calls Priory holds
/// once while news of the task is unread. A dry run or a help request is not held.
pub fn holds_for_news(tool: &str, command: Option<&str>) -> bool {
    if let Some(name) = tool.strip_prefix("mcp__") {
        return name.rsplit_once("__").is_some_and(|(server, short)| {
            if server == "priory" {
                return matches!(short, "result" | "ask" | "wait" | "consult" | "hand_over");
            }
            short == "create_pull_request"
        });
    }
    let Some(command) = command else {
        return false;
    };
    commands(command).iter().any(|words| {
        let pushes = words[0] == "git"
            && positional(words).first() == Some(&"push")
            && words
                .iter()
                .position(|word| word == "push")
                .is_some_and(|at| !dry_run(&words[at + 1..]));
        !help(words) && (pushes || starts(words, &["gh", "pr", "create"], &[]))
    })
}

/// Whether a `git push` with `args` sends nothing: its last `--dry-run` or `--no-dry-run` decides.
pub(crate) fn dry_run(args: &[String]) -> bool {
    let mut dry = false;
    let mut args = args.iter();
    while let Some(arg) = args.next().filter(|arg| *arg != "--") {
        match arg.as_str() {
            "--dry-run" => dry = true,
            // Git takes an unambiguous start of an option.
            _ if arg.starts_with("--no-dr") => dry = false,
            "--repo" | "--push-option" | "--receive-pack" | "--exec" | "--recurse-submodules" => {
                args.next();
            }
            _ if arg.starts_with("--") => {}
            _ if arg.starts_with('-') => {
                let flags = arg[1..].split('o').next().unwrap_or_default();
                dry |= flags.contains('n');
                if flags.len() + 2 == arg.len() && arg.ends_with('o') {
                    args.next();
                }
            }
            _ => {}
        }
    }
    dry
}

/// `git push` that forces, deletes, or updates the default branch. A push of every branch or of
/// a wildcard counts as one to the default branch.
fn git_push(words: &[String], git: Git) -> Option<&'static str> {
    let mut dir = None;
    let mut configured = false;
    let mut mirrored = false;
    let mut args = words[1..].iter();
    loop {
        let arg = args.next()?;
        if arg == "-C" {
            dir = args.next().map(String::as_str);
        } else if arg == "-c" || arg == "--config-env" {
            let setting = args.next().map_or("", String::as_str);
            configured |= routes_push(setting);
            mirrored |= mirrors(setting);
        } else if let Some(setting) = arg.strip_prefix("--config-env=") {
            configured |= routes_push(setting);
            mirrored |= mirrors(setting);
        } else if takes_value("git", arg) {
            args.next();
        } else if !arg.starts_with('-') {
            if arg != "push" {
                return None;
            }
            break;
        }
    }
    if dry_run(args.as_slice()) {
        return None;
    }
    let mut refspecs = Vec::new();
    let mut remote_given = false;
    let mut every = false;
    while let Some(arg) = args.next() {
        let arg = arg.as_str();
        match arg {
            "--mirror" | "--delete" | "--prune" => return Some("git-push-force"),
            _ if arg.starts_with("--force") => return Some("git-push-force"),
            "--all" | "--branches" => every = true,
            "--repo" => {
                remote_given = true;
                args.next();
            }
            _ if arg.starts_with("--repo=") => remote_given = true,
            "--push-option" | "--receive-pack" | "--exec" => {
                args.next();
            }
            _ if arg.starts_with("--") => {}
            _ if arg.starts_with('-') && arg.len() > 1 => {
                for (index, flag) in arg[1..].char_indices() {
                    match flag {
                        'f' | 'd' => return Some("git-push-force"),
                        'o' => {
                            // `-o value` or `-ovalue`: the rest of the word is the value.
                            if index == arg.len() - 2 {
                                args.next();
                            }
                            break;
                        }
                        _ => {}
                    }
                }
            }
            _ => refspecs.push(arg),
        }
    }
    let configured_mirror = || {
        git(dir, &["config", "--get-regexp", r"^remote\..*\.mirror$"])
            .is_some_and(|lines| lines.lines().any(|line| line.ends_with(" true")))
    };
    if mirrored || configured_mirror() {
        return Some("git-push-force");
    }
    let refspecs = if remote_given {
        &refspecs[..]
    } else {
        refspecs.get(1..).unwrap_or_default()
    };
    if refspecs
        .iter()
        .any(|spec| spec.starts_with('+') || spec.starts_with(':'))
    {
        return Some("git-push-force");
    }
    if configured || every || refspecs.iter().any(|spec| spec.contains('*')) {
        return Some("git-push-default");
    }
    let current = || git(dir, &["symbolic-ref", "--short", "HEAD"]);
    let targets: Vec<String> = if refspecs.is_empty() {
        // Git's config, such as `push.default=upstream`, may send it elsewhere than the branch.
        let destination = git(dir, &["rev-parse", "--abbrev-ref", "@{push}"])
            .and_then(|push| push.split_once('/').map(|(_, branch)| branch.to_owned()));
        destination.into_iter().chain(current()).collect()
    } else {
        refspecs
            .iter()
            .filter_map(|spec| {
                let target = spec.rsplit(':').next().unwrap_or(spec);
                let target = target.strip_prefix("refs/heads/").unwrap_or(target);
                match target {
                    "HEAD" | "@" => current(),
                    _ => Some(target.to_owned()),
                }
            })
            .collect()
    };
    targets
        .iter()
        .any(|target| is_default(git, dir, target))
        .then_some("git-push-default")
}

/// Whether a `-c` or `--config-env` setting can change where a push goes.
/// Whether a `-c` or `--config-env` setting makes a push mirror its remote, which force-updates and
/// deletes its branches.
fn mirrors(setting: &str) -> bool {
    let key = setting.split('=').next().unwrap_or_default();
    key.to_ascii_lowercase().starts_with("remote.") && key.to_ascii_lowercase().ends_with(".mirror")
}

fn routes_push(setting: &str) -> bool {
    let key = setting
        .split('=')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    key.starts_with("push.")
        || key.starts_with("branch.")
        || (key.starts_with("remote.") && (key.ends_with(".push") || key.ends_with(".pushurl")))
        || key == "remote.pushdefault"
}

/// Whether `branch` is `main`, `master` or the branch `origin/HEAD` names.
fn is_default(git: Git, dir: Option<&str>, branch: &str) -> bool {
    if ["main", "master"].contains(&branch) {
        return true;
    }
    let origin = git(
        dir,
        &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"],
    );
    origin
        .as_deref()
        .and_then(|head| head.strip_prefix("origin/"))
        == Some(branch)
}

/// `curl` or `wget` that posts to Slack's chat or a webhook, or sends Linear a mutation. A body read from a
/// file or stdin counts as a mutation, since the guard cannot read it.
fn curl(words: &[String]) -> Option<&'static str> {
    let args = &words[1..];
    if args
        .iter()
        .any(|arg| arg.contains("hooks.slack.com") || arg.contains("slack.com/api/chat."))
    {
        return Some("slack-write");
    }
    if !args.iter().any(|arg| arg.contains("api.linear.app")) {
        return None;
    }
    const DATA: &[&str] = &[
        "-d",
        "--data",
        "--data-raw",
        "--data-binary",
        "--data-ascii",
        "--data-urlencode",
        "--json",
        "-F",
        "--form",
        "--post-data",
        "--body-data",
    ];
    let mut bodies: Vec<&str> = Vec::new();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        if DATA.contains(&arg.as_str()) {
            bodies.extend(args.next().map(String::as_str));
        } else if let Some((option, body)) = arg.split_once('=')
            && DATA.contains(&option)
        {
            bodies.push(body);
        } else if (arg.starts_with("-d") || arg.starts_with("-F")) && arg.len() > 2 {
            bodies.push(&arg[2..]);
        }
    }
    bodies
        .iter()
        .any(|body| body.starts_with('@') || body.contains("mutation"))
        .then_some("linear-write")
}

/// How deep `bash -c`, `eval` and substitutions may nest before the rest is not read.
const DEPTH: usize = 16;

/// The simple commands a shell script runs, each as its words, with wrappers removed and the
/// program named by its file name.
pub fn commands(script: &str) -> Vec<Vec<String>> {
    let mut found = read(script);
    found.retain(|words| !NOTES.contains(&words[0].as_str()));
    found
}

/// The commands of `script`, with the `NOTES` the splitter makes about them.
pub(crate) fn read(script: &str) -> Vec<Vec<String>> {
    let mut found = Vec::new();
    parse(script, 0, &mut found);
    found
}

/// The command the splitter records for text a script holds, such as a heredoc, a here-string or
/// an assigned value, which `SET` follows when it lasts past its command. No program has a note's
/// name.
pub(crate) const TEXT: &str = "\0text";
pub(crate) const SET: &str = "set";

/// A `cd` after this note may run in a shell of its own, or not at all.
pub(crate) const SCOPE: &str = "\0scope";

/// Words, and heredoc bodies, that hold a substitution whose output the reader cannot know.
pub(crate) const OPAQUE: &str = "\0opaque";

/// Words, and heredoc bodies, whose only expansions are variables.
pub(crate) const VARIABLE: &str = "\0variable";

/// The files a substitution prints with a plain `cat`.
pub(crate) const PRINTED: &str = "\0printed";

/// A file a redirection writes: empty for one the reader cannot name, with `LITERAL` after it when
/// the call holds the text.
pub(crate) const WRITES: &str = "\0writes";
pub(crate) const LITERAL: &str = "literal";

pub(crate) const NOTES: &[&str] = &[TEXT, SCOPE, OPAQUE, VARIABLE, PRINTED, WRITES];

/// The command the splitter records for a script nested past `DEPTH`: no program has this name.
const TOO_DEEP: &str = "\0too deeply nested for the guard to read";

/// The command the splitter records for an `env -S` string it cannot split: no program has this
/// name.
const UNSPLIT: &str = "\0an env -S string the guard cannot split";

fn parse(script: &str, depth: usize, found: &mut Vec<Vec<String>>) {
    if depth > DEPTH {
        return found.push(vec![TOO_DEEP.to_owned()]);
    }
    let mut split = Split {
        chars: script.chars().collect(),
        at: 0,
        depth,
        word: None,
        words: Vec::new(),
        heredocs: Vec::new(),
        redirect: None,
        stdin_shell: false,
        herestring: false,
        stdin: Vec::new(),
        expansion: None,
        expanding: Vec::new(),
        writes: Vec::new(),
        found,
    };
    split.run();
}

/// Reads the commands that `$(...)` and backticks in heredoc text run.
fn substitutions(text: &str, depth: usize, found: &mut Vec<Vec<String>>) {
    let mut text = Split {
        chars: text.chars().collect(),
        at: 0,
        depth,
        word: None,
        words: Vec::new(),
        heredocs: Vec::new(),
        redirect: None,
        stdin_shell: false,
        herestring: false,
        stdin: Vec::new(),
        expansion: None,
        expanding: Vec::new(),
        writes: Vec::new(),
        found,
    };
    while let Some(c) = text.peek(0) {
        text.at += 1;
        match c {
            '\\' => text.at += 1,
            '`' => {
                let inner = text.until_backtick();
                parse(&inner, depth + 1, text.found);
            }
            '$' if text.peek(0) == Some('(') => {
                text.at += 1;
                let inner = text.balanced();
                parse(&inner, depth + 1, text.found);
            }
            _ => {}
        }
    }
}

enum Redirect {
    Read,
    Write,
}

/// One pass over a script: words, the commands they form, and the scripts nested in it.
struct Split<'a> {
    chars: Vec<char>,
    at: usize,
    depth: usize,
    word: Option<String>,
    words: Vec<String>,
    /// Heredocs whose bodies start at the next newline: the delimiter, whether `<<-` strips tabs,
    /// and whether the delimiter was quoted, which keeps the body from expanding.
    heredocs: Vec<(String, bool, bool)>,
    /// The next word is a redirection's target, not an argument.
    redirect: Option<Redirect>,
    /// A command on this line is a shell reading its script from stdin, as from a heredoc.
    stdin_shell: bool,
    /// The next word is a here-string, which a shell would read as its script.
    herestring: bool,
    /// Here-strings and the text `echo` or `printf` pipes into this command: a shell's script.
    stdin: Vec<String>,
    /// The note the current word needs for an expansion it holds: `OPAQUE` or `VARIABLE`.
    expansion: Option<&'static str>,
    /// The current command's words that expand, each with its note.
    expanding: Vec<(&'static str, String)>,
    /// The files the current command's redirections write.
    writes: Vec<String>,
    found: &'a mut Vec<Vec<String>>,
}

impl Split<'_> {
    fn peek(&self, offset: usize) -> Option<char> {
        self.chars.get(self.at + offset).copied()
    }

    fn push(&mut self, c: char) {
        self.word.get_or_insert_default().push(c);
    }

    fn end_word(&mut self) {
        let expansion = self.expansion.take();
        if let Some(word) = self.word.take() {
            if let Some(redirect) = self.redirect.take() {
                if matches!(redirect, Redirect::Write) {
                    self.writes.push(expansion.map_or(word, |_| String::new()));
                }
                return;
            }
            self.expanding
                .extend(expansion.map(|note| (note, word.clone())));
            if std::mem::take(&mut self.herestring) {
                self.found.push(vec![TEXT.to_owned(), word.clone()]);
                self.stdin.push(word);
                return;
            }
            self.words.push(word);
        }
    }

    fn end_command(&mut self) {
        self.end_word();
        let words = std::mem::take(&mut self.words);
        self.note_writes(&words);
        for (note, word) in std::mem::take(&mut self.expanding) {
            self.found.push(vec![note.to_owned(), word]);
        }
        if words.is_empty() {
            return;
        }
        let lasts = words.iter().all(|word| assignment(word));
        for word in words.iter().take_while(|word| assignment(word)) {
            let mut text = vec![TEXT.to_owned(), word.clone()];
            text.extend(lasts.then(|| SET.to_owned()));
            self.found.push(text);
        }
        if !matches!(words[0].as_str(), "cd" | "pushd") {
            self.scope();
        }
        let before = self.found.len();
        unwrap(&words, self.depth, self.found);
        let shell = self.found[before..].iter().any(|command| {
            SHELLS.contains(&command[0].as_str())
                && command[1..].iter().all(|arg| arg.starts_with('-'))
        });
        self.stdin_shell |= shell;
        for script in std::mem::take(&mut self.stdin) {
            if shell {
                parse(&script, self.depth + 1, self.found);
            }
        }
    }

    /// Ends a command whose output a single `|` pipes into the next, keeping what `echo` or
    /// `printf` prints for it.
    fn end_piped(&mut self) {
        self.end_word();
        // The guard does not format: it reads the format and the arguments joined as scripts.
        let printed = match self.words.split_first() {
            Some((program, args)) if program.ends_with("echo") => {
                let options = args.iter().take_while(|arg| arg.starts_with('-')).count();
                vec![args[options..].join(" ")]
            }
            Some((program, [format, args @ ..])) if program.ends_with("printf") => {
                vec![format.clone(), args.join(" ")]
            }
            _ => Vec::new(),
        };
        self.end_command();
        self.stdin.extend(printed);
    }

    fn scope(&mut self) {
        self.found.push(vec![SCOPE.to_owned()]);
    }

    /// Notes the files the command `words` writes: the call holds what a literal `echo`, `printf`
    /// or `cat` of a heredoc prints.
    fn note_writes(&mut self, words: &[String]) {
        let literal = match words.first().map(String::as_str) {
            Some("echo" | "printf") => self.expanding.is_empty(),
            Some("cat") => words.len() == 1 && !self.heredocs.is_empty(),
            _ => false,
        };
        for path in std::mem::take(&mut self.writes) {
            let mut note = vec![WRITES.to_owned(), path];
            note.extend(literal.then(|| LITERAL.to_owned()));
            self.found.push(note);
        }
    }

    /// Notes that the current word expands, as the `$` just read makes it.
    fn expands(&mut self) {
        let name = |c: char| c.is_ascii_alphanumeric() || "_{@*?!#$-[".contains(c);
        if self.peek(0).is_some_and(name) {
            self.expansion.get_or_insert(VARIABLE);
        }
    }

    fn nested(&mut self, script: &str) {
        let before = self.found.len();
        self.scope();
        parse(script, self.depth + 1, self.found);
        match printed(&self.found[before..]) {
            Some(files) => self
                .found
                .push(std::iter::once(PRINTED.to_owned()).chain(files).collect()),
            None => self.expansion = Some(OPAQUE),
        }
        // The substitution's output is unknown, but it is part of a word.
        self.word.get_or_insert_default();
    }

    fn run(&mut self) {
        while let Some(c) = self.peek(0) {
            self.at += 1;
            match c {
                '\n' => {
                    self.end_command();
                    self.skip_heredocs();
                }
                ' ' | '\t' | '\r' => self.end_word(),
                // `2>` or `10<`: the number is the redirection's, not an argument.
                '<' | '>'
                    if self.word.as_deref().is_some_and(|word| {
                        !word.is_empty() && word.chars().all(|c| c.is_ascii_digit())
                    }) =>
                {
                    self.word = None;
                    self.redirection(c);
                }
                '#' if self.word.is_none() => {
                    while self.peek(0).is_some_and(|c| c != '\n') {
                        self.at += 1;
                    }
                }
                '|' => {
                    // Before the command it ends, which may be a `cd` in the pipe's own shell.
                    self.scope();
                    let previous = self.at.checked_sub(2).and_then(|at| self.chars.get(at));
                    if self.peek(0) == Some('|') || previous == Some(&'|') {
                        self.end_command();
                    } else {
                        self.end_piped();
                    }
                }
                '&' => {
                    let previous = self.at.checked_sub(2).and_then(|at| self.chars.get(at));
                    if self.peek(0) != Some('&') && previous != Some(&'&') {
                        self.scope();
                    }
                    self.end_command();
                }
                ';' => self.end_command(),
                '(' | ')' => {
                    self.end_command();
                    self.scope();
                }
                '<' | '>' => self.redirection(c),
                '\'' => {
                    let start = self.at;
                    while self.peek(0).is_some_and(|c| c != '\'') {
                        self.at += 1;
                    }
                    let quoted: String = self.chars[start..self.at].iter().collect();
                    self.word.get_or_insert_default().push_str(&quoted);
                    self.at += 1;
                }
                '"' => self.double_quoted(),
                '\\' => match self.peek(0) {
                    Some('\n') => self.at += 1,
                    Some(next) => {
                        self.push(next);
                        self.at += 1;
                    }
                    None => {}
                },
                '`' => {
                    let inner = self.until_backtick();
                    self.nested(&inner);
                }
                '$' if self.peek(0) == Some('(') => {
                    self.at += 1;
                    let inner = self.balanced();
                    self.nested(&inner);
                }
                '$' if self.peek(0) == Some('\'') => {
                    self.at += 1;
                    self.ansi_c_quoted();
                }
                '$' => {
                    self.expands();
                    self.push('$');
                }
                c => self.push(c),
            }
        }
        self.end_command();
    }

    /// The rest of a `$'...'` word, with its backslash escapes decoded.
    fn ansi_c_quoted(&mut self) {
        let word = self.word.get_or_insert_default();
        let start = word.len();
        while let Some(c) = self.peek(0) {
            self.at += 1;
            match c {
                '\'' => {
                    // Bash ends the quote's text at a NUL byte.
                    let word = self.word.get_or_insert_default();
                    if let Some(nul) = word[start..].find('\0') {
                        word.truncate(start + nul);
                    }
                    return;
                }
                '\\' => {
                    let Some(next) = self.peek(0) else {
                        return;
                    };
                    self.at += 1;
                    let (radix, digits) = match next {
                        'x' => (16, 2),
                        '0'..='7' => {
                            self.at -= 1;
                            (8, 3)
                        }
                        _ => (0, 0),
                    };
                    if radix != 0 {
                        let start = self.at;
                        while self.at - start < digits
                            && self.peek(0).is_some_and(|c| c.is_digit(radix))
                        {
                            self.at += 1;
                        }
                        let code: String = self.chars[start..self.at].iter().collect();
                        // Bash keeps the low byte of a value past 255, as `\547` is `g`.
                        if let Ok(value) = u32::from_str_radix(&code, radix) {
                            self.push(char::from(value as u8));
                        }
                        continue;
                    }
                    self.push(match next {
                        'n' => '\n',
                        't' => '\t',
                        'r' => '\r',
                        next => next,
                    });
                }
                c => self.push(c),
            }
        }
    }

    fn redirection(&mut self, c: char) {
        self.end_word();
        if self.peek(0) == Some('(') {
            self.at += 1;
            let inner = self.balanced();
            self.nested(&inner);
            return;
        }
        if c == '<' && self.peek(0) == Some('<') && self.peek(1) != Some('<') {
            self.at += 1;
            let heredoc = self.heredoc();
            self.heredocs.push(heredoc);
            return;
        }
        if c == '<' && self.peek(0) == Some('<') {
            self.at += 2;
            self.herestring = true;
            return;
        }
        let mut writes = c == '>';
        while let Some(next) = self.peek(0).filter(|c| "<>&|".contains(*c)) {
            writes |= next == '>';
            self.at += 1;
        }
        if self.chars[self.at - 1] == '&' {
            while self.peek(0).is_some_and(|c| c.is_ascii_digit() || c == '-') {
                self.at += 1;
            }
            return;
        }
        self.redirect = Some(if writes {
            Redirect::Write
        } else {
            Redirect::Read
        });
    }

    /// Reads the heredoc bodies that start here: as a script when a shell on the line reads its
    /// stdin, else as text whose substitutions run unless the delimiter was quoted.
    fn skip_heredocs(&mut self) {
        let script = std::mem::take(&mut self.stdin_shell);
        for (delimiter, strip, quoted) in std::mem::take(&mut self.heredocs) {
            let body = self.heredoc_body(&delimiter, strip);
            self.found.push(vec![TEXT.to_owned(), body.clone()]);
            if script {
                parse(&body, self.depth + 1, self.found);
            } else if !quoted {
                if body_expands(&body) {
                    self.found.push(vec![OPAQUE.to_owned(), body.clone()]);
                }
                substitutions(&body, self.depth, self.found);
            }
        }
    }

    /// Reads a heredoc's operator after `<<`: its delimiter, whether `<<-` strips tabs, and whether
    /// the delimiter was quoted.
    fn heredoc(&mut self) -> (String, bool, bool) {
        let strip = self.peek(0) == Some('-');
        if strip {
            self.at += 1;
        }
        while self.peek(0).is_some_and(|c| c == ' ' || c == '\t') {
            self.at += 1;
        }
        let mut delimiter = String::new();
        let mut quoted = false;
        while let Some(c) = self.peek(0).filter(|c| !" \t\n;&|()<>".contains(*c)) {
            self.at += 1;
            match c {
                '\'' | '"' => {
                    quoted = true;
                    while let Some(inner) = self.peek(0) {
                        self.at += 1;
                        if inner == c {
                            break;
                        }
                        delimiter.push(inner);
                    }
                }
                '\\' => {
                    quoted = true;
                    if let Some(escaped) = self.peek(0) {
                        delimiter.push(escaped);
                        self.at += 1;
                    }
                }
                c => delimiter.push(c),
            }
        }
        (delimiter, strip, quoted)
    }

    /// Reads a heredoc's body lines up to and past its delimiter line.
    fn heredoc_body(&mut self, delimiter: &str, strip: bool) -> String {
        let mut body = String::new();
        while self.at < self.chars.len() {
            let end = self.chars[self.at..]
                .iter()
                .position(|&c| c == '\n')
                .map_or(self.chars.len(), |end| self.at + end);
            let line: String = self.chars[self.at..end].iter().collect();
            self.at = (end + 1).min(self.chars.len());
            let line = if strip {
                line.trim_start_matches('\t')
            } else {
                &line
            };
            if line == delimiter {
                break;
            }
            body.push_str(line);
            body.push('\n');
        }
        body
    }

    fn double_quoted(&mut self) {
        self.word.get_or_insert_default();
        while let Some(c) = self.peek(0) {
            self.at += 1;
            match c {
                '"' => return,
                '\\' => {
                    if let Some(next) = self.peek(0) {
                        if !"\"\\$`\n".contains(next) {
                            self.push('\\');
                        }
                        if next != '\n' {
                            self.push(next);
                        }
                        self.at += 1;
                    }
                }
                '`' => {
                    let inner = self.until_backtick();
                    self.nested(&inner);
                }
                '$' if self.peek(0) == Some('(') => {
                    self.at += 1;
                    let inner = self.balanced();
                    self.nested(&inner);
                }
                '$' => {
                    self.expands();
                    self.push('$');
                }
                c => self.push(c),
            }
        }
    }

    fn until_backtick(&mut self) -> String {
        let mut inner = String::new();
        while let Some(c) = self.peek(0) {
            self.at += 1;
            match c {
                '`' => break,
                '\\' if self.peek(0).is_some() => {
                    inner.push(self.chars[self.at]);
                    self.at += 1;
                }
                c => inner.push(c),
            }
        }
        inner
    }

    /// The text up to the parenthesis that closes one just read, skipping quoted parentheses and
    /// heredoc bodies.
    fn balanced(&mut self) -> String {
        let start = self.at;
        let mut depth = 1;
        let mut quote = None;
        let mut heredocs = Vec::new();
        while let Some(c) = self.peek(0) {
            self.at += 1;
            match (quote, c) {
                (Some(q), c) if c == q => quote = None,
                (Some('"'), '\\') | (None, '\\') => self.at += 1,
                (Some(_), _) => {}
                (None, '<') if self.peek(0) == Some('<') && self.peek(1) == Some('<') => {
                    self.at += 2
                }
                (None, '<') if self.peek(0) == Some('<') => {
                    self.at += 1;
                    heredocs.push(self.heredoc());
                }
                (None, '\n') => {
                    for (delimiter, strip, _) in std::mem::take(&mut heredocs) {
                        self.heredoc_body(&delimiter, strip);
                    }
                }
                (None, '\'' | '"') => quote = Some(c),
                (None, '(') => depth += 1,
                (None, ')') => {
                    depth -= 1;
                    if depth == 0 {
                        return self.chars[start..self.at - 1].iter().collect();
                    }
                }
                _ => {}
            }
        }
        self.at = self.at.min(self.chars.len());
        self.chars[start..].iter().collect()
    }
}

/// Whether an unquoted heredoc's `body` holds an expansion.
fn body_expands(body: &str) -> bool {
    let mut chars = body.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => drop(chars.next()),
            '`' => return true,
            '$' if chars
                .peek()
                .is_some_and(|c| c.is_ascii_alphanumeric() || "_({@*?!#$-[".contains(*c)) =>
            {
                return true;
            }
            _ => {}
        }
    }
    false
}

/// The files a substitution prints when its commands, `found`, are one `cat` of literal files or of
/// a heredoc whose text the script holds.
fn printed(found: &[Vec<String>]) -> Option<Vec<String>> {
    let text = found.iter().any(|words| words[0] == TEXT);
    let mut rest = found
        .iter()
        .filter(|words| words[0] != SCOPE && words[0] != TEXT);
    let (Some(cat), None) = (rest.next(), rest.next()) else {
        return None;
    };
    let files = &cat[1..];
    let literal = files.iter().all(|file| !file.starts_with('-'));
    (cat[0] == "cat" && literal && (text || !files.is_empty())).then(|| files.to_vec())
}

/// Words that start a command without being its program.
const KEYWORDS: &[&str] = &[
    "!", "{", "}", "if", "then", "else", "elif", "do", "while", "until", "builtin", "coproc", "fi",
    "done",
];

/// Shells whose `-c` script, or stdin, the guard reads.
const SHELLS: &[&str] = &["sh", "bash", "zsh", "dash", "ksh"];

/// Strips the assignments, keywords and wrappers in front of `words`' program, then adds the
/// command, or the commands of the script a shell or `eval` runs.
fn unwrap(words: &[String], depth: usize, found: &mut Vec<Vec<String>>) {
    if depth > DEPTH {
        return found.push(vec![TOO_DEEP.to_owned()]);
    }
    let mut at = 0;
    while let Some(word) = words.get(at) {
        if assignment(word) {
            // `end_command` notes the ones that come first.
            if !words[..at].iter().all(|word| assignment(word)) {
                found.push(vec![TEXT.to_owned(), word.clone()]);
            }
            at += 1;
            continue;
        }
        if KEYWORDS.contains(&word.as_str()) {
            at += 1;
            continue;
        }
        let program = word.rsplit('/').next().unwrap_or(word);
        let rest = &words[at + 1..];
        if let Some(list) = word
            .strip_prefix('{')
            .and_then(|word| word.strip_suffix('}'))
            && list.contains(',')
        {
            let expanded: Vec<String> = list
                .split(',')
                .map(str::to_owned)
                .chain(rest.iter().cloned())
                .collect();
            return unwrap(&expanded, depth + 1, found);
        }
        let skip = |values: &[&str]| options(rest, values);
        match program {
            "function" => at += 2,
            "find" => {
                let mut args = rest.iter();
                while args
                    .find(|arg| ["-exec", "-execdir", "-ok", "-okdir"].contains(&arg.as_str()))
                    .is_some()
                {
                    let command: Vec<String> = args
                        .by_ref()
                        .take_while(|arg| *arg != ";" && *arg != "+")
                        .cloned()
                        .collect();
                    unwrap(&command, depth + 1, found);
                }
                break;
            }
            "env" => {
                let mut index = 0;
                while let Some(arg) = rest.get(index) {
                    let (value, after) = if arg == "-S" || arg == "--split-string" {
                        (rest.get(index + 1).map(String::as_str), index + 2)
                    } else if let Some(value) = arg
                        .strip_prefix("--split-string=")
                        .or_else(|| arg.strip_prefix("-S").filter(|value| !value.is_empty()))
                    {
                        (Some(value), index + 1)
                    } else {
                        (None, 0)
                    };
                    if after > 0 {
                        let Some(split) = env_split(value.unwrap_or_default()) else {
                            return found.push(vec![UNSPLIT.to_owned()]);
                        };
                        // env reads the split words as its own arguments, options included.
                        let words: Vec<String> = std::iter::once("env".to_owned())
                            .chain(split)
                            .chain(rest.iter().skip(after).cloned())
                            .collect();
                        return unwrap(&words, depth + 1, found);
                    }
                    let dir = match arg.as_str() {
                        "-C" | "--chdir" => rest.get(index + 1).map(String::as_str),
                        _ => arg
                            .strip_prefix("--chdir=")
                            .or_else(|| arg.strip_prefix("-C").filter(|dir| !dir.is_empty())),
                    };
                    if let Some(dir) = dir {
                        found.push(vec![SCOPE.to_owned()]);
                        found.push(vec!["cd".to_owned(), dir.to_owned()]);
                    }
                    if ["-u", "-C", "--unset", "--chdir"].contains(&arg.as_str()) {
                        index += 2;
                    } else if assignment(arg) {
                        found.push(vec![TEXT.to_owned(), arg.clone()]);
                        index += 1;
                    } else if arg.starts_with('-') {
                        index += 1;
                    } else {
                        break;
                    }
                }
                at += 1 + index;
            }
            "sudo" | "doas" => {
                at += 1 + skip(&[
                    "-u",
                    "-g",
                    "-h",
                    "-p",
                    "-C",
                    "-D",
                    "-r",
                    "-t",
                    "-U",
                    "-T",
                    "--user",
                    "--group",
                    "--host",
                    "--prompt",
                    "--close-from",
                    "--chdir",
                    "--role",
                    "--type",
                    "--other-user",
                    "--command-timeout",
                ])
            }
            "command" | "time" => at += 1 + skip(&[]),
            "exec" => at += 1 + skip(&["-a"]),
            "nice" => at += 1 + skip(&["-n"]),
            "stdbuf" | "nohup" | "setsid" => at += 1 + skip(&[]),
            "xargs" => at += 1 + skip(&["-I", "-n", "-P", "-L", "-d", "-E", "-s", "-a"]),
            "timeout" => at += 2 + skip(&["-s", "-k", "--signal", "--kill-after"]),
            _ if SHELLS.contains(&program) => {
                let flag = rest.iter().position(|arg| {
                    arg.starts_with('-') && !arg.starts_with("--") && arg.contains('c')
                });
                match flag {
                    Some(flag) => {
                        if let Some(script) =
                            rest[flag + 1..].iter().find(|arg| !arg.starts_with('-'))
                        {
                            parse(script, depth + 1, found);
                        }
                        return;
                    }
                    None => break,
                }
            }
            "eval" => return parse(&rest.join(" "), depth + 1, found),
            _ => break,
        }
    }
    if let Some(word) = words.get(at) {
        let mut command = vec![word.rsplit('/').next().unwrap_or(word).to_owned()];
        command.extend_from_slice(&words[at + 1..]);
        found.push(command);
    }
}

/// How many leading words of `args` are options, counting the values of those in `values`.
/// What a backslash and the character after it make in an `env -S` string.
enum Escape {
    Char(char),
    Separator,
    Stop,
}

fn escape(c: char) -> Option<Escape> {
    Some(match c {
        '\\' | '"' | '\'' | '#' | '$' => Escape::Char(c),
        'f' => Escape::Char('\x0c'),
        'n' => Escape::Char('\n'),
        'r' => Escape::Char('\r'),
        't' => Escape::Char('\t'),
        'v' => Escape::Char('\x0b'),
        '_' => Escape::Separator,
        'c' => Escape::Stop,
        _ => return None,
    })
}

/// `text` split into words as GNU `env -S` splits it, or None where it expands a variable, whose
/// value the guard cannot know, or where env refuses it.
fn env_split(text: &str) -> Option<Vec<String>> {
    let mut words = Vec::new();
    let mut word: Option<String> = None;
    let mut quote = None;
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match quote {
            Some('\'') => match c {
                '\'' => quote = None,
                '\\' => {
                    let next = chars.next()?;
                    let word = word.get_or_insert_default();
                    if next != '\\' && next != '\'' {
                        word.push('\\');
                    }
                    word.push(next);
                }
                _ => word.get_or_insert_default().push(c),
            },
            Some(_) => match c {
                '"' => quote = None,
                '\\' => match escape(chars.next()?)? {
                    Escape::Char(c) => word.get_or_insert_default().push(c),
                    Escape::Separator => word.get_or_insert_default().push(' '),
                    Escape::Stop => return None,
                },
                '$' => return None,
                _ => word.get_or_insert_default().push(c),
            },
            None => match c {
                ' ' | '\t' | '\n' | '\x0b' | '\x0c' | '\r' => words.extend(word.take()),
                '#' if word.is_none() => break,
                '\'' | '"' => {
                    quote = Some(c);
                    word.get_or_insert_default();
                }
                '\\' => match escape(chars.next()?)? {
                    Escape::Char(c) => word.get_or_insert_default().push(c),
                    Escape::Separator => words.extend(word.take()),
                    Escape::Stop => break,
                },
                '$' => return None,
                _ => word.get_or_insert_default().push(c),
            },
        }
    }
    if quote.is_some() {
        return None;
    }
    words.extend(word);
    Some(words)
}

fn options(args: &[String], values: &[&str]) -> usize {
    let mut index = 0;
    while let Some(arg) = args.get(index) {
        if arg == "--" {
            return index + 1;
        }
        if !arg.starts_with('-') {
            break;
        }
        index += if values.contains(&arg.as_str()) { 2 } else { 1 };
    }
    index
}

/// Whether `word` is `NAME=value`.
fn assignment(word: &str) -> bool {
    word.split_once('=').is_some_and(|(name, _)| {
        !name.is_empty()
            && !name.starts_with(|c: char| c.is_ascii_digit())
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn news_holds_a_pull_request_a_push_and_a_step_end_and_not_a_read() {
        let held = |command: &str| holds_for_news("Bash", Some(command));
        assert!(held("gh pr create --title T --body B"));
        assert!(held("bash -c 'gh -R o/r pr create -f'"));
        assert!(held("git push -u origin priory/7"));
        assert!(held("git -C repo push"));
        assert!(!held("git push --dry-run origin priory/7"));
        assert!(!held("git push -n origin priory/7"));
        assert!(held("git push --dry-run --no-dry-run origin priory/7"));
        assert!(!held("gh pr create --help"));
        assert!(!held("gh pr view 7"));
        assert!(!held("git status"));
        assert!(holds_for_news("mcp__priory__result", None));
        assert!(holds_for_news("mcp__github__create_pull_request", None));
        assert!(!holds_for_news("mcp__priory__progress", None));
        assert!(!holds_for_news("Read", None));
    }

    fn guards(command: &str) -> Vec<String> {
        let git = |_: Option<&str>, args: &[&str]| match args {
            ["symbolic-ref", "--short", "HEAD"] => Some("priory/1".to_owned()),
            _ => None,
        };
        Guards::default()
            .hits("Bash", Some(command), &git)
            .into_iter()
            .map(|hit| hit.guard)
            .collect()
    }

    fn hit(command: &str) -> Option<String> {
        guards(command).into_iter().next()
    }

    #[test]
    fn a_heredoc_body_is_text_not_commands() {
        let create = "gh pr create --title T --body \"$(cat <<'EOF'\nThen run:\ngit push origin main\nEOF\n)\"";

        assert_eq!(hit(create), None);
    }

    #[test]
    fn a_substitution_inside_double_quotes_is_read() {
        assert_eq!(
            hit(r#"echo "done: $(gh pr merge 12)""#).as_deref(),
            Some("gh-pr-merge")
        );
        assert_eq!(hit("echo `gh pr merge 12`").as_deref(), Some("gh-pr-merge"));
    }

    #[test]
    fn wrappers_in_front_of_the_program_are_removed() {
        for command in [
            "sudo -u me gh pr merge 12",
            "timeout -s KILL 30 gh pr merge 12",
            "echo 12 | xargs -n 1 gh pr merge",
            "cd repo && /usr/bin/gh pr merge 12",
            "nice -n 5 nohup gh pr merge 12 > out.log 2>&1",
            "eval 'gh pr merge 12'",
            "sh -ec \"gh pr merge 12\"",
            "env -S 'gh pr merge 12'",
            "env --split-string=gh pr merge 12",
            "env --split-string='gh pr' merge 12",
            "env -Sgh pr merge 12",
            r"env -S 'gh\_pr\_merge\_12'",
            r"env --split-string='gh\_pr' merge 12",
            r#"env -S 'g"h" pr merge 12'"#,
            r"env -S '-i gh pr merge\_12'",
            r"env -S 'FOO=1 gh pr merge 12 \c # rest'",
            r"env -S 'env -S gh\\_pr\\_merge\\_12'",
            "if true; then gh pr merge 12; fi",
        ] {
            assert_eq!(hit(command).as_deref(), Some("gh-pr-merge"), "{command}");
        }
    }

    #[test]
    fn a_quoted_mention_and_a_search_term_are_not_calls() {
        for command in [
            "echo 'gh pr merge 12'",
            "gh pr list --search merge",
            "gh pr view 12 --json mergeable",
            "grep -n 'git push origin main' docs/x.md",
        ] {
            assert_eq!(hit(command), None, "{command}");
        }
    }

    #[test]
    fn a_program_named_by_a_variable_is_a_known_hole() {
        assert_eq!(hit("x=gh; $x pr merge 12"), None);
    }

    #[test]
    fn gh_api_writes_only_with_a_method_or_a_body() {
        assert_eq!(hit("gh api repos/o/r/pulls/12"), None);
        assert_eq!(hit("gh api -X GET repos/o/r/pulls -f state=open"), None);
        assert_eq!(
            hit("gh api -X PUT repos/o/r/pulls/12/merge").as_deref(),
            Some("gh-pr-merge")
        );
        assert_eq!(
            hit("gh api repos/o/r/issues/1/comments -f body=hi").as_deref(),
            Some("gh-api-write")
        );
    }

    #[test]
    fn git_push_is_guarded_on_the_default_branch_force_and_delete() {
        assert_eq!(hit("git push origin priory/1"), None);
        assert_eq!(hit("git push -u origin HEAD"), None);
        assert_eq!(
            hit("git push origin HEAD:main").as_deref(),
            Some("git-push-default")
        );
        assert_eq!(
            hit("git -C ../x push origin master").as_deref(),
            Some("git-push-default")
        );
        assert_eq!(
            hit("git push -fu origin priory/1").as_deref(),
            Some("git-push-force")
        );
        assert_eq!(
            hit("git push --force-with-lease").as_deref(),
            Some("git-push-force")
        );
        assert_eq!(
            hit("git push origin +priory/1").as_deref(),
            Some("git-push-force")
        );
        assert_eq!(
            hit("git push origin :old").as_deref(),
            Some("git-push-force")
        );
    }

    #[test]
    fn curl_to_slack_chat_or_a_linear_mutation_writes() {
        assert_eq!(
            hit("curl -X POST https://slack.com/api/chat.postMessage -d text=hi").as_deref(),
            Some("slack-write")
        );
        assert_eq!(
            hit("curl https://slack.com/api/conversations.history"),
            None
        );
        assert_eq!(
            hit(r#"curl https://api.linear.app/graphql --json '{"query":"mutation { x }"}'"#)
                .as_deref(),
            Some("linear-write")
        );
        assert_eq!(
            hit(r#"curl https://api.linear.app/graphql -d '{"query":"{ viewer { id } }"}'"#),
            None
        );
        assert_eq!(
            hit("curl https://api.linear.app/graphql -d @body.json").as_deref(),
            Some("linear-write")
        );
    }

    #[test]
    fn a_factory_guard_matches_its_command_and_its_flags() {
        let guards = Guards {
            commands: vec![CommandGuard {
                name: "deploy-prod".into(),
                run: vec!["fly".into(), "deploy".into()],
                flags: vec!["--prod".into()],
            }],
            tools: vec!["mcp__stripe__*".into()],
        };
        let git = |_: Option<&str>, _: &[&str]| None;
        let guard = |tool: &str, command: Option<&str>| {
            guards.hits(tool, command, &git).pop().map(|hit| hit.guard)
        };

        assert_eq!(
            guard("Bash", Some("fly deploy --prod")).as_deref(),
            Some("deploy-prod")
        );
        assert_eq!(guard("Bash", Some("fly deploy")), None);
        assert_eq!(
            guard("mcp__stripe__refund", None).as_deref(),
            Some("mcp__stripe__*")
        );
        assert_eq!(guard("mcp__priory__send_message", None), None);
        assert_eq!(guard("mcp__github__get_pull_request", None), None);
    }

    #[test]
    fn gh_api_names_a_merge_or_an_approval_by_its_endpoint_and_operation() {
        let cases = [
            (
                "gh api -X PUT /repos/o/r/pulls/12/merge?x=1",
                Some("gh-pr-merge"),
            ),
            ("gh api repos/o/r/pulls/12/merge", None),
            (
                "gh api repos/o/r/pulls/12/reviews -f event=APPROVE",
                Some("gh-pr-approve"),
            ),
            (
                "gh api repos/o/r/pulls/12/reviews/9/events -F event=APPROVE",
                Some("gh-pr-approve"),
            ),
            (
                "gh api repos/o/r/pulls/12/reviews --input review.json",
                Some("gh-pr-approve"),
            ),
            (
                "gh api repos/o/r/pulls/12/reviews -f event=COMMENT -f body=ok",
                Some("gh-api-write"),
            ),
            (
                "gh api /graphql -f query='mutation { submitPullRequestReview(input: {event: APPROVE}) { clientMutationId } }'",
                Some("gh-pr-approve"),
            ),
            (
                "gh api graphql -f query='mutation { enablePullRequestAutoMerge(input: {}) { clientMutationId } }'",
                Some("gh-pr-merge"),
            ),
            ("gh api graphql --input merge.json", Some("gh-pr-merge")),
            ("gh api --input merge.json graphql", Some("gh-pr-merge")),
            (
                "gh api --input review.json repos/o/r/pulls/12/reviews",
                Some("gh-pr-approve"),
            ),
            (
                "gh api graphql -F query=@approve.graphql",
                Some("gh-pr-merge"),
            ),
            (
                "gh api graphql -f query='mutation($e: PullRequestReviewEvent!) { submitPullRequestReview(input: {event: $e}) { clientMutationId } }' -f e=APPROVE",
                Some("gh-pr-approve"),
            ),
            (
                "gh api graphql -f query='mutation($b: String!) { addComment(input: {body: $b}) { clientMutationId } }' -f b='Use mergePullRequest, PullRequestReview APPROVE'",
                Some("gh-api-write"),
            ),
            (
                "gh api graphql -f query='{ search(query: \"review:approved\") { issueCount } }' -f states=APPROVED",
                Some("gh-api-write"),
            ),
            (
                "gh api repos/o/r/issues/1/comments -f body='Use mergePullRequest with event APPROVE'",
                Some("gh-api-write"),
            ),
        ];
        for (command, guard) in cases {
            assert_eq!(hit(command).as_deref(), guard, "{command}");
        }
    }

    #[test]
    fn a_merge_or_an_approving_review_keeps_its_guard_however_it_is_made() {
        let guards = Guards {
            commands: vec![
                CommandGuard {
                    name: "github-write".into(),
                    run: vec!["gh".into(), "pr".into()],
                    flags: Vec::new(),
                },
                CommandGuard {
                    name: "github-api".into(),
                    run: vec!["gh".into(), "api".into()],
                    flags: Vec::new(),
                },
            ],
            tools: vec!["mcp__github__*".into()],
        };
        let git = |_: Option<&str>, _: &[&str]| None;
        let guard = |tool: &str, command: Option<&str>| {
            guards.hits(tool, command, &git).pop().map(|hit| hit.guard)
        };
        let bash = |command: &str| guard("Bash", Some(command));

        assert_eq!(bash("gh pr merge 12").as_deref(), Some("gh-pr-merge"));
        assert_eq!(
            bash("gh pr review 12 --approve").as_deref(),
            Some("gh-pr-approve")
        );
        assert_eq!(bash("gh pr comment 12").as_deref(), Some("github-write"));
        assert_eq!(
            bash("gh api -X PUT repos/o/r/pulls/12/merge").as_deref(),
            Some("gh-pr-merge")
        );
        assert_eq!(
            bash("gh api repos/o/r/pulls/12/reviews -f event=APPROVE").as_deref(),
            Some("gh-pr-approve")
        );
        assert_eq!(
            bash("gh api graphql -f query='mutation { mergePullRequest(input: {}) { clientMutationId } }'")
                .as_deref(),
            Some("gh-pr-merge")
        );
        assert_eq!(
            bash("gh api repos/o/r/issues/1/comments -f body=hi").as_deref(),
            Some("github-api")
        );
        assert_eq!(
            guard("mcp__github__merge_pull_request", None).as_deref(),
            Some("gh-pr-merge")
        );
        assert_eq!(
            guard("mcp__github__add_issue_comment", None).as_deref(),
            Some("mcp__github__*")
        );
        assert_eq!(
            Guards::default()
                .hits("mcp__github__merge_pull_request", None, &git)
                .pop()
                .map(|hit| hit.guard)
                .as_deref(),
            Some("gh-pr-merge")
        );
    }

    #[test]
    fn every_guarded_command_of_a_compound_call_is_a_hit() {
        assert_eq!(
            guards("gh pr merge 12 && git push --force origin feature"),
            ["gh-pr-merge", "git-push-force"]
        );
    }

    #[test]
    fn a_call_that_commits_moves_the_head_and_a_merge_alone_does_not() {
        assert!(moves_head("git commit -am fix && gh pr merge 12"));
        assert!(moves_head("git -C repo pull && gh pr merge 12"));
        assert!(!moves_head("gh pr merge 12"));
        assert!(!moves_head("git status && gh pr merge 12"));
    }

    #[test]
    fn a_descriptor_number_before_a_redirection_is_not_an_argument() {
        assert_eq!(
            hit("2>/dev/null gh pr merge 12").as_deref(),
            Some("gh-pr-merge")
        );
        assert_eq!(
            hit("gh 2>/dev/null pr merge 12").as_deref(),
            Some("gh-pr-merge")
        );
        assert_eq!(
            hit("gh pr merge 12 2>&1 | tee log").as_deref(),
            Some("gh-pr-merge")
        );
    }

    #[test]
    fn wrapper_options_and_ansi_c_quotes_are_read() {
        for command in [
            "command -- gh pr merge 12",
            "exec -- gh pr merge 12",
            "time -p gh pr merge 12",
            "sudo --user root gh pr merge 12",
            "bash -c $'gh pr merge 12'",
            "bash -c $'echo hi\\ngh pr merge 12'",
        ] {
            assert_eq!(hit(command).as_deref(), Some("gh-pr-merge"), "{command}");
        }
    }

    #[test]
    fn a_heredoc_runs_its_substitutions_and_a_shell_runs_its_body() {
        for command in [
            "cat <<EOF\n$(gh pr merge 12)\nEOF",
            "bash <<'EOF'\ngh pr merge 12\nEOF",
            "cat <<'EOF' | sh\ngh pr merge 12\nEOF",
        ] {
            assert_eq!(hit(command).as_deref(), Some("gh-pr-merge"), "{command}");
        }
        assert_eq!(hit("cat <<'EOF'\n$(gh pr merge 12)\nEOF"), None);
    }

    #[test]
    fn a_push_naming_its_remote_by_option_or_pushing_every_branch_is_read() {
        assert_eq!(
            hit("git push --repo=origin main").as_deref(),
            Some("git-push-default")
        );
        assert_eq!(
            hit("git push --repo origin main").as_deref(),
            Some("git-push-default")
        );
        assert_eq!(
            hit("git push --all origin").as_deref(),
            Some("git-push-default")
        );
        assert_eq!(
            hit("git push origin 'refs/heads/*'").as_deref(),
            Some("git-push-default")
        );
    }

    #[test]
    fn help_a_dry_run_and_an_attached_push_option_are_not_denied() {
        for command in [
            "gh pr merge --help",
            "git push --dry-run origin main",
            "git push -n origin main",
            "git push -ofoo origin feature",
            "git push -o ci.skip origin feature",
        ] {
            assert_eq!(hit(command), None, "{command}");
        }
    }

    #[test]
    fn help_given_as_an_option_value_does_not_exempt_the_command() {
        for (command, guard) in [
            ("gh pr merge 7 --squash --body '--help'", "gh-pr-merge"),
            ("gh pr review 7 --approve --body '--help'", "gh-pr-approve"),
            ("gh pr merge 12 -t -h", "gh-pr-merge"),
            ("gh pr merge 12 -- --help", "gh-pr-merge"),
            ("git push origin HEAD:main -o --help", "git-push-default"),
            ("git push origin HEAD:main -o -h", "git-push-default"),
        ] {
            assert_eq!(hit(command).as_deref(), Some(guard), "{command}");
        }
        for command in [
            "gh pr merge 7 --help",
            "gh pr merge -h",
            "gh pr review --body x --help",
            "git push --help",
        ] {
            assert_eq!(hit(command), None, "{command}");
        }
    }

    #[test]
    fn priory_person_verbs_from_an_agent_shell_are_guarded_and_keep_their_name() {
        let guards = Guards {
            commands: vec![CommandGuard {
                name: "priory-any".into(),
                run: vec!["priory".into()],
                flags: Vec::new(),
            }],
            tools: Vec::new(),
        };
        let git = |_: Option<&str>, _: &[&str]| None;
        for command in [
            "priory answer 12 approve --revision 1",
            "priory answer --run -- 12 approve",
            "env -u PRIORY_SESSION priory dismiss 12",
            "/usr/local/bin/priory release 7",
            "priory resume 7",
            "priory revive 7 --step merge",
            "priory cancel 7",
            "priory hold 7 --reason x",
            "priory move 7 --top",
            "priory say 7 merge it",
            "priory report pick 3 skip --expected 2",
            "priory subscribe 7 github:o/r#12",
            "priory snooze ask 12 --for 1h",
            "priory pause",
            "priory give github:o/r#12 b",
            "priory eval prompts/review.md",
            "priory label 7 wrong --evidence https://example.com/pull/9",
            "priory upgrade resume",
            "priory upgrade retry 1.4.0",
            "priory memories accept 3",
            "priory memories drop ~/factory 3",
            "priory score --budget 5 --calibrate review",
            "priory score . --calibrate=review --budget 5",
            "priory batch edit 2 --expected 1 --drop 4",
            "env -u PRIORY_SESSION priory mcp --outside",
        ] {
            let hits = guards.hits("Bash", Some(command), &git);
            assert_eq!(hits[0].guard, "priory-person", "{command}");
        }
        for command in [
            "priory report put 7 -",
            "priory tasks",
            "priory report reply 3 ok",
            "priory gate",
            "priory memories --all",
            "priory memories",
            "priory memories --json",
            "priory score 7 --budget 1",
            "priory batch show 2",
            "priory batch add 12 13",
            "priory mcp --session 3f2a",
        ] {
            assert_eq!(hit(command), None, "{command}");
        }
    }

    #[test]
    fn a_push_goes_where_git_config_sends_it() {
        let git = |_: Option<&str>, args: &[&str]| match args {
            ["symbolic-ref", "--short", "HEAD"] => Some("priory/1".to_owned()),
            ["rev-parse", "--abbrev-ref", "@{push}"] => Some("origin/main".to_owned()),
            _ => None,
        };
        let hits = Guards::default().hits("Bash", Some("git push"), &git);
        assert_eq!(hits[0].guard, "git-push-default");
        for command in [
            "git -c push.default=upstream push",
            "git -c remote.origin.push=HEAD:main push origin",
            "git --config-env=remote.origin.push=REFSPEC push origin",
            "git -c branch.priory/1.merge=refs/heads/main push",
        ] {
            assert_eq!(
                hit(command).as_deref(),
                Some("git-push-default"),
                "{command}"
            );
        }
        assert_eq!(hit("git -c user.name=x push"), None);
    }

    #[test]
    fn functions_expansions_here_strings_pipes_and_exec_are_read() {
        for command in [
            "function f { gh pr merge 12; }; f",
            "function f() { gh pr merge 12; }",
            "{gh,pr,merge,12}",
            "bash <<< 'gh pr merge 12'",
            "echo 'gh pr merge 12' | sh",
            "printf '%s\\n' 'gh pr merge 12' | bash -s",
            "$'\\x67h' pr merge 12",
            "$'\\147h' pr merge 12",
            "setsid gh pr merge 12",
            "setsid -f gh pr merge 12",
            "find . -name x -exec gh pr merge 12 \\;",
            "cat <<\"E OF\"\ntext\nE OF\ngh pr merge 12",
        ] {
            assert_eq!(hit(command).as_deref(), Some("gh-pr-merge"), "{command}");
        }
        assert_eq!(
            hit("wget --post-data 'text=hi' https://hooks.slack.com/services/x").as_deref(),
            Some("slack-write")
        );
        assert_eq!(hit("echo 'gh pr merge 12' | grep merge"), None);
        assert_eq!(hit("grep x <<< 'gh pr merge 12'"), None);
    }

    #[test]
    fn an_env_split_string_is_split_as_env_splits_it() {
        assert_eq!(
            commands(r"env -S 'gh\_pr\_merge\_12'"),
            [["gh", "pr", "merge", "12"]]
        );
        assert_eq!(
            commands(r#"env -S "a\\'b \"c d\\_e\" 'f\\_g\\'h' \\#i #j" j"#),
            [["a'b", "c d e", "f\\_g'h", "#i", "j"]]
        );
        for command in [
            r"env -S 'echo gh\_pr\_merge'",
            r"env -S 'echo hi #gh pr merge 12'",
            r"env -S 'echo hi \c gh pr merge 12'",
        ] {
            assert_eq!(hit(command), None, "{command}");
        }
        for command in [
            r"env -S 'gh pr ${VERB} 12'",
            r"env -S 'gh pr $VERB 12'",
            r"env -S 'gh pr merge \q 12'",
            r#"env -S 'gh pr "merge 12'"#,
        ] {
            assert_eq!(hit(command).as_deref(), Some(UNREADABLE), "{command}");
        }
    }

    #[test]
    fn a_script_nested_past_the_limit_is_unreadable_not_passed() {
        let nested = format!("{}gh pr merge 12{}", "echo $(".repeat(20), ")".repeat(20));
        let finds = format!("{}echo hi", "find . -exec ".repeat(40));
        for command in [nested.as_str(), &finds] {
            assert_eq!(hit(command).as_deref(), Some(UNREADABLE), "{command}");
        }
        let shallow = "bash -c 'bash -c \"echo hi\"'";
        assert_eq!(hit(shallow), None);
    }

    #[test]
    fn escaped_delimiters_echo_options_printf_formats_and_bash_bytes_are_read() {
        for command in [
            "cat <<E\\ OF\ntext\nE OF\ngh pr merge 12",
            "printf '%s ' gh pr merge 12 | bash",
            "echo -n 'gh pr merge 12' | bash",
            "$'\\547h' pr merge 12",
            "$'gh\\0ignored' pr merge 12",
        ] {
            assert_eq!(hit(command).as_deref(), Some("gh-pr-merge"), "{command:?}");
        }
    }

    #[test]
    fn find_keeps_its_own_guard_and_its_exec_nesting_is_bounded() {
        let guards = Guards {
            commands: vec![CommandGuard {
                name: "find-delete".into(),
                run: vec!["find".into()],
                flags: vec!["-delete".into()],
            }],
            tools: Vec::new(),
        };
        let git = |_: Option<&str>, _: &[&str]| None;
        let hits = guards.hits("Bash", Some("find . -name x -delete"), &git);
        assert_eq!(hits[0].guard, "find-delete");
        let nested = format!("{}gh pr merge 12", "find . -exec ".repeat(5000));
        guards.hits("Bash", Some(&nested), &git);
    }

    #[test]
    fn a_mirror_push_set_on_the_command_line_or_in_config_is_a_force_push() {
        assert_eq!(
            hit("git -c remote.origin.mirror=true push origin").as_deref(),
            Some("git-push-force")
        );
        let git = |_: Option<&str>, args: &[&str]| match args {
            ["config", "--get-regexp", ..] => Some("remote.origin.mirror true".to_owned()),
            _ => None,
        };
        let hits = Guards::default().hits("Bash", Some("git push origin"), &git);
        assert_eq!(hits[0].guard, "git-push-force");
    }

    #[test]
    fn mcp_tools_that_review_push_or_auto_merge_are_guarded() {
        let git = |_: Option<&str>, _: &[&str]| None;
        for (tool, guard) in [
            ("mcp__github__pull_request_review_write", "gh-pr-approve"),
            ("mcp__github__create_pull_request_review", "gh-pr-approve"),
            (
                "mcp__github__submit_pending_pull_request_review",
                "gh-pr-approve",
            ),
            ("mcp__github__enable_pr_auto_merge", "gh-pr-merge"),
            ("mcp__my__srv__merge_pull_request", "gh-pr-merge"),
            ("mcp__github__push_files", "git-push-default"),
            ("mcp__github__create_or_update_file", "git-push-default"),
            ("mcp__github__delete_file", "git-push-default"),
        ] {
            let hits = Guards::default().hits(tool, None, &git);
            assert_eq!(
                hits.first().map(|hit| hit.guard.as_str()),
                Some(guard),
                "{tool}"
            );
        }
    }

    #[test]
    fn gh_api_merges_by_url_merge_endpoint_or_ref_update_keep_their_guard() {
        for (command, guard) in [
            (
                "gh api -X PUT https://api.github.com/repos/o/r/pulls/12/merge",
                "gh-pr-merge",
            ),
            (
                "gh api -X PUT https://ghe.example.com/api/v3/repos/o/r/pulls/12/merge",
                "gh-pr-merge",
            ),
            (
                "gh api repos/o/r/merges -f base=main -f head=priory/7",
                "gh-pr-merge",
            ),
            (
                "gh api https://api.github.com/graphql -f query='mutation{mergePullRequest(input:{}){clientMutationId}}'",
                "gh-pr-merge",
            ),
            (
                "gh api -X PATCH repos/o/r/git/refs/heads/main -f sha=abc",
                "git-push-default",
            ),
            (
                "gh api -X DELETE repos/o/r/git/refs/heads/priory/1",
                "git-push-force",
            ),
            (
                "gh api -X PATCH repos/o/r/git/refs/heads/priory/1 -f sha=abc -F force=true",
                "git-push-force",
            ),
        ] {
            assert_eq!(hit(command).as_deref(), Some(guard), "{command}");
        }
        assert_eq!(
            hit("gh api -X PATCH repos/o/r/git/refs/heads/priory/1 -f sha=abc").as_deref(),
            Some("gh-api-write")
        );
    }

    #[test]
    fn an_apostrophe_in_a_heredoc_inside_a_substitution_hides_nothing() {
        let comment = "gh pr comment 12 --body \"$(cat <<'EOF'\nit's done\nEOF\n)\" && gh pr merge 12 --squash";
        assert_eq!(guards(comment), ["gh-pr-comment", "gh-pr-merge"]);
        let commit =
            "git commit -m \"$(cat <<-EOF\n\tdon't\n\tEOF\n)\" && git push origin HEAD:main";
        assert_eq!(hit(commit).as_deref(), Some("git-push-default"));
        let ran = "echo \"$(bash <<EOF\ngh pr merge 12\nEOF\n)\"";
        assert_eq!(hit(ran).as_deref(), Some("gh-pr-merge"));
    }

    #[test]
    fn a_short_flag_inside_a_cluster_is_read() {
        for command in [
            "gh pr review 12 -ab lgtm",
            "gh pr review 12 -ba",
            "gh pr review 12 -ab'looks good'",
        ] {
            assert_eq!(hit(command).as_deref(), Some("gh-pr-approve"), "{command}");
        }
        assert_eq!(hit("gh pr review 12 -c -b 'all good'"), None);
    }

    #[test]
    fn linear_issue_writes_through_mcp_are_guarded() {
        let git = |_: Option<&str>, _: &[&str]| None;
        for tool in [
            "mcp__linear__save_issue",
            "mcp__linear__create_issue",
            "mcp__linear__update_issue",
        ] {
            let hits = Guards::default().hits(tool, None, &git);
            assert_eq!(hits.len(), 1, "{tool}");
            assert_eq!(hits[0].guard, "mcp-write");
        }
    }

    #[test]
    fn a_wildcard_tool_guard_is_named_by_its_pattern() {
        let guards = Guards {
            commands: Vec::new(),
            tools: vec!["mcp__stripe__*".into()],
        };
        let git = |_: Option<&str>, _: &[&str]| None;

        let hits = guards.hits("mcp__stripe__create_refund", None, &git);

        assert_eq!(
            hits,
            [Hit {
                guard: "mcp__stripe__*".into(),
                call: "mcp__stripe__create_refund".into()
            }]
        );
    }

    use proptest::prelude::*;

    fn single_quoted(word: &str) -> String {
        format!("'{}'", word.replace('\'', "'\\''"))
    }

    fn double_quoted(script: &str) -> String {
        let escaped: String = script
            .chars()
            .flat_map(|c| match c {
                '\\' | '"' | '$' | '`' => vec!['\\', c],
                _ => vec![c],
            })
            .collect();
        format!("\"{escaped}\"")
    }

    /// One way to run `script` through something that must not hide it.
    fn wrap(how: u8, script: String) -> String {
        match how % 10 {
            0 => format!("sudo -u me {script}"),
            1 => format!("timeout -s KILL 30 {script}"),
            2 => format!("FOO=bar {script}"),
            3 => format!("bash -c {}", single_quoted(&script)),
            4 => format!("sh -ec {}", double_quoted(&script)),
            5 => format!("eval {}", single_quoted(&script)),
            6 => format!("true && {script}"),
            7 => format!("echo hi; {script}"),
            8 => format!("echo \"$({script})\""),
            _ => format!("( {script} ) | cat"),
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(512))]

        #[test]
        fn a_script_of_any_shape_is_split_without_panicking(
            script in proptest::string::string_regex("[a-z '\"`$(){}<>|&;\\\\\\n=#-]{0,200}").unwrap(),
            deep in 0..40usize,
        ) {
            commands(&script);
            commands(&format!("{}{script}{}", "$(".repeat(deep), ")".repeat(deep)));
            commands(&"bash -c '".repeat(deep));
            commands(&"eval ".repeat(deep));
        }

        #[test]
        fn a_mutated_guarded_command_is_matched_or_missed_without_panicking(
            command in proptest::sample::select(vec![
                "git push --force origin main", "git push origin :main", "gh api -X POST repos/o/r/pulls/1/merge",
                "gh api repos/o/r/pulls/1/reviews -f event=APPROVE", "curl -d '{}' https://slack.com/api/chat.postMessage",
                "gh pr merge 12 --admin", "git -C repo push -fu origin HEAD",
            ]),
            at in 0..60usize,
            junk in "\\PC{0,12}",
            cut in 0..60usize,
        ) {
            let chars: Vec<char> = command.chars().collect();
            let at = at.min(chars.len());
            let mut mutated: String = chars[..at].iter().collect();
            mutated.push_str(&junk);
            mutated.extend(&chars[at..chars.len().min(cut.max(at))]);
            guards(&mutated);
            guards(&format!("{mutated} {command}"));
        }

        #[test]
        fn any_text_is_split_without_panicking(script in "\\PC{0,300}") {
            commands(&script);
        }

        #[test]
        fn any_env_split_string_is_split_or_refused_without_panicking(
            text in "[a-z '\"$#{}_\\\\cntq\t-]{0,200}",
        ) {
            env_split(&text);
            commands(&format!("env -S {}", single_quoted(&text)));
        }

        #[test]
        fn a_merge_stays_matched_under_wrapping_and_quoting(
            hows in proptest::collection::vec(any::<u8>(), 0..5),
        ) {
            let command = hows
                .into_iter()
                .fold("gh pr merge 12".to_owned(), |script, how| wrap(how, script));
            prop_assert!(guards(&command).contains(&"gh-pr-merge".to_owned()), "{command}");
        }
    }
}
