# The diocese

This note designs the diocese: a small rented VM that is always on and holds what every runner
shares. A diocese oversees many priories, as this VM serves the Priory daemon on every runner. It
holds the log, the intake, the trace store and a public MCP gateway that cloud runs reach through
a claude.ai connector. Two runners, the desktop and a second machine, keep running the agents. The
note builds nothing. It holds what exists, the target, its security and failure modes, the VM's
size and cost, the open questions with a recommendation each, and six slices that each ship alone.

## What exists today

- **Runners own tasks through the log.** Each runner reads every message of the log, and the first
  claim of a key in the log wins it. Each daemon publishes `priory.runner.alive` once a minute, and
  a runner's lease ends 30 minutes after its last one. Only a `waiting` task moves to another runner
  when the lease ends; a `kept` task stays. On Postgres, each message takes the time the database
  stored it, so leases compare one clock
  ([runners.md](../daemon/runners.md#holds-and-leases),
  [ADR 0003](../adr/0003-runners-own-tasks-through-claims-and-leases-on-the-shared-log.md)). A
  daemon reads the log on its tick, once a second.
- **Each runner keeps its own state.** Its tasks, asks, runs and waits live in its own state file,
  `PRIORY_DB`, next to its tmux server and worktrees ([runners.md](../daemon/runners.md#every-runner-reads-the-whole-log)).
- **The log lives on the desktop.** It is a Postgres container bound to `127.0.0.1`. The second
  machine reaches it through a reverse ssh tunnel that the desktop starts, so nothing listens
  beyond loopback on either machine ([adding-a-second-machine.md](../adding-a-second-machine.md)).
  The log's password is also the power to approve on every runner, since a message on the log is
  applied as a person's answer. When the desktop sleeps, the log is gone for both runners.
- **A runner cut off from the log keeps going.** Its started tasks run on, no `waiting` task
  starts, and its claims, holds and alive messages queue until the link is back
  ([runners.md](../daemon/runners.md#a-runner-cut-off-from-the-log)).
- **The intake runs on a runner.** `priory intake` polls GitHub, Linear and Slack and publishes
  events ([intake/README.md](../intake/README.md)). It reads the waits, holds and subscriptions it
  watches from the state file that `PRIORY_DB` names, so it watches one runner's waits
  ([watches.md](../intake/watches.md)). Webhook deliveries reach the daemon's server instead, and a
  trigger's `pre` script runs on the machine that took the event ([pre.md](../intake/pre.md#where-it-runs)).
- **The server listens on loopback.** `priory serve` and the daemon's `[serve]` refuse a
  non-loopback `listen` unless `public = true`, speak plain HTTP and check one bearer token,
  `PRIORY_TOKEN`. A phone reaches one machine through `tailscale serve`
  ([serve.md](../daemon/serve.md#beyond-this-machine)).
- **Tools reach the daemon over its socket.** A run's MCP server, `priory mcp --session <id>`, is a
  stdio process that sends each tool call to the daemon's Unix socket. `priory mcp --outside`
  serves the desk to a person's own session ([tools.md](../daemon/tools.md)).
- **A cloud run reports only by a branch push.** A Claude cloud session cannot reach the socket, so
  it has no `ask`, `wait` or `result`. The daemon polls `origin` for the run's branch and stores the
  outcome `pushed`. The branch name is all that ties a push to its run
  ([cloud.md](../daemon/cloud.md), [cloud-runs.md](cloud-runs.md),
  [ADR 0005](../adr/0005-a-cloud-run-reports-by-pushing-a-branch-the-daemon-polls.md)).
- **Local runs load no connector.** Priory starts each Claude Code run with `--strict-mcp-config`,
  under which Claude Code loads none of the account's claude.ai connectors. An agent's
  `connectors` key drops the flag for the connectors it names
  ([claude-ai-connectors.md](claude-ai-connectors.md)). That note says a connector follows the
  person's Claude login to every machine and cloud session, but no test has checked that a session
  that `claude --cloud` starts loads it. Open question 1's test settles it.
- **R8.5 is partly built.** Every runner's asks and tasks show on every desk, actions on another
  runner's task travel through the log, and `priory daemon --view-only` serves them from a machine
  with no runner file ([runners.md](../daemon/runners.md#a-machine-that-only-views)). A task's
  detail and runs still show only on its runner, and no view is served from a machine that stays on
  ([views-from-any-machine.md](views-from-any-machine.md#later-slices)).
- **Traces leave by OTLP.** The daemon sends its step records as spans to the `otlp` URL in
  `[telemetry]`, `http://127.0.0.1:4318` in the examples ([trace.md](../daemon/trace.md)).

## The target

### What runs where

| Part | The diocese VM | Each runner |
| ---- | ---------- | ----------- |
| The log | Postgres, on the private network address only | a client, through `PRIORY_LOG_URL` |
| The intake | `priory intake`, with read-only GitHub, Linear and Slack tokens, and the Slack shortcut's tokens | none |
| Views | `priory daemon --view-only`, its web view on the private network | its own daemon and web view, as today |
| The gateway | `priory gateway`, behind a TLS proxy on port 443 | none |
| Traces | OpenObserve, on the private network address only | the daemon's OTLP export points at it |
| Daemon, tmux, worktrees, state file | none | as today |
| GitHub writes: push, comment, pull request, merge | none | `gh`, signed in on the runner |
| Harness logins: `claude`, `codex` | none | as today |

The diocese runs no agent. Each runner keeps its daemon, tmux server, worktrees and state file, so a
task's runs, sessions and terminal stay where they are today.

### The network

The three machines join one Tailscale tailnet. The Runner concept already names a runner by its
Tailscale hostname, and `tailscale serve` already carries the web view to a phone. Plain WireGuard
works too, with the peers' keys kept by hand.

- Postgres listens on the diocese's tailnet address only, and its `pg_hba.conf` accepts that range only.
  The runners reach it directly, and the reverse ssh tunnel goes away.
- The tailnet's access rules let each runner reach the diocese's Postgres, OTLP and web view ports, and
  let the diocese reach no runner. A runner opens every connection.
- The VM's provider firewall opens port 443 only, for the gateway. ssh is reachable on the tailnet
  only.

### An MCP call from a cloud session

The gateway is the first part of Priory that a machine outside the factory reaches. It never calls
a runner. It hands each call to the runner that owns the task through the log, as `priory answer`
and `priory give` already hand writes to another runner.

1. Before a cloud run starts, its runner makes a run grant: a random token, the task, the run, the
   runner and a deadline. It publishes `priory.run.granted` with the token's hash, and puts the
   token in the prompt. The token itself never reaches the log, a trace or a file the runner keeps.
2. The session calls `ask`, `wait` or `result` on the connector, with the token as an argument.
   claude.ai sends the call to the gateway with the person's OAuth access token.
3. The gateway checks the access token, then finds the grant by the token's hash. The gateway reads
   the log as its own consumer, `priory.gateway`, and keeps the grants, claims and replies it needs
   in a state file of its own. It refuses a grant that is unknown, past its deadline or revoked, and a tool
   other than `ask`, `wait` and `result`.
4. It reads the task's claim. When the owner is not the grant's runner, it refuses the
   call.
5. It publishes `priory.run.called` with a call id, the grant, the tool and its arguments, and waits
   for the reply.
6. The owner reads the call on its next tick, within about a second. It applies it in one
   transaction, through the same checks as a call on its socket, and publishes `priory.run.replied`
   with the call id and the tool's text or error.
7. The gateway reads the reply from the log and answers the session. With no reply within 40
   seconds, the same bound a run's MCP server waits on the socket, it answers that the runner is
   away. The call id makes a retry apply once.

`ask` and `wait` end the session's turn, as they end a local run's turn. When the answer or the
event arrives, the runner wakes the session with `claude -p MSG --cloud ID`, as
[cloud-runs.md](cloud-runs.md#later-slices) plans for asks. `result` ends the step. The runner
still takes the code from the run's pushed branch, so the branch push of ADR 0005 stays the way
code arrives. `result` adds outcomes other than `pushed`, so a cloud step can route them as a
local step does, and `priory check` stops limiting a cloud step to `pushed` and `failed`.

### A task that moves

A run grant names one runner. A cloud run's task is `kept`, so it moves only when a person gives it
with `priory give --force`. A task that moves starts over at its workflow's start on the new owner
([runners.md](../daemon/runners.md)), so the new owner starts a new cloud run with a new session
and a new grant. The old grant stops working at once: the gateway refuses a call whose grant names a
runner that no longer owns the task, and the old owner publishes `priory.run.revoked` when it drops
the task.

The old session keeps running until its deadline, since no command stops a cloud session. Only a
runner signed in to the same Claude account can send it a message with `claude -p MSG --cloud ID`.
So both runners sign in to one Claude account, and the new owner tells the old session to stop.
With two accounts, the old session runs until it ends on its own, and its calls are refused.

## Security

The gateway is Priory's first listener beyond loopback. It serves only what a cloud run needs.

### Scopes

| Who | Signs in with | Reaches |
| --- | ------------- | ------- |
| A cloud session | the person's connector token and a run grant | `ask`, `wait` and `result` of its own task, while its run is live |
| A person, from slice 4 | the web view's token, `PRIORY_TOKEN`, on the tailnet only | the desk and the views of every runner |
| A person, after slice 5, if a later slice serves the view publicly | GitHub, through the gateway's sign-in | the same |

A connector belongs to the claude.ai account, so every session of that account gets its tools. So
the gateway's MCP endpoint never serves the desk: a session that could reach it could answer its
own asks. The person's desk stays on the web view: on the tailnet with its token in slice 4, and
behind GitHub sign-in only once a later slice serves it publicly (open question 4).

### Sign-in and tokens

The gateway follows the MCP authorization spec, revision 2025-11-25: OAuth 2.1 with PKCE, Protected
Resource Metadata at `/.well-known/oauth-protected-resource`, and authorization server metadata. It
is its own authorization server, and it hands the login to GitHub. That revision makes dynamic
client registration optional, and prefers a pre-registered client or a Client ID Metadata Document.

- One pre-registered client for claude.ai, and no `/register` endpoint. A claude.ai custom
  connector accepts a static client id and secret in place of registering itself, by Anthropic's
  connector docs. To verify in slice 5, before the gateway drops `/register`. Its only redirect URIs
  are the claude.ai and claude.com callback URLs.
- Each access token names the gateway's MCP URL as its audience, from the `resource` parameter of
  RFC 8707. The gateway refuses a token issued for another audience, and never passes a token on to
  GitHub or anyone else.
- A GitHub OAuth app with no scope, which proves who the person is and reads nothing else.
- An allow list of GitHub account ids, not logins, since a login can be renamed. The gateway checks
  it at each sign-in, each refresh and each call.
- Access tokens last 1 hour. Refresh tokens last 30 days and rotate on each use. The gateway
  stores only their hashes.
- `priory gateway revoke <account>` deletes an account's tokens at once. Removing the account from
  the allow list refuses its next call.
- A run grant lasts until its run ends or its step's `timeout`, whichever comes first. The runner
  publishes `priory.run.revoked` when the run ends.

### Secrets on the diocese

| Secret | Its power |
| ------ | --------- |
| The superuser's password | any message on the log, so an answer on any runner; held by nothing that runs |
| The intake's database role | publishes only the event types the intake publishes, as Database roles says |
| The gateway's database role | publishes only `priory.run.called` |
| The view daemon's database role | publishes answers, dismisses and task actions, so it approves on any runner |
| A read-only GitHub token, fine-grained, for the factory's repositories | reads issues, pull requests, checks and contents |
| Linear and Slack read tokens | reads what the triggers watch |
| The Slack shortcut's tokens, `SLACK_APP_TOKEN` and `SLACK_BOT_TOKEN` | opens a Socket Mode connection with `connections:write`, opens the form with `views.open`, posts to the shortcut's `response_url` ([slack.md](../intake/slack.md#message-shortcut)) |
| Webhook secrets, once webhooks move to the diocese | accepts a forged delivery |
| The GitHub OAuth app's secret and the gateway's signing key | signs in an allowed account, issues tokens |

The intake reads GitHub through `gh api`. On the diocese, the read-only token reaches `gh` as
`GH_TOKEN` in the intake's service environment, so `gh` is never signed in there. Every GitHub
write stays on the runners: pushes, comments, pull requests and merges use each runner's own `gh`.
The Slack shortcut is the diocese's one write to a source: a form and an ephemeral reply.

Each service runs as its own Unix user, with its own environment file, so the gateway cannot read
the intake's tokens or the view daemon's database password.

### Database roles

Today one password reaches the log, and it can publish any message, so whoever holds it approves on
every runner ([adding-a-second-machine.md](../adding-a-second-machine.md#limits)). The gateway is
the public listener and publishes to the log, so it must not hold that password. Each part gets a
Postgres role of its own:

| Role | May read | May publish | May move the cursor of |
| ---- | -------- | ----------- | ---------------------- |
| One per runner | every message | any message, as today | `priory.<runner>` |
| The view daemon | every message | answers, dismisses and task actions | `priory.view.<hostname>` |
| The intake | every message | the source events and requests its triggers make: never an answer, claim, hold, give or alive message | `priory.intake` |
| The gateway | every message | `priory.run.called` only | `priory.gateway` |

A row security policy on `priory_log_messages` checks the `type` field of each inserted message
against the role's list, and one on `priory_log_cursors` checks the consumer. A restricted role skips
the schema step a client runs on connect, since it may not create tables. Slice 2 builds both with
the intake's role, and slice 5 adds the gateway's and the view daemon's.

### What an attacker with the VM gets

Root on the diocese reads and writes the log. So it can answer any ask on any runner, request tasks,
and give tasks away. Through those, it steers what agents do on the runners with their permissions.
It also reads the intake's tokens, the traces and the audit log. It cannot push to GitHub, open
a shell on a runner, or read a runner's state file, since the diocese reaches no runner.

Code execution in the gateway, through a bug in its HTTP or OAuth handling, gets the gateway's Unix
user and its database role. It can then:

- publish `priory.run.called` for any live grant, so call `ask`, `wait` or `result` for any running
  cloud run, and end its step with any outcome that the step routes to;
- read the log, so every task's events, asks and answers;
- issue tokens to itself, which reach only the gateway.

It cannot answer an ask, claim, give or approve, and it cannot read the intake's tokens. A step
that a cloud run ends still leads to the human step before `open-pr`, so a forged `result` cannot
merge code.

This is the same power the log's password grants today. The diocese moves it from a desktop to a VM on
the internet, so the VM runs nothing else: unattended security updates, ssh on the tailnet with
keys only, no password login, and the one public port. Signed answers, which a runner checks
against a person's key, would take approval power away from the log. They are a later design.

### Backups, TLS, rate limits and the audit log

- **Backups.** A nightly `pg_dump`, encrypted with a public key whose private key is off the VM, is
  copied to object storage outside the VM's provider. A runner restores the latest one into a
  scratch container once a month. OpenObserve's data is not backed up: losing a few days of spans
  loses no task.
- **TLS.** A TLS proxy with an ACME certificate on a domain the owner holds, such as Caddy, ends TLS
  on port 443 and forwards to the gateway on `127.0.0.1`. The gateway keeps `serve`'s rule: it
  listens on loopback only.
- **Rate limits.** Per address on the OAuth endpoints, 10 requests a minute. Per access token on the
  MCP endpoint, 60 calls a minute. Request bodies up to 64 KB, and at most 128 calls waiting on a
  reply at once, as `serve` holds at most 128 waiting requests.
- **Audit log.** Each sign-in, refusal, registration, token issued or revoked, and each MCP call is
  one JSON line on the diocese, with the account, the grant, the tool and the outcome, and one span in
  OpenObserve. A call's arguments are not logged, since they travel on the log already.

## Failure modes

| Failure | What keeps working | What stops | Back to normal |
| ------- | ------------------ | ---------- | -------------- |
| The VM is down | Each runner's started tasks, local asks answered on that machine, local runs' tools | New events, new claims, `waiting` tasks, cross-runner answers and gives, the gateway, the tailnet view | Runners publish their queues in order when it is back; the intake polls from its last cursor |
| One runner's link to the diocese is down | The same, on that runner; everything on the other | That runner's claims and starts | As today's tunnel cut: it reads the log before any run |
| A runner is away past its 30-minute lease | Its `kept` tasks wait for it | Its `waiting` tasks move to the other runner | Its tasks that moved are dropped with `moved to runner <name>` |
| The gateway is down | Cloud runs still push their branch | Cloud runs' `ask`, `wait` and `result` | The session retries; the call id applies a retry once |
| Clock skew | Leases use Postgres's clock | Token expiry is wrong by the VM's skew | chrony on the VM |
| Slow Postgres over the WAN | Everything, slower | Nothing, below the `[log]` `timeout` of 10 seconds | |

Notes on each:

- **The VM is down.** This is today's desktop-asleep case for the second runner, now for both. A
  cloud run whose gateway call fails can still end its step by pushing its branch, so ADR 0005's
  channel is also the fallback. A webhook delivery that fails is caught by the next poll.
- **An outage longer than 30 minutes moves `waiting` tasks.** When the log comes back, the first
  runner to publish an alive message sees the other's last alive message 30 minutes old, and claims
  its `waiting` tasks before the other's queued alive message lands. No task runs twice, since only
  never-started tasks move, but tasks shuffle for no reason. The same happens today when the
  desktop holding the log sleeps. Slice 3 fixes it with a diocese heartbeat: the diocese's view daemon
  publishes `priory.diocese.alive` once a minute. A lease does not run across a gap in the heartbeats,
  which means the log was down. When both runners sleep and the diocese stays up, the heartbeats go on,
  so the runners' leases end as they do today, and a runner that wakes first takes the other's
  `waiting` tasks.
- **A runner away.** A cloud run owned by that runner keeps working in its session, but its calls
  get "runner away" after 40 seconds. Its branch push waits on GitHub until the runner polls again.
- **Clock skew.** Postgres stamps alive, claim and held messages, so the runners' clocks never decide
  a lease. The diocese's clock decides token lifetimes, grant deadlines and TLS, so it runs chrony. Run
  grant deadlines travel as durations and take the log's time when stored.
- **Latency.** A publish takes the log's advisory lock and inserts in one transaction: three round
  trips, about 100 ms at a 30 ms round trip. A runner publishes about one message a minute and
  reads once a second, so two runners and the diocese make a few queries a second. A claim is first in
  the log, so the runner nearer the diocese wins a race slightly more often. With the diocese between them,
  neither runner is favored by being next to the log, as the desktop is today.

## Sizing and cost

What the diocese's parts need, estimated, to be measured in slice 1:

| Part | Memory | Disk |
| ---- | ------ | ---- |
| Postgres with the log | 512 MB to 1 GB | 1 GB, growing slowly: alive messages add about 2,900 rows a day for two runners |
| `priory intake` | 100 MB | its checkout of the factory |
| `priory gateway` and the view daemon | 200 MB | a state file of a few hundred MB |
| OpenObserve, single node | 1 to 2 GB | 5 to 20 GB for 30 days of spans |
| Tailscale, the TLS proxy, the OS | 300 MB | 5 GB |
| Total | about 3 GB at peak | about 30 GB with room |

So 4 GB of memory is the floor and 8 GB leaves room for OpenObserve's queries.

Approximate monthly prices, excluding VAT, checked on 2026-11-26 from third-party price trackers.
The providers' own pages win over this table, and Hetzner raised its prices twice in 2026.

| Option | vCPU | Memory | Disk | About |
| ------ | ---- | ------ | ---- | ----- |
| Hetzner CX33, x86, cost-optimized line | 4 | 8 GB | 80 GB | 9 EUR with IPv4, often out of stock |
| Hetzner CAX21, Arm | 4 | 8 GB | 80 GB | 11 EUR with IPv4 |
| Scaleway DEV1-M, x86 | 3 | 4 GB | block storage billed apart | 15 EUR, about 20 EUR with IPv4 and 40 GB |

Recommendation: the Hetzner CAX21. It fits the budget with 8 GB. It needs an aarch64 build of
`priory`, and Postgres and OpenObserve publish Arm images. Take the CX33 when it is in stock and an
x86 build matters more.

## Open questions

Decided on 2026-11-26: the owner took every recommendation below. Question 1's test still runs
before slice 5 writes code.

1. **Does a `claude --cloud` session load the account's connectors?** One test settles it: add a
   connector of the account to a test MCP server with one `ping` tool, start a session with
   `claude --cloud "call ping"`, and read the session's page. Recommendation: run this test before
   slice 5 writes code. If sessions do not load connectors, slice 6 uses an HTTP MCP server in the
   cloud environment's settings instead, with the run grant as a bearer token, and slice 5 needs no
   OAuth at all. Either way, the test's answer then goes into
   [claude-ai-connectors.md](claude-ai-connectors.md)'s opening paragraph, which today says a
   connector reaches every cloud session, and into the MCP servers row of
   [cloud.md](../daemon/cloud.md#what-still-applies-and-what-a-cloud-run-gives-up).
2. **One `priory gateway` command, or a separate service?** Recommendation: a command of the
   `priory` binary. It shares the log trait, the tools' schemas and the release, and a separate
   service would copy the tool contract. Its OAuth needs may add a dependency, which slice 5
   proposes with its license and last release.
3. **A GitHub OAuth app, or a GitHub App, for sign-in?** Recommendation: an OAuth app with no
   scope. Sign-in needs identity only, and an OAuth app needs no installation on any repository. A
   GitHub App is worth it later if it replaces the intake's read token with an installation token
   that expires.
4. **Is the web view also served from the VM?** Recommendation: yes, on the tailnet only, from the
   view daemon through `tailscale serve`, in slice 4. A phone then reaches the desk while both
   runners sleep. Serving it publicly behind the gateway's GitHub sign-in can come after slice 5
   proves that sign-in.
5. **Tailscale or plain WireGuard?** Recommendation: Tailscale. Runners are already named by their
   Tailscale hostnames, its access rules keep the diocese from reaching the runners, and its traffic
   needs no open port. Plain WireGuard removes the coordination service, at the cost of keys and
   rules kept by hand.

## Slices

Each slice ships alone and leaves the factory working.

### 1. The VM, Postgres and the private network, with the desktop as the only runner

- **Operations.** Rent the VM, join it to the tailnet, start Postgres bound to the tailnet address
  and OpenObserve beside it, set up the nightly dump. Stop the desktop's daemon and intake, dump the
  desktop's log, restore it on the VM, point `PRIORY_LOG_URL` and `[telemetry]` `otlp` at the VM, and
  start them again.
- **Code.** None.
- **Tests.** The existing Postgres log tests, against the VM from the desktop. Measure a publish and
  a read from the desktop, median and 99th percentile, and the log's growth for a week. Restore the
  first nightly dump into a scratch container.
- **Rollback.** Stop the daemon, dump the VM's log, restore it into the desktop's container, and
  point `PRIORY_LOG_URL` back.

### 2. The intake on the VM

- **Operations.** A checkout of the factory on the VM, the read-only tokens, and `priory intake` as
  a service there. Stop the desktop's intake.
- **Code.** The intake's database role, as Database roles says. Each runner publishes what its
  intake would watch: its open waits, holds, subscriptions and task branches, as
  `priory.watch.posted`. The intake reads them from the log as its own consumer, `priory.intake`,
  in place of a state file. A `pre` script then runs on the VM, so its tools must be there.
- **Tests.** A runner's wait on a pull request reaches the intake through the log, and its event
  routes back to that runner. A runner that ends the task removes the watch.
- **Rollback.** Stop the VM's intake and start the desktop's. Events carry their ids, so an event
  both published adds nothing.

### 3. The second runner

- **Operations.** The second machine joins the tailnet, and its `PRIORY_LOG_URL` points at the VM.
  Its runner file, such as `runners/key-drum.toml`, already exists. Remove the reverse ssh tunnel.
- **Code.** The diocese heartbeat, `priory.diocese.alive`, and a lease that does not run across a gap in
  it, as Failure modes says, so a diocese outage moves no task.
- **Built.** `priory daemon --view-only --diocese` publishes the heartbeat, and a gap of more than
  3 minutes stops every lease until the heartbeats are back, when each lease runs 30 minutes again.
  A diocese silent for 24 hours counts as gone. `scripts/second-machine.sh cut local|remote` cuts
  one runner's link. See [runners.md](../daemon/runners.md#a-log-outage-with-a-diocese).
- **Tests.** `scripts/second-machine.sh` with the log on the VM, its `tunnel-down` replaced by
  cutting one runner's tailnet link. A property test: after a log gap longer than 30 minutes, the
  first runner back claims none of the other's `waiting` tasks, and with the heartbeats going on, a
  runner asleep for 30 minutes still loses them.
- **Rollback.** Stop the second daemon. Its `waiting` tasks move after 30 minutes, or sooner with
  `priory give --force`.

### 4. Views from any runner (R8.5)

- **Operations.** The diocese's view daemon serves the web view on the tailnet through `tailscale
  serve`.
- **Code.** A task's detail and runs from another runner, the later slice 4 of
  [views-from-any-machine.md](views-from-any-machine.md#later-slices), only if a link to the owner's
  view proves too thin.
- **Tests.** The view-only tests already on `main`. By hand: answer an ask from a phone while the
  desktop sleeps, and see it applied when the desktop wakes.
- **Rollback.** Stop the diocese's view daemon. Each runner still serves its own view.

### 5. The OAuth MCP gateway with GitHub sign-in

- **Operations.** A domain, the TLS proxy, port 443 open, a GitHub OAuth app, and the connector added
  in claude.ai. Settle open question 1 first.
- **Code.** `priory gateway`: the OAuth endpoints with a pre-registered client, the audience check,
  the allow list, tokens and revocation, the rate limits and the audit log. The gateway's and the
  view daemon's database roles. Its MCP endpoint serves one `whoami` tool, which proves the
  connector works and acts on nothing.
- **Tests.** The OAuth flow against a fake GitHub: an allowed account gets a token, another is
  refused, an expired token is refused, a refresh rotates, revoke ends a token, a redirect URI that
  is not claude.ai's is refused, a token for another audience is refused, and the rate limits answer
  `429`. Against Postgres: the gateway's role cannot insert `priory.ask.answered`, and the intake's
  cannot insert a claim.
- **Rollback.** Stop the gateway, close port 443 and remove the connector. Nothing else depends on
  it.

### 6. Cloud runs call `ask`, `wait` and `result` through the gateway

- **Operations.** None beyond slice 5.
- **Code.** Run grants, `priory.run.called` and `priory.run.replied`, the owner applying a call as
  a socket call, waking a session with `claude -p MSG --cloud ID`, and `priory check` allowing
  `ask` and `wait` on a cloud agent, and other outcomes than `pushed`. Before it takes a pushed
  branch, the runner checks its commits and messages for the run's token, as it screens private
  terms, and fails the step and revokes the grant when it finds it. An ADR records that a cloud run
  reports through the gateway, with the branch push still carrying its code. A `[cloud] gateway` key turns it on per factory.
- **Tests.** A fake session calls each tool through the gateway, and the owner applies it once. A
  grant of another task, an ended run and a moved task are refused. An owner away answers "runner
  away" after 40 seconds, and the retry applies once. A pushed commit holding the token fails the
  step. A task given to the other runner gets a new grant, and the old one is refused.
- **Rollback.** Set `[cloud] gateway = false`: cloud runs report by branch push only, as today.
