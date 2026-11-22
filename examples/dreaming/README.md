# Dreaming

At 03:00, an agent reads what the day's tasks concluded and what people answered, with
`priory recap`, and proposes at most five memories with `remember`. In the morning, the desk's
`memories` row lists them, and you accept or drop each one. At 06:00, a code step opens one pull
request on your kit repository with the memories you accepted.

At 02:45, a code step runs `priory patterns find`: it groups the day's failed, blocked, repaired and
marked-wrong tasks into failure patterns, and the desk shows each pattern that holds two tasks. It
runs no model.

## What it needs

`claude` on `PATH`, `gh` logged in with push access to the kit repository, and `PRIORY_DB` set for
the intake and the daemon. The agent runs `priory recap`, so put Priory's binary on the daemon's
`PATH`. Replace `owner/abbey` in `factory.toml` and every trigger with your kit repository, and
rename `runners/my-machine.toml` after your machine's Tailscale hostname.

`prices.toml` prices `claude-sonnet-5-5`, the model `sonnet` names today. The dream step's
`budget` needs the price of the model the transcript names: a model it cannot price fails the task.

## Gates

- The dream workflow has no human step and no irreversible step, so it asks you nothing at night.
  Its only writes are candidates, which reach no other task until you accept them.
- The agent may run one command, `priory recap`. `dontAsk` denies every other call that would need
  a permission, and `deny` takes `ask`, `wait` and the other `priory` tools it has no use for.
- `budget = "1.00"` fails the task once a turn ends over a dollar. The next night starts a new one.
- `remember` refuses the text of a candidate you dropped, and the recap names it.
- `open-memories-pr` is irreversible and `autonomous`: you review its pull request before any
  memory merges.

## Limits

The dream reads the state file of the runner that wins its task. In a factory of several runners,
it covers that runner's tasks.

## Run it

```sh
cp -r examples/dreaming my-factory
priory check my-factory
export PRIORY_DB=~/.local/state/priory-oss/priory.db
priory intake my-factory    # keeps polling: run the daemon in a second terminal
priory daemon my-factory
```

See [How to set up a nightly dream](../../docs/setting-up-a-nightly-dream.md) to add it to a factory
you already run.
