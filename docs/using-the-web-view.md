# How to use the web view

This guide shows how to turn on the web view, open it on the machine that runs the daemon, then
reach it from a phone, install it there and get notifications. It assumes a factory that
`priory check` accepts and a daemon you can restart.

## 1. Turn it on

Add a `[serve]` table to `factory.toml`:

```toml
[serve]
listen = "127.0.0.1:7340"
```

The daemon refuses to start with this table unless `PRIORY_TOKEN` holds a token. Generate one, keep
it, and restart the daemon with it in its environment, from your factory folder:

```sh
export PRIORY_TOKEN=$(openssl rand -hex 32)
echo "$PRIORY_TOKEN"
priory daemon
```

When the daemon runs as a service, put the token in the services' environment file instead, as
[serve.md](daemon/serve.md#the-token) shows. The daemon prints
`priory: serving on http://127.0.0.1:7340`.

## 2. Open it on the same machine

Open `http://127.0.0.1:7340` in a browser and paste the token. The browser keeps it, so it asks
once. [views.md](reference/views.md#what-each-panel-shows) shows what each panel holds.

The server listens on this machine only, and nothing else can connect yet.

## 3. Reach it from a phone

A phone needs two things the daemon does not give by itself: a way in from outside the machine, and
HTTPS, because a browser installs an app and offers notifications only on a secure origin.

Which tunnel provides them is your call, for each machine. Whoever reaches the view and holds the
token reads every task and every agent's screen, and answers asks. Keep the daemon on loopback and
put a proxy that terminates HTTPS in front of it. `public = true` in `[serve]` allows a `listen`
address beyond the machine itself, which the daemon then serves over plain HTTP: see
[Beyond this machine](daemon/serve.md#beyond-this-machine) before you set it.

This guide uses [Tailscale](https://tailscale.com/kb/1242/tailscale-serve), so only the devices of
your tailnet can connect. It assumes Tailscale runs on the machine and on the phone.

1. Put Tailscale's HTTPS proxy in front of the daemon:

   ```sh
   tailscale serve --bg 7340
   ```

   It prints the address, such as `https://runner.example.ts.net`, and keeps serving across
   reboots. `tailscale serve --bg 7340 off` turns it off. Turning it on exposes the view to every
   device of your tailnet.

2. On the phone, open that address and paste the token.

## 4. Install it on the phone

- iPhone or iPad: tap Share, then Add to Home Screen, and open Priory from the Home Screen. iOS
  lets only the installed view get notifications.
- Android: tap Install Priory under More, then Notifications, or Install app in Chrome's menu.

## 5. Turn on notifications

Under More, then Notifications, tap Turn on notifications, and allow them.

A new ask now shows its title on the phone, or its task and step, and tapping it opens the ask. A
task that turns blocked or failed shows `task <key> <state>`, and an intake that has failed to poll a
source for 15 minutes shows `the intake fails to poll <source>`. A snoozed item sends nothing.

Check the kinds this device gets on the same screen, such as Asks alone on a phone. Landed wrong and
Queue stalls are off until you check them. [The reference](reference/serve.md#notifications) lists
each kind. If the screen says the daemon did not confirm it, reload the page to see the choice again.

Turn notifications off on the same screen. [serve.md](daemon/serve.md#notifications) explains when a notification is sent and
retried.

On the machine that runs the daemon, `[notify] desktop = true` in `factory.toml` shows the same
title as a desktop notification, through `notify-send`, for the one next thing on the desk.
