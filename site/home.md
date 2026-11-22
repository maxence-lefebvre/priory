<div class="home">
<section class="hero">
<div>

# Build your own agent factory.

Agentic workflows as files in git, run by a daemon on machines you own, with a person on every irreversible step.

<!-- include README.md "intro" -->

<div class="cta">
<a class="btn primary" href="GETTING_STARTED.html">Get started</a>
<a class="btn secondary" href="#documentation">Read the docs</a>
</div>
</div>
<figure>
<img class="shot" src="docs/images/web-view-desk-ask-1440-dark.png" alt="The desk on a 1440 px screen in dark mode: a rail of views, the list of asks and stopped tasks, and a drawer with the ask 'Open the pull request for ENG-212?', its recommendation and its options.">
<figcaption>The desk in the web view, dark mode.</figcaption>
</figure>
</section>

<section class="sec">

## Start in three steps

<p class="sub">Priory needs Linux, git, tmux, gh logged in to GitHub, openssl, claude or codex, and the Rust toolchain.</p>

<ol class="steps">
<li>

### Build Priory

```sh
git clone https://github.com/maxence-lefebvre/priory
cd priory
cargo install --path crates/priory
```

Build from source with the pinned Rust toolchain.

[Build Priory](GETTING_STARTED.md#1-build-priory)

</li>
<li>

### Write and check a factory

```sh
priory check example
priory draw example implement
```

A factory is a folder: one agent, one workflow and one trigger to start.

[Write a factory](GETTING_STARTED.md#2-write-a-factory)

</li>
<li>

### Run it and answer the first ask

```sh
mkdir -p ~/.local/state/priory-oss
export PRIORY_DB=~/.local/state/priory-oss/priory.db
export PRIORY_TOKEN=$(openssl rand -hex 32)
echo "$PRIORY_TOKEN"
priory daemon example
```

In a second terminal:

```sh
priory add example --repo owner/name "Fix the typo in the README"
```

The daemon runs one agent per task in tmux and a git worktree. Its question lands on the desk, in the web view on `127.0.0.1:7340`, which asks for the token the `echo` printed.

[Run it](GETTING_STARTED.md#4-run-it)

</li>
</ol>
</section>

<section class="sec problem">

## The problem it solves

<!-- include README.md "The problem it solves" -->

</section>

<section class="sec">

## How a task moves

<p>An event becomes a task. A run starts an agent on each model step, and the task sleeps while it waits on the world or on you.</p>

<figure class="diagram">
<img src="docs/images/diagrams/task-flow.svg" width="680" height="652" alt="How a task moves, from top to bottom. GitHub, Linear, Slack and schedules reach the intake, and a webhook or a request you type goes straight to the log. The daemon finds the task each message concerns, and a task request creates a task or joins the active one. A run starts a model step's agent in the task's git worktree and a tmux session. The agent ends its turn with result, wait or ask. A result picks the next step, done or stop. A wait or an ask puts the task to sleep until an event or a person's answer wakes it, and the next run resumes the agent's session.">
</figure>

</section>

<section class="sec concepts">

## Concepts

A factory is a folder of files: agents, workflows and triggers. A trigger turns an event into a task.
A task goes through its workflow's steps, and each step is decided by a model, a person or code.
What waits on a person lands on one list, the desk.

[Read the concepts](docs/concepts.md)

</section>

<section class="sec phones">

## The web view

<!-- include README.md "The web view" -->

</section>

<section class="sec status">

## Status

<!-- include README.md "Status" -->

</section>

<section class="sec docmap">

## Documentation

<!-- include README.md "Documentation" -->

</section>
</div>
