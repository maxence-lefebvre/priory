// What every legacy page draws with: the state, the HTML template, icons, times and facets.
import { runnerOf } from "../asks.js";
import { taskRepo } from "../desk.js";
import { linked } from "../links.js";
import { markdown } from "../markdown.js";
import { prStack } from "../stack.js";
import { filtered, taskLabel } from "../tasks.js";
import { refresh } from "./data.js";


export const ICONS = {
  check: '<path d="M20 6 9 17l-5-5"/>',
  plus: '<path d="M5 12h14"/><path d="M12 5v14"/>',
  minus: '<path d="M5 12h14"/>',
  x: '<path d="M18 6 6 18"/><path d="m6 6 12 12"/>',
  "circle-x": '<circle cx="12" cy="12" r="10"/><path d="m15 9-6 6"/><path d="m9 9 6 6"/>',
  square: '<rect width="18" height="18" x="3" y="3" rx="2"/>',
  layers: '<path d="m12.83 2.18a2 2 0 0 0-1.66 0L2.6 6.08a1 1 0 0 0 0 1.83l8.58 3.91a2 2 0 0 0 1.66 0l8.58-3.9a1 1 0 0 0 0-1.83Z"/><path d="m22 17.65-9.17 4.16a2 2 0 0 1-1.66 0L2 17.65"/><path d="m22 12.65-9.17 4.16a2 2 0 0 1-1.66 0L2 12.65"/>',
  "circle-dashed": '<path d="M10.1 2.18a9.93 9.93 0 0 1 3.8 0"/><path d="M17.6 3.71a9.95 9.95 0 0 1 2.69 2.7"/><path d="M21.82 10.1a9.93 9.93 0 0 1 0 3.8"/><path d="M20.29 17.6a9.95 9.95 0 0 1-2.7 2.69"/><path d="M13.9 21.82a9.94 9.94 0 0 1-3.8 0"/><path d="M6.4 20.29a9.95 9.95 0 0 1-2.69-2.7"/><path d="M2.18 13.9a9.93 9.93 0 0 1 0-3.8"/><path d="M3.71 6.4a9.95 9.95 0 0 1 2.7-2.69"/>',
  "circle-dot": '<circle cx="12" cy="12" r="10"/><circle cx="12" cy="12" r="1"/>',
  "circle-alert": '<circle cx="12" cy="12" r="10"/><path d="M12 8v4"/><path d="M12 16h.01"/>',
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
  search: '<path d="m21 21-4.34-4.34"/><circle cx="11" cy="11" r="8"/>',
  clock: '<circle cx="12" cy="12" r="10"/><path d="M12 6v6l4 2"/>',
  "chevron-up": '<path d="m18 15-6-6-6 6"/>',
  "arrow-up": '<path d="m5 12 7-7 7 7"/><path d="M12 19V5"/>',
  "arrow-down": '<path d="M12 5v14"/><path d="m19 12-7 7-7-7"/>',
  "chevron-down": '<path d="m6 9 6 6 6-6"/>',
  desk: '<path d="M7.9 20A9 9 0 1 0 4 16.1L2 22Z"/>',
  asks: '<path d="M14 9a2 2 0 0 1-2 2H6l-4 4V4a2 2 0 0 1 2-2h8a2 2 0 0 1 2 2z"/><path d="M18 9h2a2 2 0 0 1 2 2v11l-4-4h-6a2 2 0 0 1-2-2v-1"/>',
  tasks: '<path d="M3 12h.01M3 18h.01M3 6h.01M8 12h13M8 18h13M8 6h13"/>',
  runs: '<path d="M3 3v16a2 2 0 0 0 2 2h16"/><path d="M7 7h8"/><path d="M11 12h7"/><path d="M9 17h4"/>',
  inbox: '<polyline points="22 12 16 12 14 15 10 15 8 12 2 12"/><path d="M5.45 5.11 2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.45-6.89A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11z"/>',
  reports: '<path d="M3 3v16a2 2 0 0 0 2 2h16"/><path d="M18 17V9"/><path d="M13 17V5"/><path d="M8 17v-3"/>',
  queue: '<path d="M16 5H3"/><path d="M16 12H3"/><path d="M9 19H3"/><path d="m16 16-3 3 3 3"/><path d="M21 5v12a2 2 0 0 1-2 2h-6"/>',
  terminal: '<path d="m4 17 6-6-6-6"/><path d="M12 19h8"/>',
  cog: '<path d="M12 20a8 8 0 1 0 0-16 8 8 0 0 0 0 16Z"/><path d="M12 14a2 2 0 1 0 0-4 2 2 0 0 0 0 4Z"/><path d="M12 2v2"/><path d="M12 22v-2"/><path d="m17 20.66-1-1.73"/><path d="M11 10.27 7 3.34"/><path d="m20.66 17-1.73-1"/><path d="m3.34 7 1.73 1"/><path d="M14 12h8"/><path d="M2 12h2"/><path d="m20.66 7-1.73 1"/><path d="m3.34 17 1.73-1"/><path d="m17 3.34-1 1.73"/><path d="m11 13.73-4 6.93"/>',
  user: '<circle cx="12" cy="8" r="4"/><path d="M4 21a8 8 0 0 1 16 0"/>',
  workflow: '<rect width="8" height="8" x="3" y="3" rx="2"/><path d="M7 11v4a2 2 0 0 0 2 2h4"/><rect width="8" height="8" x="13" y="13" rx="2"/>',
  more: '<circle cx="12" cy="12" r="1"/><circle cx="19" cy="12" r="1"/><circle cx="5" cy="12" r="1"/>',
  settings: '<path d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z"/><circle cx="12" cy="12" r="3"/>',
  pencil: '<path d="M21.2 6.8a2.8 2.8 0 0 0-4-4L3.8 16.2a2 2 0 0 0-.5.8l-1.3 4.4a.5.5 0 0 0 .6.6l4.4-1.3a2 2 0 0 0 .8-.5z"/>',
  scale: '<path d="m16 16 3-8 3 8c-.87.65-1.92 1-3 1s-2.13-.35-3-1Z"/><path d="m2 16 3-8 3 8c-.87.65-1.92 1-3 1s-2.13-.35-3-1Z"/><path d="M7 21h10"/><path d="M12 3v18"/><path d="M3 7h2c2 0 5-1 7-2 2 1 5 2 7 2h2"/>',
  history: '<path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8"/><path d="M3 3v5h5"/><path d="M12 7v5l4 2"/>',
  "message-text": '<path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/><path d="M13 8H7"/><path d="M17 12H7"/>',
  hourglass: '<path d="M5 22h14"/><path d="M5 2h14"/><path d="M17 22v-4.172a2 2 0 0 0-.586-1.414L12 12l-4.414 4.414A2 2 0 0 0 7 17.828V22"/><path d="M7 2v4.172a2 2 0 0 0 .586 1.414L12 12l4.414-4.414A2 2 0 0 0 17 6.172V2"/>',
  "corner-down-right": '<polyline points="15 10 20 15 15 20"/><path d="M4 4v7a4 4 0 0 0 4 4h12"/>',
  "arrow-right": '<path d="M5 12h14"/><path d="m12 5 7 7-7 7"/>',
  "rotate-ccw": '<path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8"/><path d="M3 3v5h5"/>',
  "log-out": '<path d="m16 17 5-5-5-5"/><path d="M21 12H9"/><path d="M9 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4"/>',
  bell: '<path d="M10.27 21a2 2 0 0 0 3.46 0"/><path d="M3.26 15.33A1 1 0 0 0 4 17h16a1 1 0 0 0 .74-1.67C19.41 13.96 18 12.5 18 8A6 6 0 0 0 6 8c0 4.5-1.41 5.96-2.74 7.33"/>',
  lock: '<rect width="18" height="11" x="3" y="11" rx="2"/><path d="M7 11V7a5 5 0 0 1 10 0v4"/>',
  costs: '<circle cx="12" cy="12" r="10"/><path d="M16 8h-6a2 2 0 1 0 0 4h4a2 2 0 1 1 0 4H8"/><path d="M12 18V6"/>',
  "trending-up": '<path d="M16 7h6v6"/><path d="m22 7-8.5 8.5-5-5L2 17"/>',
  "trending-down": '<path d="M16 17h6v-6"/><path d="m22 17-8.5-8.5-5 5L2 7"/>',
  server: '<rect width="20" height="8" x="2" y="2" rx="2" ry="2"/><rect width="20" height="8" x="2" y="14" rx="2" ry="2"/><path d="M6 6h.01"/><path d="M6 18h.01"/>',
  "unplug": '<path d="m19 5 3-3"/><path d="m2 22 3-3"/><path d="M6.3 20.3a2.4 2.4 0 0 0 3.4 0L12 18l-6-6-2.3 2.3a2.4 2.4 0 0 0 0 3.4Z"/><path d="M7.5 13.5 10 11"/><path d="M10.5 16.5 13 14"/><path d="m12 6 6 6 2.3-2.3a2.4 2.4 0 0 0 0-3.4l-2.6-2.6a2.4 2.4 0 0 0-3.4 0Z"/>',
  copy: '<rect width="14" height="14" x="8" y="8" rx="2"/><path d="M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2"/>',
  fit: '<path d="M8 3H5a2 2 0 0 0-2 2v3"/><path d="M21 8V5a2 2 0 0 0-2-2h-3"/><path d="M3 16v3a2 2 0 0 0 2 2h3"/><path d="M16 21h3a2 2 0 0 0 2-2v-3"/>',
  eye: '<path d="M2 12s3-7 10-7 10 7 10 7-3 7-10 7-10-7-10-7Z"/><circle cx="12" cy="12" r="3"/>',
  keyboard: '<path d="M10 8h.01"/><path d="M12 12h.01"/><path d="M14 8h.01"/><path d="M16 12h.01"/><path d="M18 8h.01"/><path d="M6 8h.01"/><path d="M7 16h10"/><path d="M8 12h.01"/><rect width="20" height="16" x="2" y="4" rx="2"/>',
  brain: '<path d="M12 5a3 3 0 1 0-5.997.125 4 4 0 0 0-2.526 5.77 4 4 0 0 0 .556 6.588A4 4 0 1 0 12 18Z"/><path d="M12 5a3 3 0 1 1 5.997.125 4 4 0 0 1 2.526 5.77 4 4 0 0 1-.556 6.588A4 4 0 1 1 12 18Z"/><path d="M15 13a4.5 4.5 0 0 1-3-4 4.5 4.5 0 0 1-3 4"/><path d="M17.599 6.5a3 3 0 0 0 .399-1.375"/><path d="M6.003 5.125A3 3 0 0 0 6.401 6.5"/><path d="M3.477 10.896a4 4 0 0 1 .585-.396"/><path d="M19.938 10.5a4 4 0 0 1 .585.396"/><path d="M6 18a4 4 0 0 1-1.967-.516"/><path d="M19.967 17.484A4 4 0 0 1 18 18"/>',
  shield: '<path d="M20 13c0 5-3.5 7.5-7.7 9a1 1 0 0 1-.6 0C7.5 20.5 4 18 4 13V6a1 1 0 0 1 1-1c2 0 4.5-1.2 6.2-2.7a1.2 1.2 0 0 1 1.6 0C14.5 3.8 17 5 19 5a1 1 0 0 1 1 1z"/>',
};

class Raw { constructor(text) { this.text = text; } }
export const raw = (text) => new Raw(text);
const escapeText = (value) => String(value).replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`);
const piece = (value) => {
  if (value instanceof Raw) return value.text;
  if (Array.isArray(value)) return value.map(piece).join("");
  if (value === null || value === undefined || value === false) return "";
  return escapeText(value);
};
export function html(strings, ...values) {
  return raw(strings.reduce((out, text, i) => out + text + (i < values.length ? piece(values[i]) : ""), ""));
}
// Agent text with the factory's autolinks; `linked` escapes it.
export const autolinked = (text, repo) => raw(linked(text ?? "", state.data?.links || [], repo ?? null));
// A Desk row's text links `#n` in its task's repository.
export const repoOf = (task) => taskRepo(state.data?.tasks, task);
export const md = (text, repo) => html`<div class="md">${raw(markdown(text, state.data?.links || [], repo ?? null))}</div>`;
export const icon = (name, size = "") => raw(`<svg class="ico ${size}" viewBox="0 0 24 24" aria-hidden="true">${ICONS[name]}</svg>`);

// Task and run states: role and icon (DESIGN.md, State tile).
export const STATES = {
  queued: ["muted", "circle-dashed"], held: ["hold", "pause"], ready: ["live", "circle-dot"],
  running: ["live", null], asleep: ["muted", "moon"], blocked: ["warn", "hand"], done: ["ok", "check"],
  failed: ["danger", "circle-x"], dropped: ["muted", "minus"], lost: ["warn", "unplug"],
  ask: ["wait", "ask"], answered: ["muted", "check"], withdrawn: ["muted", "minus"], prompt: ["warn", "hand"],
  refused: ["warn", "x"], moved: ["muted", "arrow-right"], todo: ["wait", "square"], batch: ["muted", "layers"],
  restarted: ["muted", "rotate-ccw"], cancelled: ["muted", "x"], exited: ["danger", "log-out"],
  // Another runner's task: its tile shows its hold, which is not a task state.
  elsewhere: ["muted", "server"],
  // Memory candidates an agent wrote, which wait on a person's accept or drop.
  memories: ["wait", "brain"],
  // A scorer's verdict on a run: an error means the scorer broke, not the run.
  // A pull request check: one not finished draws as running.
  passed: ["ok", "check"], skipped: ["muted", "minus"],
  pass: ["ok", "check"], fail: ["danger", "circle-x"], error: ["warn", "circle-alert"], "not applicable": ["muted", "minus"],
};
export const role = (state) => (STATES[state] || ["muted", "circle-help"])[0];
export function tile(state, size = "") {
  const [kind, name] = STATES[state] || ["muted", "circle-help"];
  return html`<span class="tile ${kind} ${size}" role="img" aria-label="${state}">${name ? icon(name) : raw('<i class="dot"></i>')}</span>`;
}
export const stateWord = (state) => {
  const [kind, name] = STATES[state] || ["muted", "circle-help"];
  return html`<span class="sti t-${kind}">${name ? icon(name, "s15") : raw('<i class="dot"></i>')}${state}</span>`;
};

// The last listings read and when, kept on this device, so an offline start shows them with their age.
export const kept = JSON.parse(localStorage.getItem("priory.snapshot") || "null");
export const state = {
  token: localStorage.getItem("priory.token"),
  data: kept?.data || null,
  // The data came from this device, not the server: views and links are read again.
  cached: Boolean(kept),
  // The listings to read in full, once the person scrolls to their older rows.
  all: { asks: false, tasks: false, runs: false },
  refreshes: 0,
  updated: kept?.updated || null,
  failed: null,
  signInError: null,
  list: "desk",
  listUrl: "/",
  // The page the last loaded render drew, so the Desk knows a person arrived rather than closed an ask.
  drawn: null,
  // The last list the person opened; a task or batch page opened next keeps it in its history entry.
  from: null,
  // The listings drawn last, as the query cache holds them.
  listings: null,
  rendered: 0,
  scrolls: new Map(),
  observers: [],
  // The answer in flight: { id, text }.
  pending: null,
  // The option the first tap armed, until the second tap sends it or 4 s pass: { id, revision, text, at, timer }.
  armed: null,
  // A task's cancel or revive armed by its first tap, until the second tap or 4 s: { task, act, at, timer }.
  taskArmed: null,
  // The task whose action is in flight, each task's last action note, and its unsent say text.
  taskBusy: null,
  taskNotices: new Map(),
  // The remote task row opened in place, by its id `runner:key`.
  remoteOpen: null,
  // The Unrouted view's groups, read when the view opens, and each resource's last subscribe note.
  unrouted: { groups: null, error: null, loading: false, busy: null, notices: new Map() },
  todoBusy: null,
  todoNotices: new Map(),
  rowBusy: null,
  rowNotices: new Map(),
  // Memory candidates: the Accept armed by its first tap { id, at, timer }, the one in flight, each
  // candidate's last note, and those sent, whose buttons stay off until the desk drops them.
  memoryArmed: null,
  memoryBusy: null,
  memoryNotices: new Map(),
  memorySent: new Set(),
  // The candidates whose Current text is open, so a redraw keeps it open.
  memoryCurrent: new Set(),
  // The task whose Move is in flight, and each task's last Move refusal.
  moveBusy: null,
  moveNotices: new Map(),
  // The task whose Move menu is open, so a redraw keeps it open.
  moveOpen: null,
  sayDrafts: new Map(),
  // The batch the page or the go ask shows, and the go ask's plan edits: the entry whose menu is open,
  // the drop armed by its first tap, whether an edit is in flight, and what the last one said.
  batch: null,
  planEdit: { at: null, batch: null, open: null, armed: null, busy: false, note: null },
  drafts: new Map(),
  notices: new Map(),
  writing: new Set(),
  // The Reports panel: the pick waiting out its Undo, then in flight; each row's last refusal; the
  // comment field open on a row, with its text, and whether a comment verb is in flight.
  reportPending: null,
  reportNotices: new Map(),
  // The task whose report shows, null for the one written last; the search text, null while it is
  // closed; the row whose agreed pick Change reopened; the row Next pick reached last.
  reportShown: null,
  reportQuery: null,
  reportChanging: null,
  reportAt: null,
  // A pick's or a comment's outcome once its row has left the report.
  reportFlash: null,
  compose: null,
  remarking: false,
  // The Costs panel: the split it shows, whether a read is in flight and past 300 ms, and why the last failed.
  costs: { by: "workflow", loading: false, slow: false, error: null },
  // The Add sheet: its draft and chips, the server's defaults, and where the last send got to.
  // Notifications on this device: its status, a request in flight, and why the last one failed.
  // check counts status reads, so a read that a tap or a newer read overtook is dropped.
  push: { status: null, kinds: null, busy: false, error: null, check: 0, checking: false },
  // The browser's install prompt, kept until the person taps Install.
  install: null,
  // Since you looked: the events the Desk opened on and each newer one, the visit they start
  // from, when the first live read began, and whether POST /api/seen went.
  since: { events: [], from: null, opened: null, sent: false },
  add: { phase: "empty", error: null, defaults: null, task: null, draft: { text: "" }, ...JSON.parse(localStorage.getItem("priory.add") || "{}") },
};

// Times: relative under 24 h, then a date (DESIGN.md, Copy).
export function age(iso) {
  if (!iso) return "";
  const elapsed = Date.now() - Date.parse(iso);
  if (elapsed < 60000) return "just now";
  const minutes = Math.round(elapsed / 60000);
  if (minutes < 60) return `${minutes} min`;
  if (minutes < 1440) return `${Math.floor(minutes / 60)} h`;
  return date(iso);
}
export const date = (iso) => new Date(iso).toLocaleString("en", { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit", hour12: false });
export const clock = (iso) => new Date(iso).toLocaleTimeString("en", { hour: "2-digit", minute: "2-digit", hour12: false });
export const timeTag = (iso, text = age(iso)) => html`<time datetime="${iso}" title="${iso ? new Date(iso).toString() : ""}">${text}</time>`;
export const today = (iso) => new Date(iso).toDateString() === new Date().toDateString();

// Routes: /, /asks, /asks/42, /tasks, /more, /settings/appearance
// go() moves the route main.jsx renders.
export const route = () => location.pathname.split("/").filter(Boolean);

export const layout = () => (matchMedia("(min-width: 1024px)").matches ? "wide" : matchMedia("(min-width: 600px)").matches ? "medium" : "compact");

export function leaves(node) {
  if (node.panel) return [node.panel];
  return Object.values(node).flat().flatMap(leaves);
}
export const view = () => state.data.views.views.default;
export const panelsOf = () => [...new Set(leaves(view().layout))];

// The last row of a list that left out older rows; scrolling to it loads them.
export function olderRow(listing, count, wide, columns = 6) {
  if (!count) return "";
  const text = `Loading older ${listing}…`;
  return wide ? html`<tr class="grp" data-older="${listing}"><td colspan="${columns}">${text}</td></tr>` : html`<p class="empty" data-older="${listing}">${text}</p>`;
}

// "3 PRs" for a task with a stack of two or more. The queue and desk carry no stacks; the tasks listing does.
export const prs = (task, id = task.id) => prStack(task.stack ?? state.data.tasks.find((each) => each.id === id)?.stack).label;
export const prsTag = (task, id) => {
  const label = prs(task, id);
  return label ? html`<span class="tag">${label}</span>` : "";
};
// While an ask is open, the list keeps the view it was opened from.
export const listSearch = () => (selectedAsk() ? new URL(state.listUrl, location.origin).search : location.search);

export function savedViews(list, base, facets = "") {
  return html`<nav class="views" aria-label="Saved views">${list.map(([key, label, count, on]) =>
    html`<a href="${base}?view=${key}${facets}" aria-current="${on ? "true" : "false"}">${label}${count === null ? "" : html`<span class="num">${count}</span>`}</a>`)}</nav>`;
}

export const olderTasks = () => state.data.older.tasks;
export const listOrEmpty =(rows, empty) => (rows.length ? rows : html`<p class="empty">${empty}</p>`);

// Each facet's key, label, an ask's values and the order the picker offers them. An ask with no
// band, such as an outside ask, shows under `none`, and so does an ask with no risk.
const FACETS = [
  ["band", "Band", (ask) => [ask.band ?? "none"], () => [...(state.data.bands || []), "none"]],
  ["risk", "Risk", (ask) => (ask.risk?.length ? ask.risk : ["none"]), () => ["irreversible", "high", "medium", "low", "red", "yellow", "green", "none"]],
  ["runner", "Runner", (ask) => [ask.runner ?? "none"], null, (name) => (state.data.asks.some((ask) => ask.runner === name && !runnerOf(ask)) ? `${name} (here)` : name)],
];
// The Runner facet shows only in a factory of several runners, where every ask names one.
export const shownFacets = () => FACETS.filter(([key]) => key !== "runner" || state.data.asks.some((ask) => ask.runner));
export const facetPicks = (facets = FACETS) => {
  const search = new URLSearchParams(listSearch());
  return new Map(facets.map(([key]) => [key, search.get(key)]).filter(([, value]) => value));
};
export const faceted = (asks, picks) => filtered(asks, picks, FACETS);
export const facetQuery = (picks) => [...picks].map(([key, value]) => `&${key}=${encodeURIComponent(value)}`).join("");

// A facet chip of the list at `base` over `items`; a facet with no order offers its values A to Z.
export function facet(view, picks, [key, label, values, order, named = (name) => name], base = "/asks", items = state.data.asks) {
  const others = new Map([...picks].filter(([other]) => other !== key));
  const picked = picks.get(key);
  if (picked) return html`<a class="facet" aria-current="true" href="${base}?view=${view}${facetQuery(others)}" aria-label="${label}: ${picked}. Clear">${picked}${icon("x", "s13")}</a>`;
  const counts = new Map();
  for (const item of items) for (const value of values(item)) counts.set(value, (counts.get(value) || 0) + 1);
  const offered = [...counts].sort(([a], [b]) => (order ? order().indexOf(a) - order().indexOf(b) : a.localeCompare(b)));
  return html`<details class="facet"><summary aria-label="Filter by ${key}">${label}${icon("chevron-down", "s13")}</summary>
    <div class="pick">${offered.map(([name, count]) => html`<a href="${base}?view=${view}${facetQuery(others)}&${key}=${encodeURIComponent(name)}">${named(name)}<span class="num">${count}</span></a>`)}</div></details>`;
}

// The saved view the URL names, else the list's first.
export const listView = (views) => {
  const name = new URLSearchParams(location.search).get("view");
  return views.includes(name) ? name : views[0];
};

// Tasks (DESIGN.md, Tasks panel): every task, by saved view and facets; each row opens its page.
// A task by its id, named by its key once the task list holds it.
export const taskName = (id) => {
  const task = state.data?.tasks?.find((each) => each.id === id);
  return task ? taskLabel(task.key) : `task ${id}`;
};
export const batchTag = (task) => (task.batch == null ? "" : html`<a class="tag" href="/batches/${task.batch}">${icon("layers", "s13")}batch ${task.batch}</a>`);
export const reasonShown = (task) => Boolean(task.reason) && ["blocked", "held", "failed", "dropped"].includes(task.state);

// "12 min ago", "just now", or the date for a time over a day ago.
export const ago = (iso) => {
  const text = age(iso);
  if (text === "just now") return text;
  return / (min|h)$/.test(text) ? `${text} ago` : `on ${text}`;
};

// Ask detail
export const selectedAsk = () => {
  const [first, id] = route();
  return first === "asks" && id ? decodeURIComponent(id) : null;
};

export async function post(path, body) {
  const response = await fetch(path, {
    method: "POST",
    headers: { Authorization: `Bearer ${state.token}`, "Content-Type": "application/json" },
    body: JSON.stringify(body),
    cache: "no-store",
  });
  return { status: response.status, body: await response.json().catch(() => ({})) };
}

// An id no other control's parts can spell: whitespace and quotes are escaped away.
export const focusId = (...parts) => encodeURIComponent(JSON.stringify(parts));

// Why a write did not go; `refused` words a refusal the daemon gave.
export function writeRefusal(status, body, refused = () => `Not sent: ${body.error || `the server answered ${status}`}.`) {
  if (status === 503) return "Could not send: no priory daemon runs beside this server. Start priory daemon, then send again.";
  if (status === 504 || status === 0) return "The daemon may have applied this but did not confirm it. Check the report before you send again.";
  return refused();
}

export function current() {
  const [first] = route();
  if (!first || first === "index.html") return "desk";
  if (first === "settings") return "settings";
  return first;
}

export const routeKey = () => location.pathname + location.search;
const FOCUS_ATTRIBUTES = ["id", "href", "data-option", "data-pick-mode", "data-theme-pick", "data-add-chip", "data-key", "data-action"];
export function focusKey(element) {
  if (!element || element === document.body) return null;
  const name = FOCUS_ATTRIBUTES.find((attribute) => element.hasAttribute(attribute));
  return name ? `[${name}="${CSS.escape(element.getAttribute(name))}"]` : null;
}

export const track = (observer) => (state.observers.push(observer), observer);

export function loadOlder(listing) {
  if (state.all[listing]) return;
  state.all[listing] = true;
  // A render may call this; the refresh renders again once the rows arrive.
  setTimeout(refresh);
}
