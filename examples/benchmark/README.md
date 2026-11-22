# Benchmark

A factory that compares a change to itself before it merges. Its `implement` workflow is the work:
an issue labeled `agent` in `owner/repo` gets a change, and Priory opens the pull request once you
approve. When someone adds the label `benchmark` to a pull request on the factory's own repository,
`owner/factory`, Priory asks you whether to compare it. Answering `run` replays past `implement`
tasks under the pull request's base and head, and posts the table on the pull request.

| Step | What happens |
| ---- | ------------ |
| `compare` | Your desk asks `Compare pull request 12 at <commit> on the implement benchmark, for at most $40?`. `run` runs `priory bench` once, `skip` drops the task. |

The comparison writes its table as a comment, and sets the status `priory/benchmark` on the head
commit: `success` unless the comparison is incomplete or its primary scorer reads `worse`. See
[bench.md](../../docs/daemon/bench.md#before-a-change-merges).

## What it needs

- This folder as the root of a Git checkout of `owner/factory`, whose `origin` holds its pull
  requests. The command runs in it, from the `factory` target of `factory.toml`.
- `gh` logged in with access to both repositories, `claude` on `PATH`, `priory` on the daemon's
  `PATH`, and `PRIORY_DB` set for the intake and the daemon.
- One task of `owner/repo` run on this machine, so Priory holds its clone, and past tasks to
  capture. Capture each into `benchmarks/cases/`, read it, list it in
  `benchmarks/implement.toml`, and commit it:

  ```sh
  priory bench capture . 41 --into benchmarks/cases/first
  ```

- Replace `owner/repo` in `triggers/agent-label.toml` and `owner/factory` in
  `triggers/benchmark.toml`, add a price to `prices.toml` for each model your agents name, and
  rename `runners/my-machine.toml` after your machine's Tailscale hostname.

## Gates

- A label runs nothing. A bot or an agent can add one; only your answer starts the comparison.
- The ask names the commit, and the command takes that commit, so a push after your answer
  changes nothing of what runs.
- A pull request from a fork, or one whose event does not name its head repository, is dropped
  with the reason: the comparison runs the head's prompts and scripts with your sign-in.
- The budget, `--budget 40`, is in `workflows/benchmark.toml`, reviewed like any change. The
  command prints its estimate first and refuses one over the budget, which asks the step again.
- `publish` is irreversible, and a replay ends before it and never runs it.
- To compare again, remove the label and add it again. A second comparison of the same commits is
  a repeat: it posts its table and leaves the status as it is.

## Run it

```sh
cp -r examples/benchmark my-factory
priory check my-factory
export PRIORY_DB=~/.local/state/priory-oss/priory.db
priory intake my-factory    # keeps polling: run the daemon in a second terminal
priory daemon my-factory
priory desk                 # once a pull request is labeled benchmark
priory answer --run 1 run --revision 1
```
