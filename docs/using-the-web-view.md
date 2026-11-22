# How to use the web view from a phone

The web view installs as an app and can notify you of a new ask or a stopped task. A phone reaches
it over HTTPS only, because a browser offers notifications only on a secure origin. This section
uses [Tailscale](https://tailscale.com/kb/1242/tailscale-serve), so the daemon stays on loopback
and only your tailnet can connect. It assumes Tailscale runs on the machine and on the phone.

1. Serve the web view on loopback. In `factory.toml`:

   ```toml
   [serve]
   listen = "127.0.0.1:7340"
   web = true
   ```

   Set `PRIORY_TOKEN` in the daemon's environment, as [serve.md](daemon/serve.md#the-token)
   shows, and restart `priory daemon`.

2. Put Tailscale's HTTPS proxy in front of it:

   ```sh
   tailscale serve --bg 7340
   ```

   It prints the address, such as `https://runner.example.ts.net`, and keeps serving across
   reboots. `tailscale serve --bg 7340 off` turns it off. Turning it on exposes the view to every
   device of your tailnet, so decide it for each machine.

3. On the phone, open that address and paste the token.

4. Install the view:
   - iPhone or iPad: tap Share, then Add to Home Screen, and open Priory from the Home Screen.
     iOS lets only the installed view get notifications.
   - Android: tap Install Priory under More, then Notifications, or Install app in Chrome's menu.

5. Under More, then Notifications, tap Turn on notifications, and allow them.

A new ask now shows its title on the phone, or its task and step, and tapping it opens the ask. A task that turns blocked
or failed shows `task <key> <state>`, and a failing intake shows `the intake fails to poll <source>`. A snoozed item sends nothing. Turn notifications off on the
same screen.

On the machine that runs the daemon, `[notify] desktop = true` in `factory.toml` shows the same
title as a desktop notification, through `notify-send`, for the one next thing on the desk.
