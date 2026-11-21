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

- the task's current step is an irreversible model step marked `autonomous`;
- the task's current step is an irreversible model step that a person approved, and the task's
  branch is still at the commit the approval was given for;
- the task's workflow lists the guard in `guard.allow`.

Otherwise the hook denies the call. The agent reads why, and that it should call the `ask` tool if a
person must do or approve it. The daemon appends a `factory.tool.denied` record to the trace (see
[the trace schema](../trace-schema.md#factorytooldenied)), so `priory trace` shows each denial
with its task, step, guard and command.

So the guard reuses the step's approval. A workflow that needs an agent to merge gives it a step for
that, such as an irreversible `merge` model step that a human step `approves`. The daemon already
refuses to start that step without the approval. The guard then lets the merge through inside it,
and nowhere else.

A run with no task step, such as the stall handler, is denied every guarded call.

## Built-in guards

| Guard | Matches |
| ----- | ------- |
| `gh-pr-merge` | `gh pr merge` |
| `gh-pr-approve` | `gh pr review` with `--approve` or `-a` |
| `gh-pr-comment` | `gh pr comment` |
| `gh-issue-comment` | `gh issue comment` |
| `gh-api-write` | `gh api` with a method other than GET, or with a field or `--input`, which make it a POST |
| `git-push-default` | `git push` whose destination is `main`, `master` or `origin/HEAD`'s branch, or a push with no refspec from that branch |
| `git-push-force` | `git push` with `--force`, `--force-with-lease`, `-f`, `--mirror`, `--delete`, `-d`, `--prune`, a `+refspec` or a `:branch` deletion |
| `slack-write` | `curl` to `hooks.slack.com` or `slack.com/api/chat.*`, and `slack chat send` |
| `linear-write` | `curl` to `api.linear.app` with a body that holds `mutation` or comes from a file, and `linear issue create`, `update`, `delete` or `comment` |
| `mcp-write` | MCP tools named `merge_pull_request`, `add_issue_comment`, `send_message`, `slack_send_message`, `schedule_message`, `slack_schedule_message`, `save_comment` or `create_comment`, on any server but `priory` |

A push to the task's own branch passes. A factory adds its own guards with `[guard]` in
`factory.toml` (see [factory.md](../reference/factory.md#guard)).

## How a command is read

The guard parses the command rather than matching its text:

1. It splits the command into simple commands at `;`, `&&`, `||`, `|`, `&`, newlines and
   parentheses, and reads the commands inside `$(...)`, backticks and `<(...)`, quoted or not.
2. It skips heredoc bodies, so a pull request body that mentions `git push origin main` is text.
3. It drops leading `NAME=value` assignments and shell keywords, then unwraps `env`, `sudo`, `doas`,
   `nice`, `nohup`, `timeout`, `time`, `stdbuf`, `xargs`, `command` and `exec`. It reads the script
   that `sh -c`, `bash -c`, `zsh -c`, `env -S` or `eval` runs.
4. It names the program by its file name, so `/usr/bin/gh` is `gh`.
5. It drops options and the values of the options that take one, such as `gh -R` and `git -C`, then
   compares the first words left with each guard.

A quoted mention, such as `echo 'gh pr merge 12'`, and a search term, such as
`gh pr list --search merge`, are not calls.

The guard is not a sandbox. It does not see a program named by a variable, such as
`x=gh; $x pr merge 12`, a script file the agent writes and then runs, or another HTTP client such as
`wget` or `python`. It stops an agent that reaches for an irreversible tool directly or through a
wrapper.

## The hook and its deadline

Every Claude run's settings register `priory hook PreToolUse` for `Bash` and `mcp__.*` calls, with a
30-second hook timeout. A Codex run registers it for every tool (see [codex.md](codex.md)).

The hook matches the call itself, with the built-in guards and the factory's, which the run's pane
holds in `PRIORY_GUARD`. A call no guard matches passes at once, without the daemon, so reads keep
working while the daemon restarts.

For a guarded call, the hook asks the daemon over the socket (see [home.md](home.md)) and waits 5
seconds. It denies the call when the daemon says no, when no daemon listens, when the daemon does
not answer in time, and when it cannot read its input. Every one of those paths prints a deny and
exits 0, because Claude lets a call through when a `PreToolUse` hook fails.
