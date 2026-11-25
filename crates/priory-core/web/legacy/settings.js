// The settings page's legacy drawing and actions.
import { render } from "../app.js";
import { PANELS } from "../pages.js";
import { confirmPush, kindBoxes, saveKinds } from "../push.js";
import { KEPT_KINDS, api } from "./data.js";
import { deskPhone } from "./desk.js";
import { html, icon, panelsOf, post, raw, route, state } from "./shared.js";


export function morePage() {
  const rest = panelsOf().slice(4);
  return html`<div class="top"><h1>More</h1></div><div class="scroll"><ul class="links">
    ${rest.map((panel) => html`<li><a href="/${panel}">${icon(PANELS[panel][1], "s20")}${PANELS[panel][0]}<span class="chev">${icon("chevron-right", "s15")}</span></a></li>`)}
    <li><a href="/settings/appearance">${icon("settings", "s20")}Appearance<span class="chev">${icon("chevron-right", "s15")}</span></a></li>
    <li><a href="/settings/notifications">${icon("bell", "s20")}Notifications<span class="chev">${icon("chevron-right", "s15")}</span></a></li></ul>
    <div class="signout"><button class="btn secondary" data-action="sign-out">${icon("lock", "s20")}Sign out of this device</button></div></div>`;
}

// Appearance
const THEMES = [
  ["graphite", "Graphite", "The default. A grey ramp, ink actions and one amber for what waits on you."],
  ["indigo", "Indigo", "Ink emphasis with an indigo action."],
  ["ink", "Ink", "Monochrome. What needs you is printed in solid ink."],
  ["parchment", "Parchment", "Parchment and iron-gall ink, rubric red and lapis."],
  ["abbey", "Abbey", "Vellum, lapis and gold, rubric red."],
];
function appearance() {
  const root = document.documentElement;
  const mode = root.dataset.mode;
  const preview = state.data ? deskPhone(false) : "";
  return html`<div class="scroll"><div class="screen">
    <h1>Appearance</h1>
    <h2 id="mode-label">Mode</h2>
    <div class="seg" role="radiogroup" aria-labelledby="mode-label">${["system", "light", "dark"].map((value) =>
      html`<button role="radio" aria-checked="${mode === value}" data-pick-mode="${value}" data-action="mode">${value[0].toUpperCase() + value.slice(1)}</button>`)}</div>
    <h2 id="theme-label">Theme</h2>
    <div class="themes" role="radiogroup" aria-labelledby="theme-label">${THEMES.map(([key, name, line]) =>
      html`<div class="theme" role="radio" tabindex="0" aria-checked="${root.dataset.theme === key}" aria-label="${name}" data-theme-pick="${key}" data-action="theme">
        <div class="previews" aria-hidden="true" inert>
          <div class="preview" data-theme="${key}" data-mode="light"><div>${preview}</div></div>
          <div class="preview" data-theme="${key}" data-mode="dark"><div>${preview}</div></div></div>
        <span class="name">${name}<span class="radio"></span></span><p>${line}</p></div>`)}</div>
    <p class="note">A theme changes color and type only. Every screen keeps the same layout.</p></div>
    <ul class="links"><li><a href="/settings/notifications">${icon("bell", "s20")}Notifications<span class="chev">${icon("chevron-right", "s15")}</span></a></li></ul></div>`;
}

// Notifications
export const settingsScreen = () => (route()[1] === "notifications" ? notifications() : appearance());

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

const PUSH_NOTES = {
  on: "On for this device.",
  unconfirmed: "On in this browser, but the daemon did not confirm it. Check that the daemon runs, then reload the page.",
  blocked: "Notifications are blocked for this site. Allow them in the browser's site settings, then come back.",
  install: "On iPhone and iPad, only Priory on the Home Screen can get notifications. Tap Share, then Add to Home Screen, and open Priory from there.",
  insecure: "Notifications need HTTPS. Open Priory at its https address, such as the one tailscale serve gives, or at localhost on this machine.",
  unsupported: "This browser cannot get notifications from a web page.",
};

function notifications() {
  const { status, busy, error } = state.push;
  if (status === null && !state.push.checking) {
    const check = ++state.push.check;
    state.push.checking = true;
    const settle = ({ status, kinds = null }) => {
      if (check !== state.push.check) return;
      Object.assign(state.push, { status, kinds, checking: false });
      render();
    };
    pushStatus().then(settle, () => settle({ status: "unsupported" }));
  }
  const button = status === "on" || status === "unconfirmed"
    ? html`<button class="btn secondary" data-action="push-off" aria-disabled="${busy}">Turn off notifications</button>`
    : status === "off" ? html`<button class="btn primary" data-action="push-on" aria-disabled="${busy}">Turn on notifications</button>` : "";
  const install = state.install
    ? html`<div class="push-row"><button class="btn secondary" data-action="install">Install Priory</button>
      <p class="note">Installing opens Priory in its own window, so notifications reach it with no tab open.</p></div>`
    : "";
  return html`<div class="scroll"><div class="screen">
    <h1>Notifications</h1>
    <p>Get a notification on this device when something waits on you. It shows a title and opens the item. A snoozed item sends nothing.</p>
    ${PUSH_NOTES[status] ? html`<p class="note" role="status">${icon(status === "on" ? "check" : "unplug", "s15")}<span>${PUSH_NOTES[status]}</span></p>` : ""}
    ${error ? html`<p class="note warn" role="alert">${icon("unplug", "s15")}<span>${error}</span></p>` : ""}
    ${status === "on" ? raw(kindBoxes(state.push.kinds, busy)) : ""}
    ${button ? html`<div class="push-row">${button}</div>` : ""}
    ${install}</div></div>`;
}

// The VAPID key as the bytes PushManager.subscribe takes.
const keyBytes = (base64url) => Uint8Array.from(atob(base64url.replace(/-/g, "+").replace(/_/g, "/")), (char) => char.charCodeAt(0));

export async function turnPush(on) {
  if (state.push.busy) return;
  Object.assign(state.push, { busy: true, error: null, checking: false });
  state.push.check++;
  render();
  try {
    if (on) {
      // iOS asks only from a tap, so this is the first await after it.
      const permission = await Notification.requestPermission();
      if (permission !== "granted") return void (state.push.status = permission === "denied" ? "blocked" : "off");
      const { key } = await api("/api/push/key");
      const registration = await navigator.serviceWorker.ready;
      const subscription = await registration.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: keyBytes(key) });
      // A browser subscription the daemon never stored would read "On" and get nothing.
      try {
        const { status, body, kinds, confirmed } = await confirmPush(subscription.toJSON(), pushIo);
        if (status !== 200) throw new Error(body.error || `the server answered ${status}`);
        Object.assign(state.push, { kinds, status: confirmed ? "on" : "unconfirmed" });
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
    state.push.status = "off";
  } catch (error) {
    const verb = on ? "turn on" : "turn off";
    const why = error instanceof TypeError ? "the connection dropped" : error.message;
    state.push.error = `Could not ${verb} notifications: ${why}. Check that the daemon runs, then try again.`;
  } finally {
    state.push.busy = false;
    render();
  }
}

// A refused save shows the choice as it was. A lost reply proves nothing, so the boxes hide until the
// next status read.
export async function chooseKind(kind, on) {
  if (state.push.busy) return render();
  const before = state.push.kinds;
  Object.assign(state.push, { busy: true, error: null, kinds: { ...before, [kind]: on } });
  render();
  try {
    const subscription = await (await navigator.serviceWorker.ready).pushManager.getSubscription();
    if (!subscription) throw new Error("this browser is no longer subscribed");
    const kinds = await saveKinds(subscription.toJSON(), { [kind]: on }, before, pushIo);
    Object.assign(state.push, { kinds, status: kinds ? "on" : "unconfirmed" });
  } catch (error) {
    const why = `Could not save what this device gets: ${error.message}.`;
    Object.assign(state.push, { kinds: before, error: `${why} Check that the daemon runs, then try again.` });
  } finally {
    state.push.busy = false;
    render();
  }
}

export function settingsClick(action, target) {
  if (action === "push-on" || action === "push-off") turnPush(action === "push-on");
  else if (action === "install" && state.install) {
    state.install.prompt();
    state.install = null;
    render();
  } else if (action === "mode" || action === "theme") {
    const root = document.documentElement;
    if (action === "mode") root.dataset.mode = target.dataset.pickMode;
    else root.dataset.theme = target.dataset.themePick;
    localStorage.setItem("priory.mode", root.dataset.mode);
    localStorage.setItem("priory.theme", root.dataset.theme);
    render();
  } else return false;
  return true;
}

export function settingsChange(target) {
  const kind = target.dataset.pushKind;
  if (kind) chooseKind(kind, target.checked);
}

// The browser offers to install only once the page handles this; the Notifications screen shows it.
addEventListener("beforeinstallprompt", (event) => {
  event.preventDefault();
  state.install = event;
  render();
});
