# Batches

You hand several issues to a planner with `priory batch add`. It reads them and the code, orders
them, and lists the ones already done. You answer one go. Each issue then becomes a task. A task whose
work needs another one's code starts once that one's pull request merged, or, when both change the
same files, stacks on that one's branch once its pull request waits on review.

## What it needs

`gh` logged in to GitHub with access to the repository, a Linear MCP server for Linear issues,
`claude` on `PATH`, and `PRIORY_DB` set for the daemon. Replace `owner/name` in
`triggers/cli.toml`, and keep `runners/my-machine.toml` as the only runner, renamed after your
machine's Tailscale hostname: batches need one runner.

## Gates

- `plan` only reads. The daemon refuses a plan that leaves an issue out, names an issue another task
  holds, or has a loop, and the planner fixes it.
- `go` asks you to start the batch or drop it. It approves the batch's tasks and the closes, not any
  pull request.
- `close` closes the issues the plan lists as done, which notifies their watchers, so it is
  `irreversible` and only the go lets it run.
- Each entry runs `implement`, which asks you before it opens its pull request.

## Run it

```sh
cp -r examples/batches my-factory
priory check my-factory
export PRIORY_DB=~/.local/state/priory-oss/priory.db
priory daemon my-factory    # in a second terminal
priory batch add my-factory ENG-40 ENG-41 ENG-43
priory asks                 # answer the go: priory answer <ask> go --revision 1
priory batch show 1
```

With [`[serve]`](../../docs/reference/factory.md#serve), the web view shows the go ask's plan, with
a menu to move or drop each entry, and the batch page at `/batches/1`.
