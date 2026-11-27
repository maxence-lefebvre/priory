# A cloud run reports through the gateway, and its branch push carries its code

Date: 2026-11-26. Status: accepted. Amends
[ADR 0005](0005-a-cloud-run-reports-by-pushing-a-branch-the-daemon-polls.md).

## Context

ADR 0005 made a branch push a cloud run's only channel: the daemon polls `origin` and stores the
outcome `pushed`. A cloud step therefore has two outcomes, cannot ask a person, and cannot wait for
an event. The diocese now runs a gateway that a cloud session reaches as an HTTP MCP server, with a
run grant as its bearer token ([the diocese design](../design/diocese.md)). It hands each call to the
runner that owns the task through the log.

## Decision

With `[cloud] gateway = true`, each cloud run gets a run grant, and its session calls `ask`, `wait`
and `result` through the gateway. The runner that made the grant applies each call once, as a call
on its socket. A `result` names the step's outcome, summary and evidence.

The code still arrives by the branch push. A `result` does not end the step by itself: the daemon
looks for the push, takes it as ADR 0005 says, and stores the result with the pushed commit. When
the branch has no new commit, the step ends with the result and no commit.

## Alternatives

- **Push the code through the gateway too.** The gateway would carry patches to a runner, and the
  runner would apply code nobody pushed to GitHub. The push already reaches GitHub, which the
  session can write to and the runner can read.
- **A result file on the branch.** ADR 0005 named it. It needs no gateway, but still gives no
  `ask` or `wait`, and a person could not answer during the session.

## Consequences

- A cloud step may route any outcome, and `priory check` allows it only with the gateway on.
- The gateway is a second channel to keep up. When it is down, a cloud run still ends by its push,
  so ADR 0005's channel is the fallback.
- A session holds a token. A push whose commits hold it fails the step, and the grant is revoked.
- `[cloud] gateway = false`, the default, keeps ADR 0005 exactly.
