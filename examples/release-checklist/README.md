# Release checklist

A runbook: a workflow made only of human steps, so no agent runs. Each step is an ask on your desk.
The `test` step carries a command: answering `run` runs `cargo test` once in your project's
checkout, then moves to `tag`. If the tests fail, or Priory stops while they run, the desk asks the
`test` step again with the exit or the uncertain outcome.

## What it needs

`cargo` on the daemon's `PATH` and `PRIORY_DB` set for the intake and the daemon. Point the `repo`
target in `factory.toml` at your project's checkout, replace `owner/name` in `triggers/cli.toml`,
and rename `runners/my-machine.toml` after your machine's Tailscale hostname.

## Gates

- No step is irreversible. `cargo test` only reads and builds.
- The command runs only on an answer that confirms it: `priory answer --run`, or the run option in
  a view.

## Run it

```sh
cp -r examples/release-checklist my-factory
priory check my-factory
export PRIORY_DB=~/.local/state/priory-oss/priory.db
priory daemon my-factory &
priory add my-factory "Release 1.2.0"
priory desk                   # the changelog step's ask
priory answer 1 written --revision 1
priory answer --run 2 run --revision 1
```
