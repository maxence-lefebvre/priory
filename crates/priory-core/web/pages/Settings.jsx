// Settings: Appearance and Notifications (DESIGN.md, Settings). A phone opens them from More.
import { memo, useEffect, useState } from "react";
import { Icon } from "../components/Icon.jsx";
import { useStore } from "../hooks/useData.js";
import { PUSH_KINDS, confirmPush, saveKinds } from "../push.js";
import { KEPT_KINDS, api, post } from "../session.js";
import { store } from "../store.js";
import { LegacyPreview } from "./PageGrid.jsx";
import { listClass, usePlace } from "./place.js";

// Notifications on this device: its status, a request in flight, and why the last one failed, and
// the browser's install prompt, kept until the person taps Install. `check` counts status reads, so a
// read that a tap or a newer read overtook is dropped.
export const push = store({ status: null, kinds: null, busy: false, error: null, check: 0, checking: false, install: null });
const change = (part) => push.set({ ...push.get(), ...part });

// The browser offers to install only once the page handles this; the Notifications screen shows it.
if (typeof window !== "undefined") {
  addEventListener("beforeinstallprompt", (event) => {
    event.preventDefault();
    change({ install: event });
  });
}

export const Settings = memo(function Settings() {
  const { size, section } = usePlace();
  const screen = section === "notifications" ? <Notifications /> : <Appearance />;
  const back = size === "compact" ? <div className="top"><a className="back" href="/more"><Icon name="chevron-left" size="s22" />More</a></div> : null;
  return <div className={listClass(size, "list ")}>{back}{screen}</div>;
});

const NotificationsLink = () => (
  <ul className="links"><li><a href="/settings/notifications"><Icon name="bell" size="s20" />Notifications<span className="chev"><Icon name="chevron-right" size="s15" /></span></a></li></ul>
);

// Appearance
const THEMES = [
  ["graphite", "Graphite", "The default. A grey ramp, ink actions and one amber for what waits on you."],
  ["indigo", "Indigo", "Ink emphasis with an indigo action."],
  ["ink", "Ink", "Monochrome. What needs you is printed in solid ink."],
  ["parchment", "Parchment", "Parchment and iron-gall ink, rubric red and lapis."],
  ["abbey", "Abbey", "Vellum, lapis and gold, rubric red."],
];
const MODES = ["system", "light", "dark"];

/** Sets the mode or the theme of the whole document, and keeps both on this device. */
export function pick(name, value) {
  const root = document.documentElement;
  root.dataset[name] = value;
  localStorage.setItem("priory.mode", root.dataset.mode);
  localStorage.setItem("priory.theme", root.dataset.theme);
}

// Space or Enter picks a theme, which is no button.
const pickKey = (event, choose) => {
  if (event.key !== " " && event.key !== "Enter") return;
  event.preventDefault();
  choose();
};

export const Appearance = memo(function Appearance() {
  const root = document.documentElement;
  const [{ mode, theme }, set] = useState(() => ({ mode: root.dataset.mode, theme: root.dataset.theme }));
  const choose = (name, value) => {
    pick(name, value);
    set({ mode: root.dataset.mode, theme: root.dataset.theme });
  };
  return (
    <div className="scroll"><div className="screen">
      <h1>Appearance</h1>
      <h2 id="mode-label">Mode</h2>
      <div className="seg" role="radiogroup" aria-labelledby="mode-label">{MODES.map((value) =>
        <button type="button" key={value} role="radio" aria-checked={mode === value ? "true" : "false"} onClick={() => choose("mode", value)}>{value[0].toUpperCase() + value.slice(1)}</button>)}</div>
      <h2 id="theme-label">Theme</h2>
      <div className="themes" role="radiogroup" aria-labelledby="theme-label">{THEMES.map(([key, name, line]) =>
        <div key={key} className="theme" role="radio" tabIndex={0} aria-checked={theme === key ? "true" : "false"} aria-label={name} onClick={() => choose("theme", key)} onKeyDown={(event) => pickKey(event, () => choose("theme", key))}>
          <div className="previews" aria-hidden="true" inert>
            <div className="preview" data-theme={key} data-mode="light"><LegacyPreview name="desk" /></div>
            <div className="preview" data-theme={key} data-mode="dark"><LegacyPreview name="desk" /></div></div>
          <span className="name">{name}<span className="radio" /></span><p>{line}</p></div>)}</div>
      <p className="note">A theme changes color and type only. Every screen keeps the same layout.</p></div>
      <NotificationsLink /></div>
  );
});

// Notifications
// iOS lets only a Home Screen web app subscribe, so Safari in a tab lacks PushManager.
const iosTab = () => /iPhone|iPad|iPod/.test(navigator.userAgent) && !navigator.standalone;

const pushIo = {
  send: (body) => post("/api/push/subscribe", body),
  async kept() {
    const hit = await (await caches.open("priory-push")).match(KEPT_KINDS);
    return hit ? hit.json().catch(() => null) : null;
  },
  async keep(kinds) {
    const cache = await caches.open("priory-push");
    if (!kinds) return cache.delete(KEPT_KINDS);
    return cache.put(KEPT_KINDS, new Response(JSON.stringify(kinds)));
  },
};

async function pushStatus() {
  if (!window.isSecureContext) return { status: "insecure" };
  if (!("serviceWorker" in navigator) || !("PushManager" in window) || !("Notification" in window)) {
    return { status: iosTab() ? "install" : "unsupported" };
  }
  if (Notification.permission === "denied") return { status: "blocked" };
  const registration = await navigator.serviceWorker.ready;
  const subscription = await registration.pushManager.getSubscription();
  if (!subscription) return { status: "off" };
  // The browser keeps a subscription the daemon may have dropped or never stored: send it again.
  // Only the daemon's refusal ends it; a dropped connection or a stopped daemon proves nothing.
  const { status, kinds, confirmed } = await confirmPush(subscription.toJSON(), pushIo)
    .catch(() => ({ status: 0, kinds: null, confirmed: false }));
  if (status === 200) return { status: confirmed ? "on" : "unconfirmed", kinds };
  if (status !== 422) return { status: "unconfirmed" };
  await subscription.unsubscribe().catch(() => {});
  return { status: "off" };
}

/** Reads this device's status once, unless a read or a tap is under way. */
function checkPush() {
  const { status, checking } = push.get();
  if (status !== null || checking) return;
  const check = push.get().check + 1;
  change({ check, checking: true });
  const settle = ({ status, kinds = null }) => {
    if (check === push.get().check) change({ status, kinds, checking: false });
  };
  pushStatus().then(settle, () => settle({ status: "unsupported" }));
}

const PUSH_NOTES = {
  on: "On for this device.",
  unconfirmed: "On in this browser, but the daemon did not confirm it. Check that the daemon runs, then reload the page.",
  blocked: "Notifications are blocked for this site. Allow them in the browser's site settings, then come back.",
  install: "On iPhone and iPad, only Priory on the Home Screen can get notifications. Tap Share, then Add to Home Screen, and open Priory from there.",
  insecure: "Notifications need HTTPS. Open Priory at its https address, such as the one tailscale serve gives, or at localhost on this machine.",
  unsupported: "This browser cannot get notifications from a web page.",
};

/** The checkboxes of the kinds the daemon replied with; nothing for a daemon with none. */
export const KindBoxes = memo(function KindBoxes({ kinds, busy }) {
  if (!kinds) return null;
  const rows = PUSH_KINDS.filter(([kind]) => kind in kinds);
  return (
    <>
      <fieldset className="push-kinds"><legend>Notify this device about</legend>{rows.map(([kind, name, line]) =>
        <label key={kind} className="push-kind"><input type="checkbox" id={`push-kind-${kind}`} checked={Boolean(kinds[kind])} aria-disabled={String(busy)} onChange={(event) => chooseKind(kind, event.target.checked)} /><span><b>{name}</b><span>{line}</span></span></label>)}</fieldset>
      {rows.some(([kind]) => kinds[kind]) ? null : <p className="note" role="status">Nothing is chosen, so this device gets no notification.</p>}
      <p className="note">Each device keeps its own choice. A change applies at once.</p>
    </>
  );
});

const install = () => {
  push.get().install?.prompt();
  change({ install: null });
};

export const Notifications = memo(function Notifications() {
  const { status, kinds, busy, error, install: offered } = useStore(push);
  useEffect(checkPush, [status]);
  const button = status === "on" || status === "unconfirmed"
    ? <button type="button" className="btn secondary" aria-disabled={String(busy)} onClick={() => turnPush(false)}>Turn off notifications</button>
    : status === "off" ? <button type="button" className="btn primary" aria-disabled={String(busy)} onClick={() => turnPush(true)}>Turn on notifications</button> : null;
  return (
    <div className="scroll"><div className="screen">
      <h1>Notifications</h1>
      <p>Get a notification on this device when something waits on you. It shows a title and opens the item. A snoozed item sends nothing.</p>
      {PUSH_NOTES[status] ? <p className="note" role="status"><Icon name={status === "on" ? "check" : "unplug"} size="s15" /><span>{PUSH_NOTES[status]}</span></p> : null}
      {error ? <p className="note warn" role="alert"><Icon name="unplug" size="s15" /><span>{error}</span></p> : null}
      {status === "on" ? <KindBoxes kinds={kinds} busy={busy} /> : null}
      {button ? <div className="push-row">{button}</div> : null}
      {offered ? <div className="push-row"><button type="button" className="btn secondary" onClick={install}>Install Priory</button>
        <p className="note">Installing opens Priory in its own window, so notifications reach it with no tab open.</p></div> : null}
    </div></div>
  );
});

// The VAPID key as the bytes PushManager.subscribe takes.
const keyBytes = (base64url) => Uint8Array.from(atob(base64url.replace(/-/g, "+").replace(/_/g, "/")), (char) => char.charCodeAt(0));

export async function turnPush(on) {
  if (push.get().busy) return;
  change({ busy: true, error: null, checking: false, check: push.get().check + 1 });
  try {
    if (on) {
      // iOS asks only from a tap, so this is the first await after it.
      const permission = await Notification.requestPermission();
      if (permission !== "granted") return void change({ status: permission === "denied" ? "blocked" : "off" });
      const { key } = await api("/api/push/key");
      const registration = await navigator.serviceWorker.ready;
      const subscription = await registration.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: keyBytes(key) });
      // A browser subscription the daemon never stored would read "On" and get nothing.
      try {
        const { status, body, kinds, confirmed } = await confirmPush(subscription.toJSON(), pushIo);
        if (status !== 200) throw new Error(body.error || `the server answered ${status}`);
        change({ kinds, status: confirmed ? "on" : "unconfirmed" });
      } catch (error) {
        await subscription.unsubscribe().catch(() => {});
        throw error;
      }
      return;
    }
    const subscription = await (await navigator.serviceWorker.ready).pushManager.getSubscription();
    if (subscription) {
      const { status, body } = await post("/api/push/unsubscribe", { endpoint: subscription.endpoint });
      if (status !== 200) throw new Error(body.error || `the server answered ${status}`);
      await subscription.unsubscribe();
    }
    change({ status: "off" });
  } catch (error) {
    const verb = on ? "turn on" : "turn off";
    const why = error instanceof TypeError ? "the connection dropped" : error.message;
    change({ error: `Could not ${verb} notifications: ${why}. Check that the daemon runs, then try again.` });
  } finally {
    change({ busy: false });
  }
}

// A refused save shows the choice as it was. A lost reply proves nothing, so the boxes hide until the
// next status read.
export async function chooseKind(kind, on) {
  if (push.get().busy) return;
  const before = push.get().kinds;
  change({ busy: true, error: null, kinds: { ...before, [kind]: on } });
  try {
    const subscription = await (await navigator.serviceWorker.ready).pushManager.getSubscription();
    if (!subscription) throw new Error("this browser is no longer subscribed");
    const kinds = await saveKinds(subscription.toJSON(), { [kind]: on }, before, pushIo);
    change({ kinds, status: kinds ? "on" : "unconfirmed" });
  } catch (error) {
    const why = `Could not save what this device gets: ${error.message}.`;
    change({ kinds: before, error: `${why} Check that the daemon runs, then try again.` });
  } finally {
    change({ busy: false });
  }
}
