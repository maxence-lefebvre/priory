<div class="home">
<section class="hero">
<div>

# Keep coding agents moving while nobody watches.

<!-- include README.md "intro" -->

<div class="cta">
<a class="btn primary" href="GETTING_STARTED.html">Get started</a>
<a class="btn secondary" href="#documentation">Read the docs</a>
</div>
</div>
<figure>
<img class="shot" src="docs/images/web-view-desktop.png" alt="The desk on a 1440 px screen in the Indigo theme, dark mode: a rail of views, the list of asks and stopped tasks with what each recommends, and a drawer with the ask's question, recommendation and options.">
<figcaption>The desk in the web view, Indigo theme, dark mode.</figcaption>
</figure>
</section>

<section class="sec">

## Start in three steps

<p class="sub">Priory needs Linux, git, tmux, gh logged in to GitHub, and claude or codex.</p>

<ol class="steps">
<li>

### Install Priory

```sh
curl -fsSL https://raw.githubusercontent.com/maxence-lefebvre/priory/main/install.sh | sh
priory --version
```

The script installs the latest release to `~/.local/bin`. You can also build from source.

[Install Priory](GETTING_STARTED.md#1-install-priory)

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

### Run it

```sh
export PRIORY_DB=~/.local/state/priory-oss/priory.db
priory intake example
```

```sh
priory daemon example
```

Run the intake and the daemon side by side, each in its own terminal with `PRIORY_DB` set. The intake finds work. The daemon runs one agent per task in tmux and a git worktree.

[Run it](GETTING_STARTED.md#5-run-it)

</li>
</ol>
</section>

<section class="sec problem">

## The problem it solves

<!-- include README.md "The problem it solves" -->

</section>

<section class="sec concepts">

## Concepts

<!-- include GETTING_STARTED.md "Concepts" -->

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
