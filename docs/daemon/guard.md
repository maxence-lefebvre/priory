# The tool guard

The approval rule covers steps: an irreversible step runs only after a person approved it, unless it
is `autonomous` (see [workflows.md](../reference/workflows.md#irreversible-steps-and-their-gates)).
An agent inside a model step can still do something irreversible itself, such as `gh pr merge`,
`git push origin main` or a Slack post. Claude's permission deny rules do not stop it reliably: a
rule on `gh pr merge` misses `gh -R owner/repo pr merge` and `bash -c 'gh pr merge 12'`. The tool
guard closes that gap with a `PreToolUse` hook, which Claude runs before every matching call, in
every permission mode, before any allow rule.

## What the guard denies

A call is guarded when a guard matches it. A guarded call runs only when one of these holds:

- the task's current step is an irreversible model step that lists the guard in its own
  `guard.allow`, and either a person approved the step in the task's current generation and the
  task's branch is still at the commit the approval was given for, or the step is `autonomous`;
- the task's workflow lists the guard in `guard.allow`.

An approved or `autonomous` step grants only the guards it names. A step that names none may make
no guarded call beyond its workflow's list.

`gh-pr-merge` and `gh-pr-approve` need a person's approval: an `autonomous` step's grant never
covers them, and `priory check` refuses them in an `autonomous` step's `guard.allow` and in a
workflow's. So no call the guard reads merges or approves a pull request without a human step
that approves the step making it.

`priory-person` covers Priory's own verbs that only a person runs, such as `priory answer`, which
would let an agent approve its own step. Nothing grants it: `priory check` refuses it in any
`guard.allow`, so the guard denies it in every run. Those verbs also refuse to run in a shell that
has `PRIORY_SESSION`, which every run's pane sets.

A call that runs several guarded commands, such as `gh pr merge 12 && git push --force`, needs a yes
for each one. A call that also moves the branch's commit, such as `git commit -am fix && gh pr
merge 12`, cannot rely on an approval bound to the commit before it: the guard denies it and tells
the agent to run the guarded command in a call of its own.

Otherwise the hook denies the call. The agent reads why, and that it should call the `ask` tool if a
person must do or approve it. The daemon appends a `factory.tool.denied` record to the trace (see
[the trace schema](../trace-schema.md#factorytooldenied)), so `priory trace` shows each denial
with its task, step, guard and command.

So the guard reuses the step's approval. A workflow that needs an agent to merge gives it a step for
that, such as an irreversible `merge` model step with `guard = { allow = ["gh-pr-merge"] }` that a
human step `approves`. The daemon already refuses to start that step without the approval. The
guard then lets the merge through inside it, and nowhere else.

A run with no task step, such as the stall handler, is denied every guarded call.

## Built-in guards

| Guard | Matches |
| ----- | ------- |
| `gh-pr-merge` | `gh pr merge`; a `gh api` write to `repos/<owner>/<repo>/pulls/<n>/merge` or `repos/<owner>/<repo>/merges`, or to `graphql` with `mergePullRequest`, `enablePullRequestAutoMerge` or a body read from a file; and MCP tools named `merge_pull_request` or `enable_pr_auto_merge` |
| `gh-pr-approve` | `gh pr review` with `--approve`, or `-a` alone or in a cluster such as `-ab`; a `gh api` write to a pull request's `reviews` with `event=APPROVE` or a body read from a file; a `gh api` write to `graphql` with a `PullRequestReview` mutation and `APPROVE`; and MCP tools named `pull_request_review_write`, `create_pull_request_review`, `create_and_submit_pull_request_review` or `submit_pending_pull_request_review`, whatever review they write, since the guard cannot read their arguments |
| `gh-pr-comment` | `gh pr comment` |
| `gh-issue-comment` | `gh issue comment` |
| `gh-api-write` | `gh api` with a method other than GET, or with a field or `--input`, which make it a POST, other than the merges and approvals above |
| `git-push-default` | `git push` whose destination is `main`, `master` or `origin/HEAD`'s branch; a push with no refspec whose `@{push}` or current branch is one of those; `--all`, `--branches` and wildcard refspecs; a push with a `-c` or `--config-env` setting of `push.*`, `branch.*`, `remote.pushDefault` or `remote.<name>.push`; a `gh api` write to `git/refs/heads/<that branch>`; and MCP tools named `push_files`, `create_or_update_file` or `delete_file` |
| `git-push-force` | `git push` with `--force`, `--force-with-lease`, `-f`, `--mirror`, `--delete`, `-d`, `--prune`, a `+refspec` or a `:branch` deletion; a push while `remote.<name>.mirror` is set, on the command line or in git's config; and a `gh api` `DELETE` of `git/refs/heads/<branch>` or a write to it with `force=true` |
| `slack-write` | `curl` or `wget` to `hooks.slack.com` or `slack.com/api/chat.*`, and `slack chat send` |
| `linear-write` | `curl` or `wget` to `api.linear.app` with a body that holds `mutation` or comes from a file, and `linear issue create`, `update`, `delete` or `comment` |
| `mcp-write` | MCP tools named `add_issue_comment`, `send_message`, `slack_send_message`, `schedule_message`, `slack_schedule_message`, `save_comment`, `create_comment`, `save_issue`, `create_issue` or `update_issue`, on any server but `priory` |
| `connector-sign-in` | MCP tools named `authenticate` or `complete_authentication`, on any server but `priory`. See [Connector sign-in](#connector-sign-in) |
| `priory-person` | `priory answer`, `dismiss`, `deliver`, `resume`, `revive`, `cancel`, `release`, `hold`, `move`, `say`, `pause`, `unpause`, `snooze`, `seen`, `subscribe`, `unsubscribe`, `give`, `eval`, `bench`, `label` and `report pick`. `priory add`, `request` and `gate` stay open to agents |
| `unreadable` | A script nested more than 16 levels deep, through `bash -c`, `$(...)`, `eval`, `find -exec` and the like. The guard cannot read it, so the daemon denies it: "too deeply nested for the guard to read". Nothing grants it |

An MCP tool's server is the part of its name before the last `__`, so a server named `my__srv`
cannot hide a tool's name.

A push to the task's own branch passes. A task's branch created since tracking was dropped tracks
nothing, so a bare `git push` with `push.default = upstream` does not reach the trunk. For an older
branch that still tracks it, the guard reads the push's destination from `@{push}`. `git push --dry-run`, `git push -n` and a help
request pass too. A help request is `--help`, or `-h` for `gh` and `git`, after a word that is no
option and before `--`: in `gh pr merge 12 --body --help` the `--help` is the body, so the merge is
guarded. A factory adds its own guards with `[guard]` in
`factory.toml` (see [factory.md](../reference/factory.md#guard)). A factory guard that matches the
same call as `gh-pr-merge` or `gh-pr-approve`, such as one on `gh pr` or `mcp__github__*`, does not
rename it: the call keeps its built-in guard.

## Connector sign-in

An MCP server that needs a sign-in offers an `authenticate` tool in place of its own. A run that
calls it clears the stored token, so one unattended run signs out every later run. The guard
denies `authenticate` and `complete_authentication` in every run, and only an agent with
`connectorSignIn = true` in its frontmatter passes (see [agents.md](../reference/agents.md)).
Nothing else grants `connector-sign-in`: `priory check` refuses it in any `guard.allow`.

The denial tells the agent to carry on with its other sources and to name the connector in its
result. The connector is the server in the tool's name: `linear` for `mcp__linear__authenticate`.
Each denial also does two things:

- It appends a `factory.tool.denied` record with the task, step and tool, as any denial does.
- It adds a to-do, `Sign in to <connector>`, to the desk, unless one is open already. Two runs
  denied for one connector leave one item. See [desk.md](desk.md#sign-in-to-dos).

## How a command is read

The guard parses the command rather than matching its text:

1. It splits the command into simple commands at `;`, `&&`, `||`, `|`, `&`, newlines and
   parentheses, and reads the commands inside `$(...)`, backticks and `<(...)`, quoted or not.
2. It reads a heredoc body as text, so a pull request body that mentions `git push origin main`
   is not a command. It still reads the `$(...)` and backticks of a body whose delimiter is not
   quoted, and reads the body as a script when a shell on that line reads its stdin, as in
   `bash <<'EOF'` or `cat <<'EOF' | sh`. A heredoc inside `$(...)` is read the same way, so an
   apostrophe in its body hides nothing after it. It reads a here-string, and the text `echo` or
   `printf` pipes, as a script when a shell reads it, as in `bash <<< '...'` or
   `echo '...' | sh`.
3. It drops redirections, with their descriptor numbers, such as `2>/dev/null`.
4. It drops leading `NAME=value` assignments and shell keywords, then unwraps `env`, `sudo`, `doas`,
   `nice`, `nohup`, `setsid`, `timeout`, `time`, `stdbuf`, `xargs`, `command` and `exec`, with
   their options. It reads the script that `sh -c`, `bash -c`, `zsh -c`, `env -S` or `eval` runs,
   `$'...'` quotes with their `\xHH` and octal escapes decoded to a byte as Bash does, the commands
   of `find -exec` as well as `find` itself, the
   body of `function f { ...; }`, and a brace list such as `{gh,pr,merge,12}`.
5. It names the program by its file name, so `/usr/bin/gh` is `gh`.
6. It drops options and the values of the options that take one, such as `gh -R` and `git -C`, then
   compares the first words left with each guard.

A quoted mention, such as `echo 'gh pr merge 12'`, and a search term, such as
`gh pr list --search merge`, are not calls.

The guard is not a sandbox. It does not see a program named by a variable, such as
`x=gh; $x pr merge 12`, a script file the agent writes and then runs, another HTTP client such as
`python`, or a line written straight to the daemon's socket, as with `nc`. Nor does it see a message
published straight to the log, as with herdr-bus's CLI or `psql`, such as a `priory.ask.answered`
that answers another runner's ask. Nor does it see `priory report reply` run with `PRIORY_SESSION`
unset, which the daemon stores as a person's reply. It stops an agent that reaches for an irreversible tool
directly or through a wrapper.

## The hook and its deadline

Every Claude run's settings register `priory hook PreToolUse` for `Bash` and `mcp__.*` calls, with a
30-second hook timeout. A Codex run registers it for every tool (see [codex.md](codex.md)).

The hook matches the call itself, with the built-in guards and the factory's, which the run's pane
holds in `PRIORY_GUARD`. A call no guard matches passes at once, without the daemon, so reads keep
working while the daemon restarts.
A call to a connector with an open sign-in to-do that the guards allow also reports the use to the daemon, and waits at most one second for the answer (see [desk.md](desk.md#sign-in-to-dos)).

For a guarded call, the hook asks the daemon over the socket (see [home.md](home.md)) about each
guard the call hits, and waits up to 5 seconds for each answer. It denies the call when the daemon
says no, when no daemon listens, when the daemon does not answer in time, and when it cannot read
its input. The whole hook, git and the connection included, has 8 seconds; past that, or if it
panics, it denies. Every one of those paths prints a deny and exits 0, because Claude lets a call
through when a `PreToolUse` hook fails or times out.
