# Cloud runs

This note designs how the daemon hands a model step to a Claude cloud session, which runs on
Anthropic's machines, so a slow or shared desktop only coordinates. It holds what the CLI allows,
the model, the first slice and the slices after it. [cloud.md](../daemon/cloud.md) explains the
first slice as built, and
[ADR 0005](../adr/0005-a-cloud-run-reports-by-pushing-a-branch-the-daemon-polls.md) records how a
cloud run reports.

## What the CLI allows

Checked against Claude Code 2.1.331 and its docs on 2026-11-24
([Claude Code on the web](https://code.claude.com/docs/en/claude-code-on-the-web),
[cloud environments](https://code.claude.com/docs/en/cloud-environments)):

- `claude --cloud PROMPT` creates a session from the current folder's GitHub remote and branch. It
  needs a terminal. With piped output it prints `--cloud requires an interactive terminal`, and
  with `-p` it prints `--cloud cannot be combined with --print`. In a terminal it prints `Created
  cloud session: <title>`, `View: https://claude.ai/code/session_<id>?...` and `Resume with: claude
  --teleport session_<id>`, then exits 0 within seconds.
- `claude -p MSG --cloud ID --output-format json` queues a message into a session and prints
  `{ok, session_id, url}`.
- No command reads a session's state or transcript, lists sessions, or stops or archives one.
- `--cloud` takes one repository. A session clones the GitHub remote, not the local checkout. One
  whose GitHub connection could not reach the repository started anyway, with the files and no
  `origin`, and could not push.
- `claude --cloud` shows the trust dialog in a folder Claude does not trust, and refuses a linked
  worktree under a temporary folder.
- Inside the session, a GitHub proxy allows branch pushes only, and blocks GraphQL, so `gh pr
  create` fails.
- Cloud environments are made and edited only in the web UI. `--environment` takes self-hosted
  `ccpool_` ids only. The CLI uses the environment `/remote-env` saved as the default. A missing id
  passed through `--settings '{"remote":{"defaultEnvironmentId":...}}'` was ignored without an
  error.
- An environment's setup script runs as root before Claude starts, and the environment caches what
  it made for about seven days. A `~/.claude/CLAUDE.md` it writes reaches the session as user
  instructions. The docs name no working folder for it.
- Usage counts against the person's Claude plan. No per-session usage is exposed.

## The model

An agent with `harness = "claude-cloud"` runs each of its steps as a cloud run. A cloud run is a
row in `runs`, with a worktree and a branch like any run, and no local process. The harness value
belongs to the agent, as `claude-code` and `codex` do, so one workflow mixes local and cloud steps.

| Priory duty | Local run | Cloud run |
| ----------- | --------- | --------- |
| Start | the harness in tmux, in the worktree | `claude --cloud PROMPT` in tmux, in the worktree, for the seconds it takes |
| Room | the agent's `memory` and `disk` | none |
| Prompt | typed when idle, confirmed by a hook | passed on the command line, confirmed by the printed id |
| Result | the `result` tool | the daemon stores `pushed` when the session's branch arrives |
| Question to a person | the `ask` tool | none: questions go in the commit message |
| Tool guard and permissions | Priory's hooks and settings | the cloud environment's |
| Skills and instructions | the plugin folder and system prompt | `~/.claude`, written by the environment's setup script |
| Tokens | the transcript | unknown |
| Health | exit, timeout, permission prompt | the step's `timeout` |

## The first slice

On `main` with this note:

1. `harness = "claude-cloud"`, and `priory check` rules: a cloud agent takes no `session =
   "shared"`, `mcpServers`, `connectors`, `permissions`, `memory`, `disk` or `compactAt`. A cloud
   step takes no `proof`, `guard`, `plan_requires` or `gate`, and routes only `pushed` and `failed`.
   A debate names no cloud agent. Handler, summarizer, lead and investigator already need
   `claude-code`.
2. Dispatch: the worktree, the recorded run, the prompt with the push rules, `claude --cloud` in
   tmux, and the session id in the run's `thread` column, which `priory runs` shows.
3. The branch poll with `git ls-remote`, the result `pushed` with the head commit, and the turn end,
   so `open-pr` behind a person's approval opens the pull request.
4. The deadline, and a failed start, as outcome `failed`. A daemon stopped during dispatch blocks
   the task and never dispatches the run again.
5. No room for a cloud run, and no health verdict on it.
6. `[repos."owner/name"] cloud_setup` and `priory cloud setup-script`.

## Later slices

1. **Asks through messages.** Turn `Questions:` in the pushed commit into the human step's ask
   text, and send a person's answer back with `claude -p MSG --cloud ID`, then wait for a new push.
2. **Feedback.** Send review comments and red checks on the pull request with `claude -p`, through
   the outbox's states, and poll for the next push.
3. **A spend cap.** `[cloud] max` bounds the live cloud runs, since room does not.
4. **A merge gate.** The session holds the person's GitHub access. Before a dispatch, check that the
   default branch's rules refuse a merge without a status Priory sets.
5. **Drift.** Record the pushed branch's files for the overlap hold, and check that it merges into
   the default branch before the human step.
6. **Branch cleanup.** Delete `claude/priory-*` branches once `open-pr` pushed the task's own.
7. **A silent session.** One nudge with `claude -p` before the deadline fails the step.
8. **Several repositories and the kit.** A session with the kit repository beside the task's, once
   the CLI can start one. Until then the setup script carries the kit.
9. **Usage.** Tokens per session, once the CLI or an API reports them.
10. **A dispatch off the tick.** Run `claude --cloud` off the daemon's thread, if a slow start
    delays other runs.

## Open questions

1. **Does a session keep the branch name?** The prompt names `claude/priory-<task>-<token>`.
   The first real session used that name in its `git push`, which failed for lack of access. Recommendation: check it again with access, and fall back to a token in the commit message
   with a poll of `claude/*` branches if sessions rename it.
2. **In which folder does the setup script run?** The docs do not say. Recommendation: the script
   `priory cloud setup-script` prints embeds each `cloud_setup` script's text, so it does not
   depend on the folder or the clone.
3. **How does a missing GitHub connection show early?** The session starts and only its push fails,
   so the step waits for its deadline. Recommendation: a doctor check that pushes a branch by hand,
   as the how-to does, until the CLI reports access.
4. **Which environment does a dispatch use?** The CLI's default. Recommendation: one environment
   per person for now, and per-repository environments once the CLI takes an `env_` id.
