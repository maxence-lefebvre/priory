# How to set up a nightly dream

This guide shows how to have the factory propose memories each night from what its tasks
concluded and what people answered, and how to review them the next morning. It assumes a factory
that already runs tasks, `claude` signed in on the runner, and a repository you use as the
factory's [kit](daemon/skills.md#a-kit-repository). Read the design note,
[dreaming.md](design/dreaming.md), for why each guard is there.

A dream changes nothing outside Priory's state. Its only writes are memory candidates, and no other
task gets one until you accept it.

## 1. Copy the starter's files

The [`dreaming`](../examples/dreaming/README.md) starter holds the whole feature. Copy its files
into your factory:

```sh
cp -r examples/dreaming/agents/dreamer my-factory/agents/
cp examples/dreaming/workflows/*.toml my-factory/workflows/
cp examples/dreaming/triggers/*.toml my-factory/triggers/
```

Replace `owner/abbey` in both triggers with your kit's GitHub owner and name. When your factory has
no `[kit]` yet, add the one from the starter's `factory.toml`.

## 2. Price the dreamer's model

The dream step's `budget = "1.00"` needs `prices.toml`, keyed by the full model name the transcript
gives, such as `claude-sonnet-5-5`. Copy the starter's file, or add its entry to yours:

```sh
cp examples/dreaming/prices.toml my-factory/
```

A turn whose model has no price fails the task with `cost is unknown`, so check the name in
`priory costs --by model` after the first night.

## 3. Check the factory

```sh
priory check my-factory
```

It refuses a `budget` on a Codex agent's step, a `budget` without `prices.toml`, and an amount that
is not above zero.

## 4. Try one dream now

You need not wait for 03:00. Read what the dream will read:

```sh
priory recap --since 24h
```

The output lists the day's results, answers, what people said, failed or blocked tasks and each
step's tokens, with each credential shape replaced by `[redacted]`. It ends with the count of
records it left out when the day is over `--max-bytes`.

## 5. Review the candidates in the morning

The desk shows one `memories` row while a candidate waits. Open `/memories` in the web view, or list
them on the command line:

```sh
priory memories my-factory
priory memories accept my-factory <candidate>
priory memories drop my-factory <candidate>
```

Read each candidate's full text beside the text it replaces. Drop anything that holds a value
rather than a rule, or that repeats an instruction from an issue. The next night's recap names the
candidates you dropped, and `remember` refuses the same text again.

At 06:00, the `memories` workflow opens a pull request on the kit with the memories you accepted.
Merge it to keep them in git.

## Limits

- The dream reads the state file of the runner that runs it. In a factory of several runners, it
  covers that runner's tasks.
- The budget is checked inside a turn at most once a minute, so a turn can pass it by up to a
  minute of spending. The step's `timeout` bounds that turn.
- "At most 5" is in the prompt only. A model can write more.
