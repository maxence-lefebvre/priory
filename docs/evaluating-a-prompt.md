# How to evaluate a prompt change

This guide shows how to measure a change to text that Priory gives agents, such as a tool
description, the nudge or a task intro, before you ship it. It assumes a clone of this repository
with its [build prerequisites](../GETTING_STARTED.md#1-build-priory), and Claude Code signed in.

`priory eval` runs each case of an evaluation file several times under each variant of the text,
with a real model. The agent's only tools are a fake of Priory's MCP server: it serves the same
tools, logs every call and answers `ok`, so nothing reaches a daemon. The command counts what the
agent did after the prompt under test, prints a table, and writes it into the doc of the behaviour.

It calls a real model and costs money, so it runs only when you ask, never in CI.
[prompt-evals.md](reference/prompt-evals.md) lists every key and flag.

## 1. Write the cases beside the prompt

Keep the evaluation file beside the text it tests, named `<prompt>.eval.toml`. The nudge's is
[`crates/priory-core/src/nudge.eval.toml`](../crates/priory-core/src/nudge.eval.toml), beside
`nudge.txt`.

Give two variants: `old`, which reads the shipped text from its file, and `new`, the text you
propose. Give each case the turns that set the scene in `before`, the prompt under test with
`{prompt}` where the variant's text goes, and what a good run does in `expect`, such as
`result passed`, `ask` or `wait checks`.

## 2. Mark the table's place in the doc

Name the doc in `doc`, and add these two lines where the table goes:

```markdown
<!-- priory eval nudge.eval.toml -->
<!-- /priory eval -->
```

## 3. Try one case cheaply

Run one case once on Haiku, with a cap of ten cents:

```sh
cargo run -p priory -- eval crates/priory-core/src/nudge.eval.toml --case finished --runs 1 --budget 0.10
```

One run of a nudge case costs under a cent on Haiku. A run with `--case` prints its table and leaves
the doc alone.

## 4. Run every case

```sh
cargo run -p priory -- eval crates/priory-core/src/nudge.eval.toml --runs 5 --budget 1
```

The command stops starting runs once it has spent the budget. When every case ran within it, the
command replaces the lines between the two markers with the new table. Commit that table with your
change, and name the model and spend in the pull request.
