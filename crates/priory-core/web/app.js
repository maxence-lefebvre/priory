// Priory's web view (DESIGN.md): the frame, the Desk and the Asks, with answering through the CLI's verbs.
// Agent text is untrusted: every value goes through `html`, which escapes it.
import { askGroups, snoozed } from "./asks.js";

const ICONS = {
  check: '<path d="M20 6 9 17l-5-5"/>',
  minus: '<path d="M5 12h14"/>',
  x: '<path d="M18 6 6 18"/><path d="m6 6 12 12"/>',
  "circle-x": '<circle cx="12" cy="12" r="10"/><path d="m15 9-6 6"/><path d="m9 9 6 6"/>',
  "circle-dashed": '<path d="M10.1 2.18a9.93 9.93 0 0 1 3.8 0"/><path d="M17.6 3.71a9.95 9.95 0 0 1 2.69 2.7"/><path d="M21.82 10.1a9.93 9.93 0 0 1 0 3.8"/><path d="M20.29 17.6a9.95 9.95 0 0 1-2.7 2.69"/><path d="M13.9 21.82a9.94 9.94 0 0 1-3.8 0"/><path d="M6.4 20.29a9.95 9.95 0 0 1-2.69-2.7"/><path d="M2.18 13.9a9.93 9.93 0 0 1 0-3.8"/><path d="M3.71 6.4a9.95 9.95 0 0 1 2.7-2.69"/>',
  "circle-dot": '<circle cx="12" cy="12" r="10"/><circle cx="12" cy="12" r="1"/>',
  "circle-help": '<circle cx="12" cy="12" r="10"/><path d="M9.09 9a3 3 0 0 1 5.83 1c0 2-3 3-3 3"/><path d="M12 17h.01"/>',
  pause: '<rect x="14" y="4" width="4" height="16" rx="1"/><rect x="6" y="4" width="4" height="16" rx="1"/>',
  moon: '<path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z"/>',
  hand: '<path d="M18 11V6a2 2 0 0 0-2-2a2 2 0 0 0-2 2"/><path d="M14 10V4a2 2 0 0 0-2-2a2 2 0 0 0-2 2v2"/><path d="M10 10.5V6a2 2 0 0 0-2-2a2 2 0 0 0-2 2v8"/><path d="M18 8a2 2 0 1 1 4 0v6a8 8 0 0 1-8 8h-2c-2.8 0-4.5-.86-5.99-2.34l-3.6-3.6a2 2 0 0 1 2.83-2.82L7 15"/>',
  ask: '<path d="M7.9 20A9 9 0 1 0 4 16.1L2 22Z"/><path d="M9.09 9a3 3 0 0 1 5.83 1c0 2-3 3-3 3"/><path d="M12 17h.01"/>',
  archive: '<rect width="20" height="5" x="2" y="3" rx="1"/><path d="M4 8v11a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8"/><path d="M10 12h4"/>',
  "file-text": '<path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z"/><path d="M14 2v4a2 2 0 0 0 2 2h4"/><path d="M16 13H8"/><path d="M16 17H8"/>',
  "hard-drive": '<path d="M22 12H2"/><path d="M5.45 5.11 2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.45-6.89A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11z"/><path d="M6 16h.01"/><path d="M10 16h.01"/>',
  "chevron-right": '<path d="m9 18 6-6-6-6"/>',
  "chevron-left": '<path d="m15 18-6-6 6-6"/>',
  clock: '<circle cx="12" cy="12" r="10"/><path d="M12 6v6l4 2"/>',
  "chevron-up": '<path d="m18 15-6-6-6 6"/>',
  "chevron-down": '<path d="m6 9 6 6 6-6"/>',
  desk: '<path d="M7.9 20A9 9 0 1 0 4 16.1L2 22Z"/>',
  asks: '<path d="M14 9a2 2 0 0 1-2 2H6l-4 4V4a2 2 0 0 1 2-2h8a2 2 0 0 1 2 2z"/><path d="M18 9h2a2 2 0 0 1 2 2v11l-4-4h-6a2 2 0 0 1-2-2v-1"/>',
  tasks: '<path d="M3 12h.01M3 18h.01M3 6h.01M8 12h13M8 18h13M8 6h13"/>',
  runs: '<path d="M3 3v16a2 2 0 0 0 2 2h16"/><path d="M7 7h8"/><path d="M11 12h7"/><path d="M9 17h4"/>',
  inbox: '<polyline points="22 12 16 12 14 15 10 15 8 12 2 12"/><path d="M5.45 5.11 2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.45-6.89A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11z"/>',
  reports: '<path d="M3 3v16a2 2 0 0 0 2 2h16"/><path d="M18 17V9"/><path d="M13 17V5"/><path d="M8 17v-3"/>',
  queue: '<path d="M16 5H3"/><path d="M16 12H3"/><path d="M9 19H3"/><path d="m16 16-3 3 3 3"/><path d="M21 5v12a2 2 0 0 1-2 2h-6"/>',
  terminal: '<path d="m4 17 6-6-6-6"/><path d="M12 19h8"/>',
  workflow: '<rect width="8" height="8" x="3" y="3" rx="2"/><path d="M7 11v4a2 2 0 0 0 2 2h4"/><rect width="8" height="8" x="13" y="13" rx="2"/>',
  more: '<circle cx="12" cy="12" r="1"/><circle cx="19" cy="12" r="1"/><circle cx="5" cy="12" r="1"/>',
  settings: '<path d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z"/><circle cx="12" cy="12" r="3"/>',
  pencil: '<path d="M21.2 6.8a2.8 2.8 0 0 0-4-4L3.8 16.2a2 2 0 0 0-.5.8l-1.3 4.4a.5.5 0 0 0 .6.6l4.4-1.3a2 2 0 0 0 .8-.5z"/>',
  scale: '<path d="m16 16 3-8 3 8c-.87.65-1.92 1-3 1s-2.13-.35-3-1Z"/><path d="m2 16 3-8 3 8c-.87.65-1.92 1-3 1s-2.13-.35-3-1Z"/><path d="M7 21h10"/><path d="M12 3v18"/><path d="M3 7h2c2 0 5-1 7-2 2 1 5 2 7 2h2"/>',
  history: '<path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8"/><path d="M3 3v5h5"/><path d="M12 7v5l4 2"/>',
  "message-text": '<path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/><path d="M13 8H7"/><path d="M17 12H7"/>',
  hourglass: '<path d="M5 22h14"/><path d="M5 2h14"/><path d="M17 22v-4.172a2 2 0 0 0-.586-1.414L12 12l-4.414 4.414A2 2 0 0 0 7 17.828V22"/><path d="M7 2v4.172a2 2 0 0 0 .586 1.414L12 12l4.414-4.414A2 2 0 0 0 17 6.172V2"/>',
  "corner-down-right": '<polyline points="15 10 20 15 15 20"/><path d="M4 4v7a4 4 0 0 0 4 4h12"/>',
  lock: '<rect width="18" height="11" x="3" y="11" rx="2"/><path d="M7 11V7a5 5 0 0 1 10 0v4"/>',
  "unplug": '<path d="m19 5 3-3"/><path d="m2 22 3-3"/><path d="M6.3 20.3a2.4 2.4 0 0 0 3.4 0L12 18l-6-6-2.3 2.3a2.4 2.4 0 0 0 0 3.4Z"/><path d="M7.5 13.5 10 11"/><path d="M10.5 16.5 13 14"/><path d="m12 6 6 6 2.3-2.3a2.4 2.4 0 0 0 0-3.4l-2.6-2.6a2.4 2.4 0 0 0-3.4 0Z"/>',
};

class Raw { constructor(text) { this.text = text; } }
const raw = (text) => new Raw(text);
const escapeText = (value) => String(value).replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`);
const piece = (value) => {
  if (value instanceof Raw) return value.text;
  if (Array.isArray(value)) return value.map(piece).join("");
  if (value === null || value === undefined || value === false) return "";
  return escapeText(value);
};
function html(strings, ...values) {
  return raw(strings.reduce((out, text, i) => out + text + (i < values.length ? piece(values[i]) : ""), ""));
}
const icon = (name, size = "") => raw(`<svg class="ico ${size}" viewBox="0 0 24 24" aria-hidden="true">${ICONS[name]}</svg>`);

// Task and run states: role and icon (DESIGN.md, State tile).
const STATES = {
  queued: ["muted", "circle-dashed"], held: ["hold", "pause"], ready: ["live", "circle-dot"],
  running: ["live", null], asleep: ["muted", "moon"], blocked: ["warn", "hand"], done: ["ok", "check"],
  failed: ["danger", "circle-x"], dropped: ["muted", "minus"], lost: ["warn", "unplug"],
  ask: ["wait", "ask"], answered: ["muted", "check"], withdrawn: ["muted", "minus"], prompt: ["warn", "hand"],
  refused: ["warn", "x"], moved: ["muted", "check"],
};
const role = (state) => (STATES[state] || ["muted", "circle-help"])[0];
function tile(state, size = "") {
  const [kind, name] = STATES[state] || ["muted", "circle-help"];
  return html`<span class="tile ${kind} ${size}" role="img" aria-label="${state}">${name ? icon(name) : raw('<i class="dot"></i>')}</span>`;
}
const stateWord = (state) => {
  const [kind, name] = STATES[state] || ["muted", "circle-help"];
  return html`<span class="sti t-${kind}">${name ? icon(name, "s15") : raw('<i class="dot"></i>')}${state}</span>`;
};

const PANELS = {
  desk: ["Desk", "desk"], asks: ["Asks", "asks"], tasks: ["Tasks", "tasks"], runs: ["Runs", "runs"],
  inbox: ["Inbox", "inbox"], reports: ["Reports", "reports"], queue: ["Queue", "queue"],
  terminal: ["Terminal", "terminal"], workflow: ["Workflows", "workflow"],
};
const CLI = { tasks: "priory tasks", runs: "priory tasks", inbox: "priory inbox", reports: "priory inbox", queue: "priory queue", terminal: "priory attach", workflow: "priory check" };

const state = {
  token: localStorage.getItem("priory.token"),
  data: null,
  // The listings to read in full, once the person scrolls to their older rows.
  all: { asks: false, tasks: false },
  refreshes: 0,
  updated: null,
  failed: null,
  signInError: null,
  list: "desk",
  listUrl: "/",
  snapshot: null,
  rendered: 0,
  scrolls: new Map(),
  observers: [],
  // The answer waiting out its Undo, then in flight: { id, text, timer, sending }.
  pending: null,
  // The command option the first tap armed, until the second tap or 4 s: { id, text, at, timer }.
  armed: null,
  drafts: new Map(),
  notices: new Map(),
  writing: new Set(),
};

// Times: relative under 24 h, then a date (DESIGN.md, Copy).
function age(iso) {
  if (!iso) return "";
  const minutes = Math.max(0, Math.round((Date.now() - Date.parse(iso)) / 60000));
  if (minutes < 60) return `${minutes} min`;
  if (minutes < 1440) return `${Math.floor(minutes / 60)} h`;
  return date(iso);
}
const date = (iso) => new Date(iso).toLocaleString("en", { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit", hour12: false });
const clock = (iso) => new Date(iso).toLocaleTimeString("en", { hour: "2-digit", minute: "2-digit", hour12: false });
const timeTag = (iso, text = age(iso)) => html`<time datetime="${iso}" title="${iso ? new Date(iso).toString() : ""}">${text}</time>`;
const today = (iso) => new Date(iso).toDateString() === new Date().toDateString();

// Data
async function api(path) {
  const response = await fetch(path, { headers: { Authorization: `Bearer ${state.token}` }, cache: "no-store" });
  if (response.status === 401) throw Object.assign(new Error("the token was refused"), { status: 401 });
  if (!response.ok) throw new Error(`${path} answered ${response.status}`);
  return response.json();
}

async function refresh() {
  if (!state.token) return render();
  const failed = state.failed;
  const call = ++state.refreshes;
  // Older rows load again when the person scrolls back to them.
  const history = { asks: Boolean(selectedAsk()) || (current() === "asks" && askView() !== "open"), tasks: current() === "desk" && deskView() === "all" };
  for (const listing of ["asks", "tasks"]) state.all[listing] &&= history[listing];
  try {
    // The server reads the views once, when it starts.
    const views = state.data?.views || (await api("/api/views"));
    const full = (listing) => (state.all[listing] ? `/api/${listing}?all=1` : `/api/${listing}`);
    const [desk, asksList, queue, tasksList] = await Promise.all(["/api/desk", full("asks"), "/api/queue", full("tasks")].map(api));
    // A later call, such as the one that loads older rows, supersedes this one.
    if (call !== state.refreshes) return true;
    const [asks, tasks] = [asksList.asks, tasksList.tasks];
    const older = { asks: asksList.older, tasks: tasksList.older };
    // The listings are read one after another; the oldest revision makes the next wait catch what they missed.
    state.revision = Math.min(desk.revision, asksList.revision, queue.revision, tasksList.revision);
    const snapshot = JSON.stringify([desk, asks, queue, tasks, older]);
    state.updated = Date.now();
    state.failed = null;
    // Ages read in minutes, so an unchanged state needs a new paint once a minute at most.
    if (state.data && snapshot === state.snapshot && !failed && Date.now() - state.rendered < 60000) return true;
    const before = state.data ? new Set(state.data.asks.filter((ask) => ask.state === "open").map((ask) => ask.id)) : null;
    state.data = { views, desk, asks, bands: asksList.bands, queue, tasks, older };
    state.task = null;
    state.snapshot = snapshot;
    announce(before);
  } catch (error) {
    if (error.status === 401) return signOut("The daemon refused this token. Paste the current one.");
    state.failed = error instanceof TypeError ? "the connection dropped" : error.message;
    render();
    return false;
  }
  render();
  return true;
}

function announce(before) {
  if (!before) return;
  const fresh = state.data.asks.find((ask) => ask.state === "open" && !before.has(ask.id));
  if (fresh) document.getElementById("announce").textContent = `New ask from task ${fresh.key || fresh.task}.`;
}

function signOut(reason) {
  localStorage.removeItem("priory.token");
  Object.assign(state, { token: null, data: null, snapshot: null, revision: undefined, signInError: reason, all: { asks: false, tasks: false } });
  render();
}

// Routes: /, /asks, /asks/42, /tasks, /more, /settings/appearance
const route = () => location.pathname.split("/").filter(Boolean);
function go(path) {
  history.pushState(null, "", path);
  render();
}

const layout = () => (matchMedia("(min-width: 1024px)").matches ? "wide" : matchMedia("(min-width: 600px)").matches ? "medium" : "compact");

function leaves(node) {
  if (node.panel) return [node.panel];
  return Object.values(node).flat().flatMap(leaves);
}
const view = () => state.data.views.views.default;
const panelsOf = () => [...new Set(leaves(view().layout))];

// Desk model
function deskModel() {
  const { desk, asks, queue, tasks } = state.data;
  const byId = new Map(asks.map((ask) => [ask.id, ask]));
  const items = desk.items.filter((item) => item.kind !== "queue");
  const open = items.filter((item) => item.kind === "ask").map((item) => byId.get(item.id) || { ...item, question: item.what, options: [] });
  const stopped = items.filter((item) => ["blocked", "failed", "prompt", "command"].includes(item.kind));
  const reports = items.filter((item) => item.kind === "report");
  const other = items.filter((item) => !["ask", "blocked", "failed", "prompt", "command", "report"].includes(item.kind));
  return { open, stopped, reports, other, asleep: desk.counts.asleep, banner: desk.banner, running: queue.running, queue: queue.waiting, tasks, upNext: queue.up_next, needs: items.length };
}
const openAsks = () => deskModel().open;
// The open asks the list shows, so previous, next and the ask after an answer stay in its facets.
const listedAsks = () => faceted(openAsks(), facetPicks());

// Rows
function askRow(ask, current) {
  const options = ask.options || [];
  const line3 = ask.state === "answered"
    ? html`<span>Answered <b>${ask.answer}</b></span>`
    : ask.state === "withdrawn"
      ? html`<span>Withdrawn${ask.withdrawn_why ? `: ${ask.withdrawn_why}` : ""}</span>`
      : snoozed(ask, Date.now()) ? html`<span>${icon("clock", "s15")}${snoozedUntil(ask)}</span>`
        : ask.recommendation ? html`<span>Recommends <b>${ask.recommendation}</b></span>` : html`<span>${options.length} options</span>`;
  const when = ask.state === "answered" && ask.answered_at
    ? timeTag(ask.answered_at, today(ask.answered_at) ? clock(ask.answered_at) : age(ask.answered_at))
    : timeTag(ask.since);
  return html`<a class="row ${ask.state !== "open" ? "past" : ""}" href="/asks/${ask.id}" aria-current="${current ? "true" : "false"}">
    ${tile(ask.state === "open" ? "ask" : ask.state)}
    <span class="l1"><span class="mono">${ask.key}</span><span>${ask.step}</span>${ask.command ? html`<span class="tag">Command</span>` : ""}<span class="age">${when}</span></span>
    <span class="l2">${ask.title || ask.question}</span>
    <span class="l3">${line3}</span>
    <span class="chev">${icon("chevron-right", "s15")}</span></a>`;
}

const snoozedUntil = (ask) => `Snoozed until ${today(ask.snoozed_until) ? clock(ask.snoozed_until) : date(ask.snoozed_until)}`;

// The last row of a list that left out older rows; scrolling to it loads them.
function olderRow(listing, count, wide) {
  if (!count) return "";
  const text = `Loading older ${listing}…`;
  return wide ? html`<tr class="grp" data-older="${listing}"><td colspan="6">${text}</td></tr>` : html`<p class="empty" data-older="${listing}">${text}</p>`;
}

function itemRow(item) {
  // An uncertain command, like an uncertain prompt, waits on a person to check what happened.
  const kind = ["prompt", "command"].includes(item.kind) ? "prompt" : ["blocked", "failed"].includes(item.kind) ? item.kind : item.kind === "held" ? "held" : "queued";
  return html`<div class="row">
    ${tile(kind)}
    <span class="l1"><span class="mono">${item.key || ""}</span><span>${item.step || ""}</span><span>${item.kind}</span><span class="age">${timeTag(item.since)}</span></span>
    <span class="l2 r t-${role(kind)}">${item.what}</span></div>`;
}

function taskRow(task) {
  return html`<div class="row">
    ${tile(task.state)}
    <span class="l1"><span class="mono">${task.key}</span><span>${task.workflow} ${task.step}</span><span>${task.state}</span><span class="age">${timeTag(task.since)}</span></span>
    <span class="l2 r ${task.reason ? `t-${role(task.state)}` : ""}">${task.reason || task.tldr || task.lane || ""}</span></div>`;
}

function runRow(run) {
  return html`<div class="row">
    ${tile("running")}
    <span class="l1"><span class="mono">${run.key}</span><span>${run.step}</span><span>running</span><span class="age">${timeTag(run.started)}</span></span>
    <span class="l2 r">${run.agent}, run ${run.run}</span></div>`;
}

// Desk
const DESK_VIEWS = ["needs", "running", "queue", "asleep", "all"];
// While an ask is open, the list keeps the view it was opened from.
const listSearch = () => (selectedAsk() ? new URL(state.listUrl, location.origin).search : location.search);

function deskView() {
  const name = new URLSearchParams(listSearch()).get("view");
  return DESK_VIEWS.includes(name) ? name : "needs";
}

function savedViews(list, base, facets = "") {
  return html`<nav class="views" aria-label="Saved views">${list.map(([key, label, count, on]) =>
    html`<a href="${base}?view=${key}${facets}" aria-current="${on ? "true" : "false"}">${label}${count === null ? "" : html`<span class="num">${count}</span>`}</a>`)}</nav>`;
}

function countLine(model) {
  const oldest = model.open[0];
  const parts = [];
  if (oldest) parts.push(`Oldest waits ${age(oldest.since)}.`);
  if (model.stopped.length) parts.push(`${model.stopped.length} stopped.`);
  return parts.join(" ");
}

function upNext(model) {
  const next = (model.upNext || []).slice(0, 3);
  const when = (run) => {
    if (run.kind === "queued") return "now, on room";
    const at = new Date(run.at);
    const time = clock(run.at);
    return today(run.at) ? time : `${time} ${at.toLocaleDateString("en", { weekday: "short" })}`;
  };
  const what = (run) => html`<span class="mono">${run.key || run.trigger}</span> ${run.why}`;
  const body = next.length
    ? html`<ol>${next.map((run) => html`<li>${timeTag(run.at, when(run))}${run.task ? html`<a href="/tasks/${run.task}">${what(run)}</a>` : html`<span>${what(run)}</span>`}</li>`)}</ol>`
    : html`<p>Nothing is queued or scheduled to run next.</p>`;
  return html`<div class="sec">Up next<span>next 3 runs</span></div><div class="strip">${body}</div>`;
}

function deskBody(wide) {
  const model = deskModel();
  const current = deskView();
  const views = savedViews([
    ["needs", "Needs me", model.needs, current === "needs"],
    ["running", "Running", model.running.length, current === "running"],
    ...(wide ? [["queue", "Queue", model.queue.length, current === "queue"]] : []),
    ["asleep", "Asleep", model.asleep, current === "asleep"],
    ["all", "All", wide ? model.tasks.length + olderTasks() : null, current === "all"],
  ], "/");
  const banner = model.banner ? html`<div class="banner" role="status">${icon("hard-drive", "s20")}<span>${model.banner}</span></div>` : "";
  let rows;
  if (current === "running") rows = model.running.length ? model.running.map(runRow) : html`<p class="empty">No run is live. Runs show here while an agent works.</p>`;
  else if (current === "queue") rows = listOrEmpty(model.queue.map(taskRow), html`The queue is empty. A task waits here for its turn; see <code>priory queue</code>.`);
  else if (current === "asleep") rows = listOrEmpty(model.tasks.filter((task) => task.state === "asleep").map(taskRow), "No task is asleep. A task sleeps here while it waits on an event.");
  else if (current === "all") rows = model.tasks.length || olderTasks() ? [...model.tasks.map(taskRow), olderRow("tasks", olderTasks(), false)] : listOrEmpty([], html`No tasks yet. Start one with <code>priory request</code>.`);
  else rows = needsMe(model, wide);
  return { model, views, banner, rows };
}

const olderTasks = () => state.data.older.tasks;
const listOrEmpty =(rows, empty) => (rows.length ? rows : html`<p class="empty">${empty}</p>`);

function needsMe(model, wide) {
  const selected = selectedAsk();
  const sections = [];
  if (!model.open.length) sections.push(html`<p class="empty">No open asks. Agents ask here when they need you.</p>`);
  if (wide) {
    sections.push(deskTable(model, selected));
  } else {
    if (model.open.length) sections.push(html`<div class="sec">Asks<span>oldest first</span></div>${model.open.map((ask) => askRow(ask, String(ask.id) === selected))}`);
    if (model.stopped.length) sections.push(html`<div class="sec">Stopped<span>${model.stopped.length}</span></div>${model.stopped.map(itemRow)}`);
    if (model.other.length) sections.push(html`<div class="sec">Also waiting<span>${model.other.length}</span></div>${model.other.map(itemRow)}`);
  }
  sections.push(upNext(model));
  if (model.asleep) sections.push(html`<div class="sec"></div><a class="row one" href="/?view=asleep">${tile("asleep")}<span>${model.asleep} tasks asleep on waits</span><span class="num">${model.asleep}</span><span class="chev">${icon("chevron-right", "s15")}</span></a>`);
  if (model.reports.length) sections.push(html`<div class="sec">Reports<span>${model.reports.length}</span></div>${model.reports.map(itemRow)}`);
  return sections;
}

function deskTable(model, selected) {
  const askLine = (ask) => html`<tr data-href="/asks/${ask.id}" aria-current="${String(ask.id) === selected ? "true" : "false"}">
    <td>${stateWord("ask")}</td><td class="mono">${ask.key}</td><td>${ask.step}</td><td class="q"><span>${ask.title || ask.question}</span></td>
    <td class="recw">${ask.recommendation || ""}</td><td class="r">${timeTag(ask.since)}</td></tr>`;
  const itemLine = (item) => html`<tr>
    <td>${stateWord(item.kind === "prompt" ? "prompt" : item.kind)}</td><td class="mono">${item.key || ""}</td><td>${item.step || ""}</td>
    <td class="q full t-${role(item.kind)}"><span>${item.what}</span></td><td class="recw">${item.kind === "failed" ? "Revive" : item.kind === "blocked" ? "Resume" : ""}</td><td class="r">${timeTag(item.since)}</td></tr>`;
  const group = (title, count) => html`<tr class="grp"><td colspan="6">${title}<span>${count}</span></td></tr>`;
  return html`<table class="tbl">
    <thead><tr><th>State</th><th>Task</th><th>Step</th><th>Waits on you</th><th>Recommends</th><th class="r">Age</th></tr></thead>
    <tbody>${model.open.map(askLine)}
    ${model.stopped.length ? [group("Stopped", model.stopped.length), model.stopped.map(itemLine)] : ""}
    ${model.other.length ? [group("Also waiting", model.other.length), model.other.map(itemLine)] : ""}
    </tbody></table>`;
}

function capsule(model) {
  if (!model.open.length || deskView() !== "needs") return "";
  const n = model.open.length;
  return html`<div class="fade"></div><a class="capsule" href="/asks/${model.open[0].id}">${icon("ask", "s20")}Answer ${n} ${n === 1 ? "ask" : "asks"}</a>`;
}

// Asks list
const ASK_VIEWS = ["open", "answered", "all"];
function askView() {
  const name = new URLSearchParams(listSearch()).get("view");
  return ASK_VIEWS.includes(name) ? name : "open";
}

function askTableRow(ask, selected) {
  const answered = ask.state === "answered";
  const when = answered && ask.answered_at ? timeTag(ask.answered_at, today(ask.answered_at) ? clock(ask.answered_at) : age(ask.answered_at)) : timeTag(ask.since);
  return html`<tr data-href="/asks/${ask.id}" aria-current="${String(ask.id) === selected ? "true" : "false"}">
    <td>${stateWord(ask.state === "open" ? "ask" : ask.state)}</td><td class="mono">${ask.key}</td><td>${ask.step}</td><td class="q"><span>${ask.title || ask.question}</span></td>
    <td class="recw">${answered ? ask.answer : ask.state === "withdrawn" ? ask.withdrawn_why || "" : snoozed(ask, Date.now()) ? snoozedUntil(ask) : ask.recommendation || ""}</td><td class="r">${when}</td></tr>`;
}

// Each facet's key, label, an ask's values and the order the picker offers them. An ask with no
// band, such as an outside ask, shows under `none`, and so does an ask with no risk.
const FACETS = [
  ["band", "Band", (ask) => [ask.band ?? "none"], () => [...(state.data.bands || []), "none"]],
  ["risk", "Risk", (ask) => (ask.risk?.length ? ask.risk : ["none"]), () => ["irreversible", "red", "yellow", "green", "none"]],
];
const facetPicks = () => {
  const search = new URLSearchParams(listSearch());
  return new Map(FACETS.map(([key]) => [key, search.get(key)]).filter(([, value]) => value));
};
const faceted = (asks, picks) => asks.filter((ask) => FACETS.every(([key, , values]) => !picks.has(key) || values(ask).includes(picks.get(key))));
const facetQuery = (picks) => [...picks].map(([key, value]) => `&${key}=${encodeURIComponent(value)}`).join("");

function facet(view, picks, [key, label, values, order]) {
  const others = new Map([...picks].filter(([other]) => other !== key));
  const picked = picks.get(key);
  if (picked) return html`<a class="facet" aria-current="true" href="/asks?view=${view}${facetQuery(others)}" aria-label="${label}: ${picked}. Clear">${picked}${icon("x", "s13")}</a>`;
  const counts = new Map();
  for (const ask of state.data.asks) for (const value of values(ask)) counts.set(value, (counts.get(value) || 0) + 1);
  const offered = [...counts].sort(([a], [b]) => order().indexOf(a) - order().indexOf(b));
  return html`<details class="facet"><summary aria-label="Filter by ${key}">${label}${icon("chevron-down", "s13")}</summary>
    <div class="pick">${offered.map(([name, count]) => html`<a href="/asks?view=${view}${facetQuery(others)}&${key}=${encodeURIComponent(name)}">${name}<span class="num">${count}</span></a>`)}</div></details>`;
}

function asksBody(wide) {
  const picks = facetPicks();
  const asks = faceted(state.data.asks, picks);
  const current = askView();
  const { open, snoozed: asleep, answered, withdrawn } = askGroups(asks, Date.now());
  // shortcut: older counts are not per facet, so a facet counts the loaded rows only; split `older` by facet if that misleads.
  const pending = state.data.older.asks;
  const left = picks.size ? { answered: 0, withdrawn: 0 } : pending;
  const total = { open: open.length, answered: answered.length + left.answered, all: asks.length + left.answered + left.withdrawn };
  const unloaded = { open: 0, answered: pending.answered, all: pending.answered + pending.withdrawn }[current];
  const views = savedViews([
    ["open", "Open", total.open, current === "open"],
    ["answered", "Answered", total.answered, current === "answered"],
    ["all", "All", total.all, current === "all"],
  ], "/asks", facetQuery(picks));
  const facets = html`<div class="facets" aria-label="Facets">${FACETS.map((spec) => facet(current, picks, spec))}</div>`;
  const selected = selectedAsk();
  const section = (title, list, note, count = list.length) => {
    if (!count) return "";
    if (wide) return html`<tr class="grp"><td colspan="6">${title}<span>${note}</span></td></tr>${list.map((ask) => askTableRow(ask, selected))}`;
    return html`<div class="sec">${title}<span>${note}</span></div>${list.map((ask) => askRow(ask, String(ask.id) === selected))}`;
  };
  const answeredToday = answered.filter((ask) => ask.answered_at && today(ask.answered_at));
  const answeredEarlier = answered.filter((ask) => !answeredToday.includes(ask));
  const sections = [
    current !== "answered" ? section("Open", open, `${open.length}, oldest first`) : "",
    current === "all" ? section("Snoozed", asleep, asleep.length) : "",
    current !== "open" ? section("Answered today", answeredToday, answeredToday.length) : "",
    current !== "open" ? section("Answered earlier", answeredEarlier, answeredEarlier.length + left.answered, answeredEarlier.length + left.answered) : "",
    current === "all" ? section("Withdrawn", withdrawn, withdrawn.length + left.withdrawn, withdrawn.length + left.withdrawn) : "",
    olderRow("asks", unloaded, wide),
  ];
  const empty = { open: "No open asks. Agents ask here when they need you.", answered: "No answered asks yet. An ask moves here once someone answers it.", all: "No asks yet. Agents ask here when they need you." }[current];
  const rows = wide ? html`<table class="tbl"><thead><tr><th>State</th><th>Task</th><th>Step</th><th>Question</th><th>${current === "open" ? "Recommends" : "Answer"}</th><th class="r">Age</th></tr></thead><tbody>${sections}</tbody></table>` : sections;
  return { views: html`${views}${facets}`, rows: total[current] || unloaded ? rows : html`<p class="empty">${empty}</p>`, open };
}

// Ask detail
const selectedAsk = () => {
  const [first, id] = route();
  return first === "asks" && id ? decodeURIComponent(id) : null;
};

function askDetail(id, wide) {
  const ask = state.data.asks.find((candidate) => String(candidate.id) === id);
  const { answered, withdrawn } = state.data.older.asks;
  if (!ask && answered + withdrawn) {
    loadOlder("asks");
    return html`<p class="empty">Loading ask ${id}…</p>`;
  }
  if (!ask) return html`<p class="empty">No ask ${id} here. It may belong to another factory. Check <code>priory asks --all</code>.</p>`;
  const open = listedAsks();
  const at = open.findIndex((candidate) => String(candidate.id) === id);
  const back = [state.listUrl, state.list === "asks" ? "Asks" : "Desk"];
  const nav = (to, label, name) => (to
    ? html`<a class="iconbtn" href="/asks/${to.id}" aria-label="${label}">${icon(name, "s20")}</a>`
    : html`<span class="iconbtn" aria-disabled="true" aria-label="${label}">${icon(name, "s20")}</span>`);
  const header = html`<div class="ahead">
    ${wide ? html`<span class="ctx">${tile("ask", "sm")}<span>ask <span class="mono">#${ask.id}</span></span><span>revision ${ask.revision}</span><span>${timeTag(ask.since)}</span></span>` : html`<a class="back" href="${back[0]}">${icon("chevron-left", "s22")}${back[1]}</a>`}
    ${at >= 0 ? html`<span class="pos num">${at + 1}<span>/${open.length}</span></span>` : html`<span class="sp"></span>`}
    ${nav(open[at - 1], "Previous ask", "chevron-up")}${nav(open[at + 1], "Next ask", "chevron-down")}
    ${wide ? html`<a class="iconbtn" href="${back[0]}" aria-label="Close">${icon("x", "s20")}</a>` : ""}</div>`;
  const options = ask.options || [];
  const pending = state.pending?.id === ask.id ? state.pending : null;
  const armed = state.armed?.id === ask.id ? state.armed.text : null;
  const locked = state.failed ? "offline" : null;
  const bound = `Bound to revision ${ask.revision}.`;
  const optionButtons = options.map((option, i) => {
    const recommended = option === ask.recommendation;
    const runs = runsCommand(ask, option);
    const chosen = pending?.text === option;
    const off = locked || (pending && !chosen);
    const notes = [recommended && "Recommended", runs && "Runs a command"].filter(Boolean).join(". ");
    const label = armed === option
      ? html`<span><small>Tap again to run</small>${option}<small>Cancels in 4 s</small></span><span class="drain" aria-hidden="true"></span>`
      : html`<span>${option}${notes ? html`<small>${notes}</small>` : ""}</span>`;
    const button = html`<button class="opt ${recommended ? "rec-opt" : ""} ${chosen ? "chosen" : ""} ${armed === option ? "armed" : ""}" data-option="${i}" data-answer="${i}" aria-disabled="${off || pending ? "true" : "false"}" ${locked ? raw('aria-describedby="answer-locked"') : ""}>
      ${chosen ? icon("check", "s20") : ""}${label}${wide ? html`<kbd>${i + 1}</kbd>` : ""}</button>`;
    return armed === option ? html`${button}${commandBlock(ask, bound)}` : button;
  });
  const notice = state.notices.get(ask.id);
  const lockedNote = locked ? html`<p class="note" id="answer-locked">${icon("unplug", "s15")}<span>Reconnect to answer.</span></p>` : "";
  const writing = state.writing.has(ask.id) && !locked;
  const write = writing
    ? html`<form class="writeform" data-action="send-text"><label class="sr" for="draft">Your answer</label><textarea id="draft" name="text" rows="3">${state.drafts.get(ask.id) || ""}</textarea><button class="btn primary" ${pending ? raw('aria-disabled="true"') : ""}>Send</button></form>`
    : html`<button class="ghost write" data-action="write" aria-disabled="${locked || pending ? "true" : "false"}">${icon("pencil", "s15")}Write an answer</button>`;
  const busy = pending ? "true" : "false";
  const snoozeButtons = !ask.task || locked === "offline"
    ? ""
    : ask.snoozed_until
      ? html`<button class="ghost" data-action="unsnooze" aria-disabled="${busy}">Unsnooze</button>`
      : html`<button class="ghost" data-action="snooze" data-for="1h" aria-disabled="${busy}">Snooze 1 h</button><button class="ghost" data-action="snooze" data-tomorrow="9" aria-disabled="${busy}">Snooze until tomorrow 09:00</button>`;
  const more = html`<div class="more">${write}${locked ? "" : html`<button class="ghost" data-action="dismiss" aria-disabled="${busy}">Dismiss</button>`}${snoozeButtons}</div>`;
  const decided = ask.state !== "open"
    ? html`${notice ? html`<p class="warnline" role="status">${notice}</p>` : ""}<div class="rec">${ask.state === "answered" ? html`<span><b>Answered: ${ask.answer}.</b> ${ask.answered_at ? age(ask.answered_at) + " ago" : ""}</span>` : html`<span><b>Withdrawn${ask.withdrawn_why ? `: ${ask.withdrawn_why}` : ""}.</b></span>`}</div>${ranLine(ask.ran)}`
    : html`${notice ? html`<p class="warnline" role="status">${notice}</p>` : ""}${ask.recommendation ? html`<div class="rec"><span><b>Recommended: ${ask.recommendation}.</b></span>
        ${ask.next_action ? html`<span class="then">${icon("corner-down-right", "s15")}<span>${ask.next_action}</span></span>` : ""}</div>` : ""}
      <div class="opts" role="group" aria-label="Options">${optionButtons}</div>${lockedNote}${more}`;
  const top = html`<div class="ask">
    <a class="ctx" href="${ask.task ? `/tasks/${ask.task}` : "/asks"}">${wide ? "" : tile("ask", "sm")}<span class="mono">${ask.key}</span><span>${ask.step}</span><span>${ask.lane || ""}</span>${ask.command ? html`<span class="tag">Command</span>` : ""}${wide ? "" : html`<span>${timeTag(ask.since)}</span><span>revision ${ask.revision}</span>`}</a>
    <h2 class="question">${ask.question || ask.title}</h2>${decided}</div>`;
  const read = taskSections(taskOf(ask));
  const sections = [["why", "Why"], ...read.map(([key, label]) => [key, label]), ["history", "History"], ...(ask.command ? [["command", "Command"]] : [])];
  const jump = html`<nav class="jump" aria-label="Sections">${sections.map(([key, label], i) => html`<a href="#${key}" aria-current="${i === 0 ? "true" : "false"}">${label}</a>`)}</nav>`;
  const why = html`<section class="sect" id="why"><div class="sh">${icon("message-text", "s15")}What the agent asks<span class="meta">${ask.run ? html`run <span class="mono">${ask.run}</span>` : ""}</span></div>
    <p class="text">${ask.text || ask.question}</p>
    ${ask.tradeoff ? html`<p class="fact">${icon("scale", "s15")}<span><b>Trade-off.</b> ${ask.tradeoff}</span></p>` : ""}
    ${ask.what_waits ? html`<p class="fact">${icon("hourglass", "s15")}<span><b>What waits.</b> ${ask.what_waits}</span></p>` : ""}</section>`;
  const command = !ask.command ? "" : typeof ask.command === "string"
    ? html`<section class="sect" id="command"><div class="sh">${icon("terminal", "s15")}Command the asker attached</div>
      <div class="cmd"><span class="mono">$ ${ask.command}</span><small>Priory stores this command and never runs it: the ask came from outside Priory.</small></div></section>`
    : html`<section class="sect" id="command"><div class="sh">${icon("terminal", "s15")}Command an answer runs</div>
      ${commandBlock(ask, `Runs on ${ask.command.runs.join(", ")}. ${bound} Runs at most once.`)}</section>`;
  // Every revision and refused answer, newest first; an ask listed before Priory kept them shows its current revision.
  const dated = [
    ...(ask.revisions || [{ revision: ask.revision, at: ask.since, answer: ask.answer, answered_at: ask.answered_at }]).flatMap((revision) => [
      [revision.at, html`<li>${tile("ask", "sm")}<span>Asked revision <b>${revision.revision}</b>${revision.question ? `: ${revision.question}` : ""}</span>${timeTag(revision.at, clock(revision.at))}</li>`],
      ...(revision.answer == null ? [] : [[revision.answered_at || revision.at, html`<li>${tile("answered", "sm")}<span>Answered <b>${revision.answer}</b></span>${timeTag(revision.answered_at, clock(revision.answered_at || revision.at))}</li>`]]),
    ]),
    ...(ask.refused || []).map((refusal) => [refusal.at, html`<li>${tile("refused", "sm")}<span class="t-warn"><b>Refused:</b> ${refusal.answer}, sent for revision ${refusal.revision} from ${refusal.by}: ${refusal.why}</span>${timeTag(refusal.at, clock(refusal.at))}</li>`]),
  // Built oldest first; reversed, entries of the same second stay newest first through the stable sort.
  ].reverse().sort(([a], [b]) => b.localeCompare(a));
  const historyItems = dated.map(([, item]) => item);
  if (ask.state === "withdrawn") historyItems.unshift(html`<li>${tile("withdrawn", "sm")}<span>Withdrawn${ask.withdrawn_why ? `: ${ask.withdrawn_why}` : ""}</span><span></span></li>`);
  const history = html`<section class="sect" id="history"><div class="sh">${icon("history", "s15")}Answer history</div><ol class="hist">${historyItems}</ol></section>`;
  const recommended = options.find((option) => option === ask.recommendation) || options[0];
  const sending = pending
    ? html`<div class="sending" role="status"><span>Sending <b>"${pending.text}"</b></span>${pending.sending ? "" : html`<button class="btn secondary" data-action="undo">Undo</button>`}</div>`
    : "";
  const barArmed = armed !== null && armed === recommended;
  const bar = !wide && !pending && ask.state === "open" && options.length
    ? html`<div class="abar" hidden>${barArmed ? commandBlock(ask, bound) : ""}<button class="btn secondary" data-action="options">${icon("chevron-up", "s15")}${options.length} options</button>
      <button class="btn primary ${barArmed ? "armed" : ""}" data-answer="${options.indexOf(recommended)}" aria-disabled="${locked ? "true" : "false"}">${barArmed
        ? html`<span><small>Tap again to run</small>${recommended}<small>Cancels in 4 s</small></span><span class="drain" aria-hidden="true"></span>`
        : html`<span>${recommended}</span><small>${[recommended === ask.recommendation ? "Recommended" : "First option", runsCommand(ask, recommended) && "Runs a command"].filter(Boolean).join(". ")}</small>`}</button>
      ${locked ? html`<p class="note">${icon("unplug", "s15")}<span>Reconnect to answer.</span></p>` : ""}</div>`
    : "";
  const reading = read.map(([, , section]) => section);
  if (wide) return html`<div class="pinned">${header}${top}${sending}</div><div class="scroll" data-sections>${jump}${why}${reading}${history}${command}</div>`;
  return html`${header}<div class="scroll" data-sections>${top}${jump}${why}${reading}${history}${command}</div>${bar}${sending}`;
}

// The ask's task as /api/tasks/{id} serves it, read once per ask and again after each refresh.
function taskOf(ask) {
  if (!ask.task) return null;
  if (state.task?.id === ask.task) return state.task.body;
  state.task = { id: ask.task, body: null };
  api(`/api/tasks/${ask.task}`).then((body) => {
    if (state.task?.id !== ask.task) return;
    state.task.body = body;
    render();
  }, () => {});
  return null;
}

// Brief, Plan, PR and Trace, each left out when the task lacks it. Agent text shows as plain text.
function taskSections(task) {
  if (!task) return [];
  const sections = [];
  const sect = (key, ico, title, meta, body) => html`<section class="sect" id="${key}"><div class="sh">${icon(ico, "s15")}${title}<span class="meta">${meta}</span></div>${body}</section>`;
  const { request, result } = task.brief || {};
  if (request || result) {
    sections.push(["brief", "Brief", sect("brief", "file-text", "Brief", result ? `${result.step}, run ${result.run}` : "",
      html`${request ? html`<p class="meta">The request</p><p class="text">${request}</p>` : ""}${result ? html`<p class="text">${result.summary}</p>` : ""}`)]);
  }
  const progress = task.progress;
  const plan = task.plan ? [`${task.plan.step}, run ${task.plan.run}`, task.plan.summary]
    : progress ? ["progress", [progress.goal && `Goal: ${progress.goal}`, ...(progress.acceptanceCriteria || []).map((c) => `- ${c.id}: ${c.description}`),
      progress.latestChange && `Latest change: ${progress.latestChange}`, progress.nextAction && `Next: ${progress.nextAction}`].filter(Boolean).join("\n")] : null;
  if (plan) sections.push(["plan", "Plan", sect("plan", "check", "Plan", plan[0], html`<p class="text">${plan[1]}</p>`)]);
  const pr = task.pull_request;
  if (pr) {
    sections.push(["pr", "PR", sect("pr", "workflow", `#${pr.number} ${pr.title || ""}`, pr.review || "",
      html`<p class="meta">${pr.branch} into ${pr.base}, head <span class="mono">${String(pr.head_sha).slice(0, 7)}</span>, ${pr.state}</p>
      <ol class="hist">${pr.checks.map((check) => html`<li>${tile(check.conclusion === "success" ? "done" : "failed", "sm")}<span>${check.name}</span><span>${check.conclusion}</span></li>`)}</ol>`)]);
  }
  if (task.runs?.length) {
    sections.push(["trace", "Trace", sect("trace", "runs", "Trace", "", html`<ol class="hist">${task.runs.map((run) => {
      const tokens = run.tokens ? Object.values(run.tokens).reduce((sum, count) => sum + (Number(count) || 0), 0) : null;
      const minutes = Math.round(((run.ended ? Date.parse(run.ended) : Date.now()) - Date.parse(run.started)) / 60000);
      return html`<li>${tile(run.ended ? run.end || "moved" : "running", "sm")}<span><b>${run.step}</b>, run <span class="mono">${run.id}</span>, ${minutes} min${run.outcome ? `, ${run.outcome}` : ""}${tokens === null ? "" : `, ${tokens} tokens`}</span>${timeTag(run.started, clock(run.started))}</li>`;
    })}</ol>`)]);
  }
  return sections;
}

// An agent's command runs on the options in its `runs`; an outside ask's string command never runs.
const runsCommand = (ask, option) => Array.isArray(ask.command?.runs) && ask.command.runs.includes(option);

// The argv word by word, each in its own box, so a space, a quote or an empty word shows as it is.
function commandBlock(ask, note) {
  const { argv, target, dir } = ask.command;
  return html`<div class="cmd"><ol class="argv" aria-label="Command, word by word">${argv.map((word) => html`<li><code>${word}</code></li>`)}</ol>
    <small>In <span class="mono">${dir}</span> (${target}). ${note}</small></div>`;
}

const RAN = {
  running: "The command runs.",
  done: "The command ran: exit 0.",
  failed: "The command failed",
  uncertain: "Priory stopped while this ran. Check the target before you act on it.",
};

function ranLine(ran) {
  if (!ran) return "";
  const text = ran.status === "failed" ? `${RAN.failed}: ${ran.error}.` : RAN[ran.status] || ran.status;
  return html`<p class="${ran.status === "uncertain" ? "warnline" : "note"}" role="status">${icon("terminal", "s15")}<span>${text}</span></p>`;
}

// Writes: each is the twin of a CLI verb, sent with the revision the page showed.
const UNDO_MS = 4000;
const ARM_MS = 4000;
const askOnScreen = () => state.data.asks.find((ask) => String(ask.id) === selectedAsk());

async function post(path, body) {
  const response = await fetch(path, {
    method: "POST",
    headers: { Authorization: `Bearer ${state.token}`, "Content-Type": "application/json" },
    body: JSON.stringify(body),
    cache: "no-store",
  });
  return { status: response.status, body: await response.json().catch(() => ({})) };
}

// Nothing leaves the page until the Undo time passes; then Undo goes, since the daemon may apply it.
function answer(ask, text, written) {
  if (state.pending) return;
  state.notices.delete(ask.id);
  const pending = { id: ask.id, text, sending: false };
  pending.timer = setTimeout(() => send(ask, pending, written), UNDO_MS);
  state.pending = pending;
  render();
}

function undo() {
  if (!state.pending || state.pending.sending) return;
  clearTimeout(state.pending.timer);
  state.pending = null;
  render();
}

// The first tap on a command option arms it; the second, within 4 s, runs it with no Undo.
function arm(ask, option) {
  disarm();
  state.armed = { id: ask.id, text: option, at: Date.now(), timer: setTimeout(() => { state.armed = null; render(); }, ARM_MS) };
  render();
}

function disarm() {
  if (!state.armed) return;
  clearTimeout(state.armed.timer);
  state.armed = null;
}

function runCommand(ask, option) {
  disarm();
  if (state.pending) return;
  state.notices.delete(ask.id);
  state.pending = { id: ask.id, text: option, sending: false };
  send(ask, state.pending, false, true);
}

async function send(ask, pending, written, confirmed = false) {
  pending.sending = true;
  render();
  const body = { [written ? "text" : "option"]: pending.text, revision: ask.revision };
  if (confirmed) body.run = true;
  const sent = await write(ask, `/api/asks/${encodeURIComponent(ask.id)}/answer`, body, pending.text);
  state.pending = null;
  if (sent) return next(ask);
  render();
}

// True once the daemon applied the write; else the reason shows on the ask and the draft stays.
async function write(ask, path, body, draft) {
  let reply;
  try {
    reply = await post(path, body);
  } catch {
    reply = { status: 0, body: { error: "the connection dropped" } };
  }
  const { status, body: answered } = reply;
  if (status === 200) {
    state.drafts.delete(ask.id);
    state.writing.delete(ask.id);
    state.notices.delete(ask.id);
    return true;
  }
  if (status === 401) {
    signOut("The daemon refused this token. Paste the current one.");
    return false;
  }
  if (draft) {
    state.drafts.set(ask.id, draft);
    state.writing.add(ask.id);
  }
  const fresh = answered.ask;
  if (status === 409 && fresh) {
    state.data.asks = state.data.asks.map((candidate) => (candidate.id === fresh.id ? fresh : candidate));
    state.notices.set(ask.id, fresh.state === "open"
      ? `The question changed. Read revision ${fresh.revision} and answer again.`
      : fresh.state === "answered"
        ? `Already answered ${age(fresh.answered_at)} ago: ${fresh.answer}.`
        : `Withdrawn: ${fresh.withdrawn_why}.`);
  } else if (status === 503) {
    state.notices.set(ask.id, "Could not send: no priory daemon runs beside this server. Start priory daemon, then send again.");
  } else if (status === 504 || status === 0) {
    state.notices.set(ask.id, "The daemon may have applied this but did not confirm it. Check the ask's state before you send again.");
    refresh();
  } else {
    state.notices.set(ask.id, `Could not send: ${answered.error || `the server answered ${status}`}.`);
  }
  return false;
}

// After an answer, the next open ask opens, or the list once none is left.
async function next(ask) {
  const before = listedAsks().map((candidate) => candidate.id);
  await refresh();
  if (selectedAsk() !== String(ask.id)) return;
  const left = listedAsks().filter((candidate) => candidate.id !== ask.id);
  const following = left.find((candidate) => before.indexOf(candidate.id) > before.indexOf(ask.id)) || left[0];
  go(following ? `/asks/${encodeURIComponent(following.id)}` : state.listUrl);
}

async function snooze(ask, button) {
  const body = { kind: "ask", id: ask.id };
  if (button.dataset.action === "unsnooze") body.clear = true;
  else if (button.dataset.for) body.for = button.dataset.for;
  else {
    const at = new Date();
    at.setDate(at.getDate() + 1);
    at.setHours(Number(button.dataset.tomorrow), 0, 0, 0);
    body.until = at.toISOString();
  }
  if (!(await write(ask, "/api/snooze", body, null))) return render();
  if (body.clear) return refresh();
  next(ask);
}

async function dismiss(ask) {
  if (await write(ask, `/api/asks/${encodeURIComponent(ask.id)}/dismiss`, { revision: ask.revision }, null)) return next(ask);
  render();
}

function act(target) {
  const ask = askOnScreen();
  if (!ask || target.getAttribute("aria-disabled") === "true") return;
  if (target.dataset.answer !== undefined) {
    const option = ask.options[Number(target.dataset.answer)];
    if (!runsCommand(ask, option)) {
      disarm();
      return answer(ask, option, false);
    }
    if (state.armed?.id === ask.id && state.armed.text === option) return runCommand(ask, option);
    return arm(ask, option);
  }
  const action = target.dataset.action;
  if (action === "undo") return undo();
  if (state.pending) return;
  if (action === "options") return document.querySelector(".opts")?.scrollIntoView({ block: "center" });
  if (action === "write") {
    state.writing.add(ask.id);
    render();
    return document.getElementById("draft")?.focus();
  }
  if (action === "dismiss") return dismiss(ask);
  if (action === "snooze" || action === "unsnooze") return snooze(ask, target);
}

// Frame pieces
function connection() {
  const off = Boolean(state.failed);
  return html`<span class="conn ${off ? "off" : ""}">${off ? "reconnecting" : "live"}</span>`;
}

function staleStrip() {
  if (!state.failed) return "";
  const since = state.updated ? `Last update ${age(new Date(state.updated).toISOString())} ago.` : "";
  return html`<div class="stale" role="status">${icon("unplug", "s15")}<span>Reconnecting: ${state.failed}. ${since}</span><button class="ghost" data-action="retry">Retry</button></div>`;
}

function tabs(current, kind) {
  const open = state.data ? openAsks().length : 0;
  const panels = panelsOf();
  const shown = kind === "rail" ? panels : panels.slice(0, 4);
  const link = (panel) => {
    const [label, name] = PANELS[panel];
    const href = panel === "desk" ? "/" : `/${panel}`;
    return html`<a href="${href}" aria-current="${current === panel ? "page" : "false"}">${icon(name, "s20")}${label}${panel === "desk" && open ? html`<span class="badge num" aria-label="${open} open asks">${open}</span>` : ""}</a>`;
  };
  if (kind === "rail") {
    return html`<nav class="rail" aria-label="Panels">${shown.map(link)}<a class="settings" href="/settings/appearance" aria-current="${current === "settings" ? "page" : "false"}">${icon("settings", "s20")}Settings</a></nav>`;
  }
  return html`<nav class="tabbar" aria-label="Panels">${shown.map(link)}<a href="/more" aria-current="${current === "more" || current === "settings" ? "page" : "false"}">${icon("more", "s20")}More</a></nav>`;
}

function laterPanel(panel) {
  const [label] = PANELS[panel];
  return html`<p class="empty">The ${label} panel comes in a later version of the web view. Meanwhile run <code>${CLI[panel]}</code>.</p>`;
}

function morePage() {
  const rest = panelsOf().slice(4);
  return html`<div class="top"><h1>More</h1></div><div class="scroll"><ul class="links">
    ${rest.map((panel) => html`<li><a href="/${panel}">${icon(PANELS[panel][1], "s20")}${PANELS[panel][0]}<span class="chev">${icon("chevron-right", "s15")}</span></a></li>`)}
    <li><a href="/settings/appearance">${icon("settings", "s20")}Appearance<span class="chev">${icon("chevron-right", "s15")}</span></a></li>
    <li><a href="/" data-action="sign-out">${icon("lock", "s20")}Sign out of this device<span class="chev">${icon("chevron-right", "s15")}</span></a></li></ul></div>`;
}

// Appearance
const THEMES = [
  ["indigo", "Indigo", "The default. Ink emphasis with an indigo action."],
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
    <p class="note">A theme changes color and type only. Every screen keeps the same layout.</p></div></div>`;
}

// Screens
function deskPhone(withCapsule = true) {
  const { model, views, banner, rows } = deskBody(false);
  const n = model.open.length;
  return html`<div class="top"><span class="fac">${state.data.views.factory}</span>${connection()}</div>
    <div class="panel" role="region" aria-label="Desk"><div class="scroll ${withCapsule ? "desk-end" : ""}">
      <div class="count"><span class="n num">${n}</span><span class="w">${n === 1 ? "ask needs you" : "asks need you"}<span>${countLine(model)}</span></span></div>
      ${views}${banner}${rows}</div>${withCapsule ? capsule(model) : ""}</div>`;
}

function asksPhone() {
  const { views, rows } = asksBody();
  return html`<div class="top"><h1>Asks</h1><span class="sp"></span>${connection()}</div>
    <div class="panel" role="region" aria-label="Asks"><div class="scroll">${views}${rows}</div></div>`;
}

function listPanel(panel, wide) {
  if (panel === "desk") {
    const { model, views, banner, rows } = deskBody(wide);
    const n = model.open.length;
    return html`<div class="list" role="region" aria-label="Desk"><div class="lhead"><h1>Desk</h1>
      <span class="cnt"><span class="n num">${n}</span><span class="w">${n === 1 ? "ask needs you" : "asks need you"}<span>${countLine(model)}</span></span></span></div>
      ${views}${banner}<div class="scroll">${rows}</div>${wide ? keyHints() : ""}</div>`;
  }
  if (panel === "asks") {
    const { views, rows } = asksBody(wide);
    return html`<div class="list" role="region" aria-label="Asks"><div class="lhead"><h1>Asks</h1></div>${views}<div class="scroll">${rows}</div>${wide ? keyHints() : ""}</div>`;
  }
  return html`<div class="list" role="region" aria-label="${PANELS[panel][0]}"><div class="lhead"><h1>${PANELS[panel][0]}</h1></div>${laterPanel(panel)}</div>`;
}

const keyHints = () => html`<div class="keys"><span><kbd>J</kbd><kbd>K</kbd>move</span><span><kbd>Enter</kbd>open</span><span><kbd>Esc</kbd>close</span></div>`;

function drawer(wide) {
  const id = selectedAsk();
  const body = id ? askDetail(id, wide) : html`<p class="empty">Select an ask to read it here.</p>`;
  return html`<aside class="drawer" aria-label="Ask">${body}</aside>`;
}

function current() {
  const [first] = route();
  if (!first || first === "index.html") return "desk";
  if (first === "settings") return "settings";
  return first;
}

// The view file's tree, for a layout that is not a plain set of tabs.
function tree(node, list) {
  if (node.panel) return listPanel(node.panel, true);
  const [kind, children] = Object.entries(node)[0];
  if (kind !== "tabs") return html`<div class="tree tree-${kind}">${children.map((child) => tree(child, list))}</div>`;
  const shown = children.find((child) => leaves(child).includes(list)) || children[0];
  const strip = children.map((child) => {
    const panel = leaves(child)[0];
    return html`<a href="${panel === "desk" ? "/" : `/${panel}`}" aria-current="${child === shown ? "page" : "false"}">${icon(PANELS[panel][1], "s20")}${PANELS[panel][0]}</a>`;
  });
  return html`<div class="tree-tabs"><nav class="tabbar" aria-label="Tabs">${strip}</nav>${tree(shown, list)}</div>`;
}

function signIn() {
  return html`<form class="signin" data-action="sign-in">
    <h1>Priory</h1>
    <p>Paste the token this factory's daemon was started with. It is the value of <code>PRIORY_TOKEN</code>, or the variable <code>[serve] token_env</code> names. This device keeps it until you sign out.</p>
    <label for="token">Token</label>
    <input id="token" name="token" type="password" autocomplete="current-password" required autofocus>
    ${state.signInError ? html`<p class="field-error">${icon("circle-x", "s15")}${state.signInError}</p>` : ""}
    <button class="btn primary" type="submit">Sign in</button></form>`;
}

function loading() {
  return state.failed
    ? html`${staleStrip()}<p class="empty">Could not load the desk: ${state.failed}. Check that <code>priory serve</code> or the daemon runs, then retry.</p>`
    : "";
}

const routeKey = () => location.pathname + location.search;
const FOCUS_ATTRIBUTES = ["id", "href", "data-option", "data-pick-mode", "data-theme-pick", "data-action"];
function focusKey(element) {
  if (!element || element === document.body) return null;
  const name = FOCUS_ATTRIBUTES.find((attribute) => element.hasAttribute(attribute));
  return name ? `[${name}="${CSS.escape(element.getAttribute(name))}"]` : null;
}

function render() {
  const app = document.getElementById("app");
  // Leaving the ask, or signing out, cancels an answer that has not left yet.
  if (state.pending && !state.pending.sending && (!state.token || selectedAsk() !== String(state.pending.id))) {
    clearTimeout(state.pending.timer);
    state.pending = null;
  }
  if (state.renderedKey) state.scrolls.set(state.renderedKey, [...app.querySelectorAll(".scroll")].map((element) => element.scrollTop));
  const focused = focusKey(document.activeElement);
  const typed = app.querySelector("#token")?.value;
  const caret = document.activeElement?.id === "draft" ? document.activeElement.selectionStart : undefined;
  state.observers.forEach((observer) => observer.disconnect());
  state.observers = [];
  if (state.armed && (!state.token || selectedAsk() !== String(state.armed.id))) disarm();
  app.innerHTML = page().text;
  // A render must not restart the countdown; CSP allows no inline style, so it is set here.
  const elapsed = state.armed ? Date.now() - state.armed.at : 0;
  app.querySelectorAll(".drain").forEach((drain) => { drain.style.animationDelay = `-${elapsed}ms`; });
  state.renderedKey = routeKey();
  state.rendered = Date.now();
  const scrolls = state.scrolls.get(state.renderedKey) || [];
  app.querySelectorAll(".scroll").forEach((element, i) => { element.scrollTop = scrolls[i] || 0; });
  if (typed && app.querySelector("#token")) app.querySelector("#token").value = typed;
  if (focused) [...app.querySelectorAll(focused)].find((element) => !element.closest("[inert]"))?.focus({ preventScroll: true });
  if (caret !== undefined) app.querySelector("#draft")?.setSelectionRange(caret, caret);
  for (const row of app.querySelectorAll("tr[data-href]")) row.addEventListener("click", () => go(row.dataset.href));
  watchSections(app);
}

function page() {
  if (!state.token) return signIn();
  if (!state.data) return loading();
  const size = layout();
  const here = current();
  if (["desk", "asks"].includes(here) && !selectedAsk()) Object.assign(state, { list: here, listUrl: routeKey() });
  if (size === "compact") {
    let body;
    if (selectedAsk()) body = askDetail(selectedAsk(), false);
    else if (here === "desk") body = deskPhone();
    else if (here === "asks") body = asksPhone();
    else if (here === "more") body = morePage();
    else if (here === "settings") body = html`<div class="top"><a class="back" href="/more">${icon("chevron-left", "s22")}More</a></div>${appearance()}`;
    else if (PANELS[here]) body = html`<div class="top"><h1>${PANELS[here][0]}</h1></div>${laterPanel(here)}`;
    else body = notFound();
    const tab = selectedAsk() ? null : here === "settings" ? "more" : here;
    return html`${staleStrip()}${body}${tab ? tabs(tab, "bar") : ""}`;
  }
  const list = selectedAsk() ? state.list : here;
  const top = html`<header class="wtop"><span class="fac">${state.data.views.factory}</span>${connection()}</header>`;
  const wide = size === "wide";
  const utility = here === "settings" ? appearance() : here === "more" ? morePage() : PANELS[list] ? null : notFound();
  const nav = (kind) => tabs(here === "settings" && wide ? "settings" : here === "settings" ? "more" : list, kind);
  if (utility) {
    return html`${staleStrip()}${top}<div class="${size} no-drawer">${wide ? nav("rail") : ""}<div class="list">${utility}</div></div>${wide ? "" : nav("bar")}`;
  }
  if (!wide) return html`${staleStrip()}${top}<div class="medium">${listPanel(list, false)}${drawer(false)}</div>${nav("bar")}`;
  const layoutNode = view().layout;
  const plain = layoutNode.tabs?.every((child) => child.panel);
  const main = plain ? listPanel(list, true) : tree(layoutNode, list);
  return html`${staleStrip()}${top}<div class="wide">${nav("rail")}${main}${drawer(true)}</div>`;
}

const notFound = () => html`<p class="empty">No such page. Go back to the <a class="ghost" href="/">Desk</a>.</p>`;

const track = (observer) => (state.observers.push(observer), observer);

function loadOlder(listing) {
  if (state.all[listing]) return;
  state.all[listing] = true;
  // A render may call this; the refresh renders again once the rows arrive.
  setTimeout(refresh);
}

// The section chips mark the section in view; a list's last row loads its older rows.
function watchSections(app) {
  for (const row of app.querySelectorAll("[data-older]")) {
    track(new IntersectionObserver(([entry]) => { if (entry.isIntersecting) loadOlder(row.dataset.older); })).observe(row);
  }
  for (const scroller of app.querySelectorAll("[data-sections]")) {
    const chips = [...scroller.querySelectorAll(".jump a")];
    const sections = [...scroller.querySelectorAll(".sect")];
    const visible = new Set();
    const observer = track(new IntersectionObserver((entries) => {
      for (const entry of entries) visible[entry.isIntersecting ? "add" : "delete"](entry.target);
      const first = sections.find((section) => visible.has(section));
      if (first) chips.forEach((chip) => chip.setAttribute("aria-current", String(chip.hash === `#${first.id}`)));
    }, { root: scroller, rootMargin: "-48px 0px -40% 0px" }));
    sections.forEach((section) => observer.observe(section));
    const options = scroller.querySelector(".opts");
    const bar = app.querySelector(".abar");
    if (!options || !bar) continue;
    track(new IntersectionObserver(([entry]) => { bar.hidden = entry.isIntersecting; }, { root: scroller })).observe(options);
  }
}

// Events
document.addEventListener("click", (event) => {
  const target = event.target.closest("a, button, [data-action]");
  if (!target) return;
  const action = target.dataset.action;
  if (action === "retry") return refresh();
  if (action === "sign-out") { event.preventDefault(); return signOut(null); }
  if (target.dataset.answer !== undefined || ["undo", "options", "write", "dismiss", "snooze", "unsnooze"].includes(action)) return act(target);
  if (action === "mode" || action === "theme") {
    const root = document.documentElement;
    if (action === "mode") root.dataset.mode = target.dataset.pickMode;
    else root.dataset.theme = target.dataset.themePick;
    localStorage.setItem("priory.mode", root.dataset.mode);
    localStorage.setItem("priory.theme", root.dataset.theme);
    return render();
  }
  if (target.tagName === "A" && target.origin === location.origin && !target.hash && !event.metaKey && !event.ctrlKey) {
    event.preventDefault();
    go(target.pathname + target.search);
  }
  if (target.tagName === "A" && target.hash) {
    event.preventDefault();
    document.querySelector(target.hash)?.scrollIntoView({ behavior: matchMedia("(prefers-reduced-motion: reduce)").matches ? "auto" : "smooth" });
  }
});

document.addEventListener("input", (event) => {
  const ask = event.target.id === "draft" && askOnScreen();
  if (ask) state.drafts.set(ask.id, event.target.value);
});

document.addEventListener("submit", (event) => {
  if (event.target.dataset.action === "send-text") {
    event.preventDefault();
    const ask = askOnScreen();
    const text = (state.drafts.get(ask?.id) || "").trim();
    if (ask && text) answer(ask, text, true);
    return;
  }
  if (event.target.dataset.action !== "sign-in") return;
  event.preventDefault();
  const token = new FormData(event.target).get("token").trim();
  if (!token) return;
  localStorage.setItem("priory.token", token);
  Object.assign(state, { token, signInError: null });
  refresh();
});

// Desktop shortcuts: j and k move, Enter opens, Esc closes. Never inside an input.
document.addEventListener("keydown", (event) => {
  if ((event.key === " " || event.key === "Enter") && event.target.matches?.("[role=radio]")) {
    event.preventDefault();
    return event.target.click();
  }
  if (event.target.closest("input, textarea") || event.metaKey || event.ctrlKey || event.altKey || !state.data) return;
  const rows = [...document.querySelectorAll(".list [href^='/asks/'], .list tr[data-href]")];
  const hrefOf = (row) => row.getAttribute("href") || row.dataset.href;
  const at = rows.findIndex((row) => row.getAttribute("aria-current") === "true");
  if (selectedAsk() && /^[1-9]$/.test(event.key)) {
    document.querySelector(`.opt[data-answer="${Number(event.key) - 1}"]`)?.click();
  } else if (event.key === "j" || event.key === "k") {
    const next = rows[Math.min(rows.length - 1, Math.max(0, at + (event.key === "j" ? 1 : -1)))];
    if (next) go(hrefOf(next));
  } else if (event.key === "Enter" && at >= 0 && document.activeElement === document.body) {
    go(hrefOf(rows[at]));
  } else if (event.key === "Escape" && selectedAsk()) {
    go(state.listUrl);
  }
});

addEventListener("popstate", render);
for (const query of ["(min-width: 600px)", "(min-width: 1024px)"]) matchMedia(query).addEventListener("change", render);
document.addEventListener("visibilitychange", () => { if (!document.hidden && state.token) refresh(); });
// Waits on the state's revision; the server answers once it moves, or after 25 s.
async function follow() {
  for (;;) {
    if (!state.token || state.revision === undefined || document.hidden) {
      await new Promise((resolve) => setTimeout(resolve, 1000));
      continue;
    }
    let ok;
    try {
      const { revision } = await api(`/api/events?since=${state.revision}`);
      ok = revision === state.revision || (await refresh());
    } catch {
      await refresh();
      ok = false;
    }
    if (!ok) await new Promise((resolve) => setTimeout(resolve, 5000));
  }
}
// Ages read in minutes, so repaint them once a minute even when nothing changes.
setInterval(() => { if (!document.hidden && state.token) refresh(); }, 60000);
follow();
refresh();
