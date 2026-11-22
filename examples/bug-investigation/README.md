# Bug investigation

When someone adds the label `bug` to an issue of `owner/repo`, Sentry reports a new error issue
in `my-project`, a Honeycomb Trigger fires in `production`, or a customer starts an Intercom
conversation, an agent reproduces the bug, finds its root cause, fixes it with a test, and asks
you before it opens the fix pull request.

## What it needs

`gh` logged in to GitHub with access to the repository, `claude` on `PATH`, and `PRIORY_DB`
set for the intake and the daemon. Replace `owner/repo` in `triggers/bug-label.toml`, and rename
`runners/my-machine.toml` after your machine's Tailscale hostname. Set `gate` in `factory.toml` to
the command that runs your project's tests.

The Sentry trigger needs the server that `priory daemon` starts, reachable from Sentry, and a Sentry internal integration
with the Issue webhook pointed at `<url>/hooks/sentry`. Put the integration's Client Secret in
`SENTRY_CLIENT_SECRET` and a token in `PRIORY_TOKEN`. Replace `my-project` in
`triggers/sentry-issue.toml` with your project's slug, or delete the file and the `[serve]` and
`[webhooks.sentry]` tables to use GitHub alone. See
[docs/intake/sentry.md](../../docs/intake/sentry.md).

The Honeycomb trigger needs the same server, reachable from Honeycomb, and a Honeycomb webhook
recipient pointed at `<url>/hooks/honeycomb` with Priory's payload template. Put the recipient's
Shared Secret in `HONEYCOMB_WEBHOOK_SECRET`. Replace `production` in
`triggers/honeycomb-alert.toml` with your environment's name, or delete the file and the
`[webhooks.honeycomb]` table. See [docs/intake/honeycomb.md](../../docs/intake/honeycomb.md).

The Intercom trigger needs the same server, reachable from Intercom, and an Intercom app whose
webhook URL is `<url>/hooks/intercom`, subscribed to `conversation.user.created`. Put the app's
client secret in `INTERCOM_CLIENT_SECRET`, and replace `my-workspace` in `factory.toml` with your
workspace id. Or delete `triggers/intercom-conversation.toml` and the `[webhooks.intercom]` table.
See [docs/intake/intercom.md](../../docs/intake/intercom.md).

## Gates

- `fix` takes `passed` only once the trace records a `priory gate` that passed at the branch's
  head.
- `approve-pr` asks you before `open-pr` pushes the branch and opens the pull request.
- A bug the agent cannot reproduce ends the task at `investigate`, with nothing pushed.

## Run it

```sh
cp -r examples/bug-investigation my-factory
priory check my-factory
export PRIORY_DB=~/.local/state/priory-oss/priory.db
priory intake my-factory    # keeps polling: run the daemon in a second terminal
priory daemon my-factory
```
