# Design debate

When you label an issue `design-debate`, two agents judge the design it links to: `architect` on
Claude Code and `skeptic` on Codex. Each answers on its own first, then they answer each other for
up to three rounds. When they agree, the task ends with their verdict and the whole exchange as its
evidence. When they do not, Priory asks you to pick.

## What it needs

`claude` and `codex` on `PATH`, `gh` logged in to GitHub, and `PRIORY_DB` set for the intake and the
daemon. Replace `owner/repo` in `triggers/design-label.toml`, and rename `runners/my-machine.toml`
after your machine's Tailscale hostname. Set each agent's `model` and its price in `prices.toml`:
the debate's cap counts every turn at its own model's price.

Codex runs only hooks you trusted. Before the first task, trust Priory's hooks in the clone once, as
[codex.md](../../docs/daemon/codex.md) says.

## Gates

- The `debate` step is `read`: the agents read the repository and change nothing. Their only way to
  end a turn is the `position` tool, with a verdict of `build`, `rework` or `undecided`.
- The debate costs at most about `$3.00`. It can pass the cap by one turn, since a turn's tokens
  arrive when it ends.
- A split, a spent cap or a silent turn asks you. Your pick ends the task, and the step's record
  names you as its decider.
- No step after the debate changes anything. To act on `build`, add the steps you want after it, with
  a human step before any irreversible one.

## Run it

```sh
cp -r examples/design-debate my-factory
priory check my-factory
export PRIORY_DB=~/.local/state/priory-oss/priory.db
priory intake my-factory    # keeps polling: run the daemon in a second terminal
priory daemon my-factory
```

Then label an issue `design-debate`. `priory task <id> --json` shows the verdict and the transcript
once the debate ends. See [debates.md](../../docs/daemon/debates.md) for how the daemon runs it.
