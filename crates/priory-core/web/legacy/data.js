// The legacy pages' reads, their snapshot and signing out.
import { render } from "../app.js";
import { mergeSince } from "../desk.js";
import { client, land, params, readAll, sameListings } from "../queries.js";
import { RUN_VIEWS } from "../runs.js";
import { TASK_VIEWS, taskLabel } from "../tasks.js";
import { askView } from "./asks.js";
import { loadCosts } from "./costs.js";
import { deskView } from "./desk.js";
import { loadUnrouted } from "./inbox.js";
import { acceptReport } from "./reports.js";
import { current, listView, selectedAsk, state } from "./shared.js";
import { keyboard, watch } from "./terminal.js";


// Data
export async function api(path) {
  const response = await fetch(path, { headers: { Authorization: `Bearer ${state.token}` }, cache: "no-store" });
  if (response.status === 401) throw Object.assign(new Error("the token was refused"), { status: 401 });
  if (!response.ok) throw Object.assign(new Error(`${path} answered ${response.status}`), { status: response.status });
  return response.json();
}

// The server data lives in one QueryClient; refresh() reads it again.
export const cache = client();
// What the long poll waits on: none while signed out or before the first read.
export const revision = () => (state.token ? state.revision : undefined);
export const revisionMoved = () => watch.revision();

export async function refresh() {
  if (!state.token) return render();
  const failed = state.failed;
  const call = ++state.refreshes;
  // Older rows load again when the person scrolls back to them.
  const history = {
    asks: Boolean(selectedAsk()) || (current() === "asks" && ["answered", "all"].includes(askView())),
    tasks: (current() === "desk" && deskView() === "all") || (current() === "tasks" && listView(TASK_VIEWS) === "ended"),
    runs: current() === "runs" && listView(RUN_VIEWS) === "recent",
  };
  for (const listing of ["asks", "tasks", "runs"]) state.all[listing] &&= history[listing];
  const asked = params(state.all, state.reportShown);
  try {
    // Taken before the read, so an event between that read and the visit still shows next time.
    const read = new Date().toISOString().replace(/\.\d+Z$/, "Z");
    const listings = await readAll(cache, api, asked);
    // A later call, such as the one that loads older rows, supersedes this one.
    if (call !== state.refreshes) return true;
    if (!accept(land(cache, listings, asked), { read, failed })) return true;
  } catch (error) {
    // A superseded call, such as one from before a sign-out, must not sign the new session out.
    if (call !== state.refreshes) return true;
    if (error.status === 401) return signOut("The daemon refused this token. Paste the current one.");
    state.failed = error instanceof TypeError ? "the connection dropped" : error.message;
    render();
    return false;
  }
  render();
  return true;
}

// Takes a read's listings into state, and says whether to draw them. A listing that did not change
// is the same object, so a read with nothing new only repaints the ages, once a minute at most.
export function accept(listings, { read, failed }) {
  const { views, links, desk, asks: asksList, queue, tasks: tasksList, runs: runsList, report, reports } = listings;
  if (!state.since.opened) Object.assign(state.since, { opened: read, from: desk.last_visit });
  state.since.events = mergeSince(state.since.events, desk.since_you_looked);
  // The listings are read one after another; the oldest revision makes the next wait catch what they missed.
  state.revision = Math.min(desk.revision, asksList.revision, queue.revision, tasksList.revision, runsList.revision, report?.revision ?? Infinity);
  state.updated = Date.now();
  state.failed = null;
  state.cached = false;
  if (current() === "costs") loadCosts();
  if (current() === "inbox") loadUnrouted();
  if (state.data && sameListings(listings, state.listings) && !failed && Date.now() - state.rendered < 60000) return keep(), false;
  const before = state.data ? new Set(state.data.asks.filter((ask) => ask.state === "open").map((ask) => ask.id)) : null;
  const [asks, tasks, runs] = [asksList.asks, tasksList.tasks, runsList.runs];
  const older = { asks: asksList.older, tasks: tasksList.older, runs: runsList.older };
  // The Costs panel reads its own reports; a refresh keeps the last ones.
  state.data = { views, links, desk, asks, bands: asksList.bands, queue, tasks, runs, older, report, reports, spend: state.data?.spend ?? null, flows: state.data?.flows ?? null };
  acceptReport();
  if (state.task) state.task.stale = true;
  if (state.batch) state.batch.stale = true;
  state.listings = listings;
  keep();
  announce(before);
  return true;
}

function announce(before) {
  if (!before) return;
  const fresh = state.data.asks.find((ask) => ask.state === "open" && !before.has(ask.id));
  if (fresh) document.getElementById("announce").textContent = `New ask from ${fresh.key ? taskLabel(fresh.key) : `task ${fresh.task}`}.`;
}

// The listings and their time go in one value, so a full storage never pairs old data with a new time.
export function keep() {
  try {
    localStorage.setItem("priory.snapshot", JSON.stringify({ data: state.data, updated: state.updated }));
  } catch {}
}

// The service worker reads the same copy when the push service replaces the subscription.
export const KEPT_KINDS = "/api/worker/kinds";

// The service worker cannot read local storage, so it gets its own copy of the token to tell the
// daemon about a subscription the push service replaced. Signing out drops it, and the kept kinds.
export function keepTokenForWorker(token) {
  if (!("caches" in window)) return;
  caches.open("priory-push")
    .then((cache) => token
      ? cache.put("/api/worker/token", new Response(token))
      : Promise.all(["/api/worker/token", KEPT_KINDS].map((key) => cache.delete(key))))
    .catch(() => {});
}

export function signOut(reason) {
  for (const key of ["priory.token", "priory.snapshot"]) localStorage.removeItem(key);
  keepTokenForWorker(null);
  // A refresh still in flight must not store what this device just dropped.
  state.refreshes++;
  watch.stop();
  // Before the token goes: giving the keyboard back needs it.
  keyboard.leave();
  cache.clear();
  Object.assign(state, { token: null, data: null, listings: null, revision: undefined, signInError: reason, drawn: null, flowsError: null, all: { asks: false, tasks: false, runs: false }, costs: { by: "workflow", loading: false, slow: false, error: null } });
  render();
}
