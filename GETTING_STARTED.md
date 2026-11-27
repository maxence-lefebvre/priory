# Getting started

In this tutorial, you build Priory, write a factory with one agent, one workflow and one
[trigger](docs/concepts.md), run it, and answer an agent's first question in the web view on your
own machine. By the end, an agent has changed a repository of yours and opened a pull request
that you approved.

## Before you start

You need:

- Linux, or macOS on Apple silicon. CI runs Priory's test suite on Linux only. With a systemd
  user session, every agent runs in a slice that caps its CPU and memory; without one, as on
  macOS, agents still run (see [agents-slice.md](docs/daemon/agents-slice.md)).
- On `PATH`: `git`, `tmux`, `openssl`, `gh` logged in to GitHub, and `claude`, logged in. A
  factory that jails a workflow's agents also needs `bwrap`, which is Linux only (see
  [How to jail a workflow's agents](docs/jailing-a-workflow.md)); this tutorial's does not. The agent of this
  tutorial runs Claude Code, its harness. To run Codex instead, set `harness = "codex"` in step 2 and read
  [codex.md](docs/daemon/codex.md).
- The Rust toolchain, to build from source. `rust-toolchain.toml` pins the version, and `rustup` installs it on the first
  build.
- A GitHub repository you can push to, for the agent to work in, and a checkout of it on this
  machine. This page calls the repository `owner/name`.

Six words come back on every page. [Concepts](docs/concepts.md) explains them in full.

- A **factory** is a folder of files that says which agents exist and what they do.
- A **task** is one unit of work, such as one issue to fix.
- A **step** is one stage of a task. A model, a person or code decides its outcome.
- An **ask** is a question an agent, or a step, puts to a person.
- The **desk** is the one list of everything that waits on a person.
- A **trigger** says which events, such as a label added to an issue, start a workflow.

## 1. Build Priory

To skip the build, install the release binary with `install.sh` or a build with Nix, as
[How to install Priory](docs/installing-a-release.md) shows, then go to step 2. This step builds
Priory from a clone:

```sh
git clone https://github.com/maxence-lefebvre/priory
cd priory
cargo install --path crates/priory
priory --version
```

`cargo install` builds Priory and puts `priory` in `~/.cargo/bin`, which `rustup` added to your
`PATH`, so every terminal you open finds it. The last command prints the version, such as
`priory 1.3.16`.

## 2. Write a factory

A factory is a folder of files. Make an empty working folder, outside your clone if you built
Priory from one, then create these four files in an `example/` folder inside it.

`factory.toml` names the factory, the workflow `priory add` starts by default, and where the web
view listens:

```toml
name = "example"

[request]
workflow = "implement"

[serve]
listen = "127.0.0.1:7340"
```

`agents/implementer/agent.md` describes who runs. The frontmatter between the `+++` lines sets the
harness, the model and the permissions. The Markdown body becomes the agent's system prompt:

```markdown
+++
description = "Implements one task and opens its pull request."
harness = "claude-code"
model = "sonnet"
memory = "6GB"
permissions = { defaultMode = "auto", allow = ["Bash(cargo test:*)"] }
+++
Read the request, make the smallest correct change, and test it.
```

`workflows/implement.toml` lists the steps a task goes through:

```toml
start = "implement"

[[steps]]
id = "implement"
decides = "model"
agent = "implementer"
prompt = "Implement the request and run the tests."
effect = "reversible"
next = { passed = "approve-pr", failed = "stop" }

[[steps]]
id = "approve-pr"
decides = "human"
effect = "none"
approves = ["open-pr"]
options = ["open", "stop"]
next = { open = "open-pr", stop = "stop" }

[[steps]]
id = "open-pr"
decides = "code"
action = "open-pr"
effect = "irreversible"
next = { opened = "done", failed = "stop" }
```

`triggers/agent-label.toml` starts that workflow when someone adds the label `agent` to an issue.
Put your repository in place of `owner/name`:

```toml
source = "github"
repo = "owner/name"
on = "issues.labeled"
filter = { label = "agent" }
workflow = "implement"
key = "issue.number"
```

To start from a ready-made factory later, such as one that reviews pull requests or triages new
issues, see [How to start from a starter factory](docs/starting-from-a-starter.md). Finish this
tutorial with the four files first: a starter needs more setup.

## 3. Check and draw it

Run these from your working folder:

```sh
priory check example
priory draw example implement
```

`priory check` prints `ok: 1 agent, 1 trigger, 1 workflow, 0 runners`. A runner is a file that
describes one machine, and one machine needs none. When a file has a problem, `priory check`
prints the file, the line and what is wrong, and exits 1.

`priory draw` prints the workflow as a Mermaid flowchart, with each step's decider and effect. To
see it drawn, paste it into a Mermaid viewer, or open Flows in the web view after step 4.
Flows draws the workflow like this:

![The implement workflow. The model step implement leads to the human step approve-pr on passed, and to stop on failed. A person answers open, which leads to the code step open-pr, or stop. open-pr carries a padlock: it is irreversible and runs only after a person approved it. It leads to done on opened, and to stop on failed. A model's decisions are dashed arrows, a person's are thick solid arrows, and code's are thin dashed arrows.](docs/images/diagrams/workflow.svg)

`priory check --explain example` prints four tables. The first lists the irreversible step
`open-pr`, with the human step `approve-pr` that approves it in the `gate` column. The other
three are empty for this factory. `priory check` refuses a workflow that reaches an irreversible
step with no person's approval.

Then check your repository's checkout, here `../my-project`:

```sh
priory check example --repo ../my-project
```

It prints a score, such as `readiness: 5 of 7 met, 2 missing, 0 unknown`, then one line per
criterion, such as `met`, `test-command` and `cargo test`. Each missing one has a line that says how
to fix it, such as an `AGENTS.md` or a `gate` in `factory.toml`. It changes nothing. An agent left alone does better
in a repository that meets them all. See [readiness.md](docs/reference/readiness.md).

## 4. Run it

Priory runs as two processes. The daemon turns requests into tasks and runs the agents. The intake
polls GitHub every 60 seconds, for the trigger's label and for the checks of the pull requests
your tasks open.

In a first terminal, from your working folder, start the daemon:

```sh
mkdir -p ~/.local/state/priory-oss
export PRIORY_DB=~/.local/state/priory-oss/priory.db
export PRIORY_TOKEN=$(openssl rand -hex 32)
echo "$PRIORY_TOKEN"
priory daemon example
```

`PRIORY_DB` names the file where the daemon keeps its state, and the daemon refuses to start
without it. `PRIORY_TOKEN` is the password of the web view: with a `[serve]` table, the daemon
also refuses to start without it. Copy the token the `echo` printed. The daemon prints
`priory: serving on http://127.0.0.1:7340`. Leave it running.

In a second terminal, from your working folder, start the intake, and leave it running too:

```sh
export PRIORY_DB=~/.local/state/priory-oss/priory.db
priory intake example
```

Open `http://127.0.0.1:7340` in a browser on the same machine and paste the token. The web view
opens on the Desk, which is empty. The server listens on this machine only.

### Trust the worktrees folder, once

Each task's agent works in a new folder under `~/.local/state/priory-oss/worktrees/`. Claude asks
whether it trusts a folder it has not seen, and Priory never answers that for you. Some Claude
Code versions accept a folder because a folder above it is trusted, so trust the worktrees folder
now, before the first run:

```sh
mkdir -p ~/.local/state/priory-oss/worktrees
cd ~/.local/state/priory-oss/worktrees && claude   # choose "Yes, I trust this folder", then /exit
```

## 5. Ask for a first task

In a third terminal, from your working folder, with your repository in place of `owner/name`:

```sh
export PRIORY_DB=~/.local/state/priory-oss/priory.db
priory add example --repo owner/name "Fix the typo in the README"
priory tasks
```

`priory add` prints the task's key, such as `cli:2026-11-20-1`. Within a second the daemon makes
task 1 from it, and `priory tasks` lists it at the step `implement`. Its state is `queued` until
the machine has room for the agent (see [room.md](docs/daemon/room.md)), then `running`. Ignore
the `band` and `lane` columns for now: they order and group tasks, and [Concepts](docs/concepts.md)
explains them.

The daemon clones your repository under `~/.local/state/priory-oss/repos/`, adds a git worktree, a
folder of its own, on the branch `priory/1`, and starts `claude` in the tmux session `priory-1`.
To watch the agent work, open the task in the web view, or run `priory attach 1 --read-only`.

### Trust the clone, once

Other Claude Code versions accept a task's folder only when you trusted its repository's clone.
On the first run in a repository, Claude may still ask, and the task turns `blocked` within a few
seconds. Its reason names the folder Claude asked about and the commands to run. This is expected.
Trust the clone, then resume the task:

```sh
cd ~/.local/state/priory-oss/repos/owner/name && claude   # accept the dialog, then /exit
priory resume 1
```

Every later task in this repository starts without the dialog.

## 6. Answer the first ask

When the agent finishes, it reports the outcome `passed`. The task moves to `approve-pr`, a human
step. The agent's process ends, and the Desk in the web view shows an ask with the options `open`
and `stop`.

Open the ask, read what the agent did, and choose `open`. From a terminal, the same answer is:

```sh
priory asks
priory answer 1 open --revision 1
```

`priory asks` prints each ask with its id and its revision, and `--revision` must name that
revision, 1 for an ask nobody changed.

Your answer approves the step `open-pr` for the branch's current commit. That step pushes the
branch and opens a pull request in your repository. The task then waits, with no process, for the
pull request's checks: `priory tasks` shows it `asleep` at `implement`, the step whose agent
fixes what the checks find, and `priory waits` shows its `checks` wait on the pull request.
Green checks mark it done. A red check wakes the agent with the failure, in the same conversation.

When a step fails, `priory actions` shows the error of a code step such as `open-pr`, and
`priory task 1 --json` prints everything about the task.

## 7. Let a label start a task

The trigger you wrote starts the same workflow from GitHub. Add the label `agent` to an issue of
your repository. On its next poll, the intake sees the label, and `priory tasks` lists a new task
with a key such as `github:owner/name#42`. It goes through the same steps.

## Next steps

- Start from a ready-made factory: [How to start from a starter factory](docs/starting-from-a-starter.md).
- Install the release binary or a Nix build: [How to install Priory](docs/installing-a-release.md).
- Answer asks from a phone, with notifications: [How to use the web view](docs/using-the-web-view.md).
- Keep both processes running: [How to run Priory as services](docs/running-as-services.md).
- Try a factory with fake agents before a real one runs: [How to test a factory end to end](docs/testing-a-factory.md).
- Look up a command: [the command reference](docs/reference/cli.md). `priory` with no arguments
  prints its usage.
- Look up a key of a factory file: [the reference](docs/reference/README.md).
- Understand what the daemon does with a task: [Concepts](docs/concepts.md), then
  [A task's life](docs/daemon/README.md#a-tasks-life).
- Stop an agent's irreversible command, or make an outcome need proof: [the tool guard](docs/daemon/guard.md)
  and [proofs](docs/daemon/tools.md#proofs).
- Install and build each new worktree before its agent starts, and clean it up before the sweep
  removes it: [recipes](docs/daemon/runs.md#recipes).
- Give every agent the same writing guide, facts and skills: [writing.md](docs/daemon/writing.md) and
  [skills.md](docs/daemon/skills.md).
- Run one factory on two machines: [How to add a second machine](docs/adding-a-second-machine.md).
- Work from Herdr or from GitHub's pull request page:
  [How to install the Herdr adapter](docs/installing-the-herdr-adapter.md) and
  [the GitHub extension](docs/adapters/github.md).
- Queue a Slack thread from Slack: [the message shortcut](docs/intake/slack.md#message-shortcut).
