# Getting started

In this tutorial, you build Priory, write a factory with one agent, one workflow and one trigger,
run it, and answer an agent's first question in the web view on your own machine. By the end, an
agent has changed a repository of yours and opened a pull request that you approved.

## Before you start

You need:

- Linux. Priory is built and tested on Linux only. With a systemd user session, every agent runs
  in a slice that caps its CPU and memory; without one, agents still run (see
  [agents-slice.md](docs/daemon/agents-slice.md)).
- On `PATH`: `git`, `tmux`, `gh` logged in to GitHub, and `claude`, logged in. The agent of this
  tutorial runs Claude Code. To run Codex instead, set `harness = "codex"` in step 2 and read
  [codex.md](docs/daemon/codex.md).
- The Rust toolchain. `rust-toolchain.toml` pins the version, and `rustup` installs it on the first
  build.
- A GitHub repository you can push to, for the agent to work in. This page calls it `owner/name`.

Five words come back on every page. [Concepts](docs/concepts.md) explains them in full.

- A **factory** is a folder of files that says which agents exist and what they do.
- A **task** is one unit of work, such as one issue to fix.
- A **step** is one stage of a task. A model, a person or code decides its outcome.
- An **ask** is a question an agent, or a step, puts to a person.
- The **desk** is the one list of everything that waits on a person.

## 1. Build Priory

```sh
git clone https://github.com/maxence-lefebvre/priory
cd priory
cargo build --release
export PATH="$PWD/target/release:$PATH"
priory --version
```

The last command prints the version, such as `priory 0.1.0`.

### Install a release instead

`install.sh` installs the binary of the latest
[release](https://github.com/maxence-lefebvre/priory/releases) to `~/.local/bin`. When that page
lists no release, the script fails with a 404: build from source.

```sh
curl -fsSL https://raw.githubusercontent.com/maxence-lefebvre/priory/main/install.sh | sh
```

The script takes the binary for Linux x86_64, Linux aarch64 or macOS on Apple silicon, and checks
it against its `.sha256`. The `.sha256` sits beside the binary in the same release, so it only
catches a broken download. When [gh](https://cli.github.com/) is on your `PATH` and signed in, the
script also runs `gh attestation verify`, which proves this repository's release workflow built the
binary, and refuses a binary it cannot verify. Without gh, it prints a note and skips that check.
Run `sh install.sh --prefix /usr/local` from a clone to install to `/usr/local/bin`. Set
`PRIORY_RELEASE_URL` to a release's download address to pick a release other than the latest. With
[cargo-binstall](https://github.com/cargo-bins/cargo-binstall),
`cargo binstall --git https://github.com/maxence-lefebvre/priory priory` downloads the same binary.
Each release also holds `priory-herdr.tar.gz`, the [Herdr adapter](adapters/herdr/), and
`priory-github-extension.zip`, the [GitHub extension](adapters/github/README.md).

The release binaries leave out the cargo features. A build from source can add them: the
in-process embedding model for search with `--features model`, and the Postgres event log, which
lets several machines share one log, with `--features postgres` (see
[`[log]`](docs/reference/factory.md#log)).

## 2. Write a factory

A factory is a folder of files. Make an empty working folder outside your clone, then create
these four files in an `example/` folder inside it.

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

`priory check --explain example` lists the irreversible step `open-pr` and the human step
`approve-pr` that approves it. `priory check` refuses a workflow that reaches an irreversible step
with no person's approval.

## 4. Run the daemon

The daemon turns requests into tasks and runs the agents. In a first terminal, from your working
folder:

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

Open `http://127.0.0.1:7340` in a browser on the same machine and paste the token. The web view
opens on the Desk, which is empty. The server listens on this machine only.

## 5. Ask for a first task

In a second terminal, from your working folder, with your repository in place of `owner/name`:

```sh
export PRIORY_DB=~/.local/state/priory-oss/priory.db
priory add example --repo owner/name "Fix the typo in the README"
priory tasks
```

`priory add` prints the task's key, such as `cli:2026-11-20-1`. Within a second the daemon makes
task 1 from it, and `priory tasks` lists it at the step `implement`. Its state is `queued` until
the machine has room for the agent (see [room.md](docs/daemon/room.md)), then `running`.

The daemon clones your repository under `~/.local/state/priory-oss/repos/`, adds a worktree on the
branch `priory/1`, and starts `claude` in the tmux session `priory-1`. To watch the agent work,
open the task in the web view, or run `priory attach 1 --read-only`.

### Trust the clone, once

On the first run in a repository, Claude asks whether it trusts the folder, and Priory never
answers that for you. After 60 seconds the task turns `blocked`, and its reason tells you what to
do. This is expected. Trust the clone, then resume the task:

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
pull request's checks. Green checks mark it done. A red check wakes the agent with the failure, in
the same conversation.

When a step fails, `priory actions` shows the error of a code step such as `open-pr`, and
`priory task 1 --json` prints everything about the task.

## 7. Let a label start a task

The trigger you wrote starts the same workflow from GitHub. It needs the second process, the
intake, which polls GitHub every 60 seconds. In a third terminal, from your working folder:

```sh
export PRIORY_DB=~/.local/state/priory-oss/priory.db
priory intake example
```

Add the label `agent` to an issue of your repository. Within a minute, `priory tasks` lists a new
task with a key such as `github:owner/name#42`, and it goes through the same steps.

## Next steps

- Start from a ready-made factory: [How to start from a starter factory](docs/starting-from-a-starter.md).
- Answer asks from a phone, with notifications: [How to use the web view from a phone](docs/using-the-web-view.md).
- Keep both processes running: [How to run Priory as services](docs/running-as-services.md).
- Try a factory with fake agents before a real one runs: [How to test a factory end to end](docs/testing-a-factory.md).
- Look up a command: [the command reference](docs/reference/cli.md). The ones you need most are below.
- Look up a key of a factory file: [the reference](docs/reference/README.md).
- Understand what the daemon does with a task: [Concepts](docs/concepts.md), then
  [A task's life](docs/daemon/README.md#a-tasks-life).
- Stop an agent's irreversible command, or make an outcome need proof: [the tool guard](docs/daemon/guard.md)
  and [proofs](docs/daemon/tools.md#proofs).
- Give every agent the same writing guide, facts and skills: [writing.md](docs/daemon/writing.md) and
  [skills.md](docs/daemon/skills.md).
- Run one factory on two machines: [How to add a second machine](docs/adding-a-second-machine.md).
- Work from Herdr or from GitHub's pull request page: [the Herdr adapter](docs/adapters/herdr.md) and
  [the GitHub extension](adapters/github/README.md).
- Queue a Slack thread from Slack: [the message shortcut](docs/intake/slack.md#message-shortcut).

## Day to day

| Command | What it does |
| ------- | ------------ |
| `priory desk` | Lists what waits on you: asks, blocked and failed tasks, held tasks, the queue |
| `priory answer ASK OPTION --revision N` | Answers an ask, at the revision `priory asks` prints for it |
| `priory tasks` | Lists tasks in start order |
| `priory task TASK --json` | Prints one task with its brief, plan, pull request, runs and asks |
| `priory actions` | Lists each code step's action with its error and output: read it when a code step failed |
| `priory attach TASK [--read-only]` | Attaches your terminal to a task's agent |
| `priory say TASK TEXT` | Sends a task's agent a message, waking it if it sleeps |
| `priory resume TASK` | Resumes a blocked task |
| `priory cancel TASK` | Drops an active task and ends its run |
| `priory revive TASK` | Restarts a failed or dropped task |
| `priory pause [REASON]`, `priory unpause` | Stops new tasks from starting, then lets them start again |

[The command reference](docs/reference/cli.md) lists every command with its flags. Run `priory`
with no arguments to print its usage.
