# Credentials per agent

This page explains how an agent could name the secrets it gets, how Priory could set them in that
run's environment only, and how `priory check` could prove that no review agent holds a token that
can push. It builds nothing. It holds the model, the first code slice with its tests, the slices
after it, and the open questions with a recommendation for each.

The idea comes from a read of other agent factories on 2026-11-22. [OpenHands' software factory
guide](https://docs.openhands.dev/openhands/usage/use-cases/software-factory) gives triage,
developer and reviewer three fine-grained tokens, so the reviewer "cannot push code". [Devin's
security profiles](https://docs.devin.ai/product-guides/security-profiles) bind network, MCP and git
limits to an automation. [Codex](https://developers.openai.com/codex/cloud/environments) removes
secrets before the agent phase.

The PRD lists the risk and no mitigation: "Agents run with the user's credentials"
([prd.md](../prd.md#risks)).

## What existed when this note was written

- A run starts in a tmux session with the variables Priory adds "on top of the daemon's own"
  (`crates/priory-runtime/src/runtime.rs:541`). `launch` passes `PRIORY_HOME`, `PRIORY_SESSION` and
  the run's extra variables with `-e` (`runtime.rs:805`, `:815`). Everything else comes from the
  tmux server's environment, which is the daemon's when the daemon started the server.
- The only variables a run loses are the twelve a Claude Code session sets for its children
  (`runtime.rs:499`, `:835`).
- The run's extra variables are the telemetry ids, `PRIORY_GUARD` and the connectors list
  (`crates/priory-core/src/daemon.rs:1930`, `:2163`).
- The factory names its own secrets by variable, never by value: the server's `token_env`
  (`crates/priory-factory/src/factory.rs:291`) and each webhook's `secret_env` (`:381`).
  `secret_envs` lists them (`:1985`), and only the screen's redaction reads the list
  (`crates/priory-core/src/screen.rs:53`, `daemon.rs:1275`).
- So, from the code and not from a test: a run can read the server's token, every webhook secret
  and every other variable of the daemon. It also uses the person's `gh` login, git credentials
  and SSH keys, since it runs as the same user.
- An agent names `mcpServers` and `connectorSignIn` (`factory.rs:776`, `:780`). It names no
  secret ([agents.md](../reference/agents.md)).
- The tool guard denies irreversible calls by reading the command. It is not a sandbox
  ([guard.md](../daemon/guard.md#how-a-command-is-read)).
- The `review` starter's reviewer writes a pending review with a `gh api` POST and must never
  push (`examples/review/workflows/`).
- `open-pr` pushes and opens the pull request from the daemon, with the daemon's own `gh`
  (`runtime.rs:394`).

## The model

### A secret has a name, and the factory never holds its value

`factory.toml` declares each secret an agent can get:

```toml
[secrets.github-write]
env = "PRIORY_GH_WRITE"      # the variable of the daemon's environment that holds the value
as = "GH_TOKEN"              # the variable the run gets
push = true                  # a person's statement: this token can push or merge

[secrets.github-review]
env = "PRIORY_GH_REVIEW"
as = "GH_TOKEN"
push = false
```

The value stays where secrets already live, such as `~/.config/priory/env`. `push` is a
declaration a person makes when they create the token. The daemon checks it against GitHub
before a review agent's run starts.

An agent lists the secrets it gets:

```toml
harness = "claude-code"
secrets = ["github-review"]
push = false                 # this agent must never hold a token that can push
```

### In that run's environment only

When a run starts, the daemon resolves each named secret from its own environment and gives it to
that run's process, and nowhere else:

- Not on a command line. A `-e NAME=value` argument shows in `ps`. The daemon writes the pairs to
  a file under `PRIORY_HOME` with mode `0600`, and the `sh` wrapper that already reads and removes
  the prompt file (`runtime.rs:824`) reads it, exports the pairs and removes it.
- Not in the tmux server's environment, so no other session inherits it.
- Not in the MCP configuration, the hooks' settings, the trace, a log or an error. A secret that
  is missing blocks the task with `agent reviewer: secret github-review: PRIORY_GH_REVIEW is not
  set`, which names the variable and never a value.
- Added to the screen's redaction list for as long as any run holds it.

A run also loses what it was not given. Every run starts with these unset:

- every variable in `secret_envs`: no agent needs the server's token or a webhook secret;
- the `env` of every declared secret, since the run gets its own under `as`;
- for an agent that sets `secrets`, the `as` names of the secrets it did not name.

An agent with no `secrets` key keeps the rest of the daemon's environment, as today, so a factory
moves one agent at a time. A review agent keeps none of it, as the next section says.

### A reviewer token that cannot push

"Cannot push" is narrower than "cannot write". The `review` starter's reviewer writes a pending
review, so its token writes pull request reviews. What it must not do is change code: push a
branch, merge, or write a file through the API.

An agent with `push = false` does not inherit the daemon's environment at all. Its run starts
from a short list, `PATH`, `LANG`, `TERM`, `HOME`, Priory's own variables and what the harness
needs to sign in, plus its named secrets. So a token under a name the factory never declared does
not reach it. The run also gets:

| Setting | Why |
| ------- | --- |
| `GH_TOKEN` from its secret | `gh` prefers it over the person's stored login |
| `GH_CONFIG_DIR` set to an empty folder | `gh` finds no stored login to fall back on |
| `SSH_AUTH_SOCK` unset | git finds no agent that holds the person's keys |
| `GIT_CONFIG_COUNT` pairs that rewrite `git@github.com:` to `https://github.com/` and name `gh` as the credential helper | a push authenticates with the run's token, and GitHub refuses it |
| `GIT_TERMINAL_PROMPT=0` | a refused push fails instead of asking |

A person's `push = false` on a secret is a claim. The daemon checks it before it trusts it. When
a `push = false` run starts, the daemon probes GitHub, with the run's token, for what the token
may do in the task's repository. It starts the run only when the probe shows the token cannot
push. A token that can push, and a probe the daemon cannot read, both block the task:
`agent reviewer: secret github-review can push to owner/name, and the agent has push = false`.
Unknown is never safe. The answer is kept for an hour per token and repository, by the token's
hash.

The probe is not chosen yet, and the slice may not claim this proof until one is shown to work.
The repository's `permissions` field may describe the person and not the token. A write request
that cannot succeed, such as creating a ref at a commit that does not exist, may answer
differently with and without the right to push, and changes nothing either way. Before the slice
builds the check, it runs the candidate against a real token of each kind it supports, a
fine-grained personal token and a GitHub App token, with and without the right to push, and
records the answers in this note. A kind with no working probe is refused for a `push = false`
agent. See open question 7.

One limit remains, and it is why this is a default and not a wall. The run is still the person's
Unix user. It can read `~/.ssh` and `~/.config/gh/hosts.yml`, and an agent that goes looking will
find them. The claim is exact: a review agent's environment holds no token that can push, and its
git and `gh` do not use the person's login. Later slice 2 makes it a wall.

### What `priory check` proves

A review agent is an agent with `push = false`. `priory check` refuses, with file and line:

- a secret named by an agent and not declared, and a declared secret with no `env` or `as`;
- two secrets of one agent with the same `as`;
- a `push = false` agent that names a secret with `push = true`:
  `agents/reviewer/agent.md:5: agent reviewer has push = false and names secret github-write, which can push`;
- a `push = false` agent that names no secret for `GH_TOKEN`, since it would have no token it is
  allowed to use;
- a `push = false` agent with `mcpServers`, since an MCP server holds credentials the factory
  cannot see. Later slice 1 lifts this for servers declared read-only;
- an agent without `push = false` on a workflow that a `pull_request.review_requested` trigger
  starts, or at a step whose `effect` is `read`: the factory says the work changes no code, so its
  agent must not be able to;
- a `push = false` agent at a step whose `guard.allow` holds `git-push-default`,
  `git-push-force` or `gh-pr-merge`.

`priory check --explain` gains a table: each agent, its secrets, the variables it gets, and
`push`. No row with `push = false` lists a secret declared to push.

So the proof has two halves. `priory check` proves it for the files: no review agent names a
token declared to push, or keeps a variable or an MCP server that could hide one. The check at a
run's start proves it for the token itself. Neither sees the files in the person's home, and the
table says so under it.

## The first slice: named secrets, the scrub, the check and the token's proof

1. `[secrets.<name>]` in `factory.toml`, and `secrets` and `push` on an agent, in the schema and
   the JSON Schemas.
2. The check rules above, and the `--explain` table.
3. The run's environment: the file handoff, the unset list, the short list and settings for
   `push = false`, and the redaction list.
4. The probe at the start of a `push = false` run, once it is shown to work on real tokens, and
   the blocked task when the token can push or the answer is unknown.
5. A blocked task for a missing secret.
6. The `review` starter declares `github-review`, and its two agents set `push = false`.
7. [agents.md](../reference/agents.md), [factory.md](../reference/factory.md),
   [runs.md](../daemon/runs.md) and a how-to guide, "How to give a review agent a token that
   cannot push".

Tests, through the CLI and a run that prints its environment, as the runtime tests start a fake
harness. GitHub is a local HTTP server, the network's stand-in:

- A run of an agent with `secrets = ["github-review"]` sees `GH_TOKEN` with the value of
  `PRIORY_GH_REVIEW`, and does not see `PRIORY_GH_REVIEW`, `PRIORY_GH_WRITE`, the server's token
  variable or a webhook's secret variable.
- A run of an agent with no `secrets` key still loses the server's token and the webhook secrets,
  and keeps an unrelated variable of the daemon.
- A run of a `push = false` agent does not see an unrelated variable of the daemon, such as an
  undeclared `OTHER_GH_TOKEN`, and still starts its harness.
- The secret's value appears in no argument of the tmux command, in no file left under
  `PRIORY_HOME` after the run started, in no trace record and in no line of the daemon's log.
- A second run, started while the first holds a secret, does not see it.
- The run's screen shows `[redacted]` where the run printed its secret.
- A run of a `push = false` agent has an empty `GH_CONFIG_DIR`, no `SSH_AUTH_SOCK`, and
  `git config --get-regexp '^url\.'` shows the rewrite. Its `git push` to a remote on the local
  HTTP server, which refuses the review token, fails, and the server saw that token and no other.
- When the local server says the token can push, the `push = false` run does not start and the
  task blocks with the message above. The same when the server answers `500`, and when it is
  down.
- A second run within the hour asks the server nothing, and a run after it asks again.
- A missing variable blocks the task with its message, and the message holds no value.
- Each check rule fails with its line: an undeclared secret, a duplicate `as`, a `push = false`
  agent with a pushing secret, with no `GH_TOKEN`, with `mcpServers`, on a step that allows
  `git-push-default`, and a review-request workflow whose agent lacks `push = false`.
- `priory check` passes on every starter in `examples/`.
- A Codex agent gets the same environment: the handoff happens before the harness starts.

## Later slices

1. MCP servers. A server gains a declared `writes`, and a `push = false` agent may name a server
   that does not write.
2. Network policy, and with it the wall. On a systemd host, a run starts in its own scope
   ([agents-slice.md](../daemon/agents-slice.md)). An agent gains `network`, a list of hosts. The
   run reaches the network only through a local proxy that allows those hosts, and the same
   change hides the person's `~/.ssh` and `gh` login from a `push = false` run. See open
   question 2.
3. `priory check --tokens`, run by a person on a runner: the same question to GitHub for every
   declared secret and every repository of the factory, before any task runs. It prints each
   secret's name and verdict, never a value, and joins the `priory-person` guard.
4. Secrets for a step's `record` command and a `run` action, which start with the same scrub.
5. A secret per repository: `[secrets.<name>]` gains `repos`, so a factory of several repositories
   gives a run the token of its task's repository only.

## Open questions

1. **How is a review agent identified?** By its workflow's name, by a `role`, or by `push = false`.
   A name is a convention nothing checks. A role invents a concept the PRD does not have.
   Recommendation: `push = false` on the agent, plus the rule that review-request workflows and
   `read` steps require it. The property is stated where it is enforced.
2. **What enforces the network policy?** systemd's `IPAddressAllow` works on addresses, not names,
   and a user manager may not be allowed to load its filter. A proxy named in `HTTPS_PROXY` is a
   request, not a limit. A mount namespace through `bwrap` or systemd's `PrivateTmp` and
   `InaccessiblePaths` can hide the person's credentials, and needs testing under a user manager.
   Recommendation: write a second design note after a spike that answers two things on the
   author's machines: whether a user scope can deny all addresses but the loopback, and whether
   it can hide two paths. Do not ship a policy that only the well-behaved agent obeys.
3. **Should every run lose the whole daemon environment?** A short list is safer than the unset
   list. It also breaks factories that rely on `CLAUDE_CODE_USE_BEDROCK` or a proxy setting.
   Recommendation: the short list for `push = false` agents only, where the proof needs it, with
   an `env = [...]` key to add a name by hand. `priory check` refuses a name that a declared
   secret uses, and the `--explain` table lists every added name, since each is a hole a person
   opened on purpose. Other agents keep the unset list until a factory
   asks for more.
4. **Does an implementer need a pushing token at all?** `open-pr` pushes from the daemon. An
   agent that only commits could run with `push = false`, and fixes after a review would need a
   code step that pushes. Recommendation: not in this note. Measure first how many starter steps
   push from the agent, with the guard's records.
5. **Where does the value come from besides the environment?** A password manager command, such
   as `op read`, keeps the value out of a file. Recommendation: `env` only in the first slice.
   Add `command = [...]` when a person asks, run at each start with its output never logged.
6. **Should Priory warn about a factory with no `[secrets]`?** Every factory today has none.
   Recommendation: no warning from `priory check`. The `--explain` table prints `the person's own
   credentials` for an agent with no `secrets`, which is the honest state.
7. **Which request proves what a token can do?** Unknown until tried. Recommendation: try the
   write request that cannot succeed first, against a scratch repository. If no request tells
   the two cases apart for a kind of token, support only the kinds where one does, and say so in
   the how-to guide. Do not fall back to trusting the declaration.
