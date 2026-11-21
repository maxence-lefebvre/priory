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
];

/// The guards only a person's approval grants: never `autonomous` nor a workflow's `guard.allow`.
pub const NEED_APPROVAL: &[&str] = &["gh-pr-merge", "gh-pr-approve"];

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
        let (server, short) = name.split_once("__")?;
        if server == "priory" {
            return None;
        }
        if short == "merge_pull_request" {
            return Some(Hit {
                guard: "gh-pr-merge".to_owned(),
                call: tool.to_owned(),
            });
        }
        let added = self
            .tools
            .iter()
            .find(|pattern| match pattern.strip_suffix('*') {
                Some(prefix) => tool.starts_with(prefix),
                None => *pattern == tool,
            });
        let guard = match added {
            Some(pattern) => pattern.clone(),
            None if TOOLS.contains(&short) => "mcp-write".to_owned(),
            None => return None,
        };
        Some(Hit {
            guard,
            call: tool.to_owned(),
        })
    }

    fn command(&self, words: &[String], git: Git) -> Option<Hit> {
        let help = |arg: &String| {
            arg == "--help" || (arg == "-h" && matches!(words[0].as_str(), "gh" | "git"))
        };
        if words[1..].iter().any(help) {
            return None;
        }
        let hit = |guard: &str| {
            Some(Hit {
                guard: guard.to_owned(),
                call: words.join(" "),
            })
        };
        let built_in = COMMANDS
            .iter()
            .find(|(_, run, flags)| starts(words, run, flags))
            .map(|(name, _, _)| *name)
            .or_else(|| match words[0].as_str() {
                "gh" => gh_api(words),
                "git" => git_push(words, git),
                "curl" => curl(words),
                _ => None,
            });
        // A factory guard must not rename a merge or an approval, which only a person grants.
        if let Some(name) = built_in.filter(|name| NEED_APPROVAL.contains(name)) {
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

/// Whether `words` runs `run[0]` with positional words starting with the rest of `run`, and holds
/// one of `flags` if any.
fn starts<S: AsRef<str>>(words: &[String], run: &[S], flags: &[S]) -> bool {
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
            })
        });
    leads && flagged
}

/// Options of a program that take the next word as their value.
fn takes_value(program: &str, option: &str) -> bool {
    let options: &[&str] = match program {
        "gh" => &["-R", "--repo"],
        "git" => &["-C", "-c", "--git-dir", "--work-tree", "--namespace"],
        _ => &[],
    };
    options.contains(&option)
}

/// The arguments after the program that are no option nor an option's value.
fn positional(words: &[String]) -> Vec<&str> {
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

/// `gh api` that writes: a method other than GET, or a field or body, which makes it a POST. A
/// merge or an approving review is named as such, and so is one whose body Priory cannot read.
fn gh_api(words: &[String]) -> Option<&'static str> {
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
    let writes = match method {
        Some(method) => !method.eq_ignore_ascii_case("GET"),
        None => opaque || !fields.is_empty(),
    };
    if !writes {
        return None;
    }
    let endpoint = endpoint.unwrap_or_default();
    let endpoint = endpoint.trim_start_matches('/');
    let endpoint = endpoint.split('?').next().unwrap_or_default();
    let parts: Vec<&str> = endpoint.split('/').collect();
    let pull = |tail: &str| {
        matches!(parts.as_slice(), ["repos", _, _, "pulls", number, first, ..]
            if number.parse::<u64>().is_ok() && *first == tail)
    };
    if pull("merge") {
        return Some("gh-pr-merge");
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

/// `git push` that forces, deletes, or updates the default branch. A push of every branch or of
/// a wildcard counts as one to the default branch.
fn git_push(words: &[String], git: Git) -> Option<&'static str> {
    let mut dir = None;
    let mut args = words[1..].iter();
    loop {
        let arg = args.next()?;
        if arg == "-C" {
            dir = args.next().map(String::as_str);
        } else if takes_value("git", arg) {
            args.next();
        } else if !arg.starts_with('-') {
            if arg != "push" {
                return None;
            }
            break;
        }
    }
    let mut refspecs = Vec::new();
    let mut remote_given = false;
    let mut every = false;
    while let Some(arg) = args.next() {
        let arg = arg.as_str();
        match arg {
            "--dry-run" => return None,
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
                        'n' => return None,
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
    if every || refspecs.iter().any(|spec| spec.contains('*')) {
        return Some("git-push-default");
    }
    let current = || git(dir, &["symbolic-ref", "--short", "HEAD"]);
    let targets: Vec<String> = if refspecs.is_empty() {
        current().into_iter().collect()
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
    let origin = git(
        dir,
        &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"],
    );
    let default = origin
        .as_deref()
        .and_then(|head| head.strip_prefix("origin/"));
    targets
        .iter()
        .any(|target| {
            ["main", "master"].contains(&target.as_str()) || Some(target.as_str()) == default
        })
        .then_some("git-push-default")
}

/// `curl` that posts to Slack's chat or a webhook, or sends Linear a mutation. A body read from a
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
    let mut found = Vec::new();
    parse(script, 0, &mut found);
    found
}

fn parse(script: &str, depth: usize, found: &mut Vec<Vec<String>>) {
    if depth > DEPTH {
        return;
    }
    let mut split = Split {
        chars: script.chars().collect(),
        at: 0,
        depth,
        word: None,
        words: Vec::new(),
        heredocs: Vec::new(),
        redirect: false,
        stdin_shell: false,
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
        redirect: false,
        stdin_shell: false,
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
    redirect: bool,
    /// A command on this line is a shell reading its script from stdin, as from a heredoc.
    stdin_shell: bool,
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
        if let Some(word) = self.word.take() {
            if std::mem::take(&mut self.redirect) {
                return;
            }
            self.words.push(word);
        }
    }

    fn end_command(&mut self) {
        self.end_word();
        let words = std::mem::take(&mut self.words);
        let before = self.found.len();
        unwrap(&words, self.depth, self.found);
        self.stdin_shell |= self.found[before..].iter().any(|command| {
            SHELLS.contains(&command[0].as_str())
                && command[1..].iter().all(|arg| arg.starts_with('-'))
        });
    }

    fn nested(&mut self, script: &str) {
        parse(script, self.depth + 1, self.found);
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
                ';' | '&' | '|' | '(' | ')' => self.end_command(),
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
                c => self.push(c),
            }
        }
        self.end_command();
    }

    /// The rest of a `$'...'` word, with its backslash escapes decoded.
    fn ansi_c_quoted(&mut self) {
        self.word.get_or_insert_default();
        while let Some(c) = self.peek(0) {
            self.at += 1;
            match c {
                '\'' => return,
                '\\' => {
                    let Some(next) = self.peek(0) else {
                        return;
                    };
                    self.at += 1;
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
                if "'\"\\".contains(c) {
                    quoted = true;
                } else {
                    delimiter.push(c);
                }
                self.at += 1;
            }
            self.heredocs.push((delimiter, strip, quoted));
            return;
        }
        while self.peek(0).is_some_and(|c| "<>&|".contains(c)) {
            self.at += 1;
        }
        if self.chars[self.at - 1] == '&' {
            while self.peek(0).is_some_and(|c| c.is_ascii_digit() || c == '-') {
                self.at += 1;
            }
            return;
        }
        self.redirect = true;
    }

    /// Reads the heredoc bodies that start here: as a script when a shell on the line reads its
    /// stdin, else as text whose substitutions run unless the delimiter was quoted.
    fn skip_heredocs(&mut self) {
        let script = std::mem::take(&mut self.stdin_shell);
        for (delimiter, strip, quoted) in std::mem::take(&mut self.heredocs) {
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
            if script {
                parse(&body, self.depth + 1, self.found);
            } else if !quoted {
                substitutions(&body, self.depth, self.found);
            }
        }
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

    /// The text up to the parenthesis that closes one just read, skipping quoted parentheses.
    fn balanced(&mut self) -> String {
        let start = self.at;
        let mut depth = 1;
        let mut quote = None;
        while let Some(c) = self.peek(0) {
            self.at += 1;
            match (quote, c) {
                (Some(q), c) if c == q => quote = None,
                (Some('"'), '\\') | (None, '\\') => self.at += 1,
                (Some(_), _) => {}
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
    let mut at = 0;
    while let Some(word) = words.get(at) {
        if assignment(word) || KEYWORDS.contains(&word.as_str()) {
            at += 1;
            continue;
        }
        let program = word.rsplit('/').next().unwrap_or(word);
        let rest = &words[at + 1..];
        let skip = |values: &[&str]| options(rest, values);
        match program {
            "env" => {
                let mut index = 0;
                while let Some(arg) = rest.get(index) {
                    if arg == "-S" || arg == "--split-string" {
                        let mut script = rest.get(index + 1).cloned().unwrap_or_default();
                        for word in rest.iter().skip(index + 2) {
                            script.push(' ');
                            script.push_str(word);
                        }
                        return parse(&script, depth + 1, found);
                    }
                    if let Some(script) = arg.strip_prefix("--split-string=") {
                        return parse(script, depth + 1, found);
                    }
                    if ["-u", "-C", "--unset", "--chdir"].contains(&arg.as_str()) {
                        index += 2;
                    } else if arg.starts_with('-') || assignment(arg) {
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
            "stdbuf" | "nohup" => at += 1 + skip(&[]),
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
}
