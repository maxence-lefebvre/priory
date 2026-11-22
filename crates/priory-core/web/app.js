// Priory's web view (DESIGN.md): the frame, the Desk and the Asks, with answering through the CLI's verbs.
// Agent text is untrusted: every value goes through `html`, which escapes it.
import { ARM_MS, ASK_VIEWS, answeredBy, askGroups, openAsksOf, routedMark, routedNote, runnerOf, seenAgo, sentNote, shownGroups, snoozeShortcut, snoozed, stillArmed, submitted, takesText, writeNote } from "./asks.js";
import { SPLITS, attentionView, costTiles, dayBars, money, scoreTable, splitTable } from "./costs.js";
import { addBody, addOutcome } from "./add.js";
import { linked } from "./links.js";
import { markdown } from "./markdown.js";
import { around, dossier, nextPick, refusal } from "./reports.js";
import { checkState, prStack, stateWords } from "./stack.js";
import { KINDS, edgeStyle, historyRows, lockBadge, nodeShape, timeline, ungatedPairs } from "./flows.js";
import { emptyDetail, eventWords, handled, neverOwned, sinceWords, routeAction, unheardActs, unheardNote, unroutedNote } from "./inbox.js";
import { editBody, issueName, laneLayout, momentWords, stages, successors } from "./batches.js";
import { EARLIER_CAP, KEYS, keyboardSession, paintRows, sayLine, screenRows, screenWatch } from "./terminal.js";
import { actionNote, batchParts, countWords, dueParts, memoryNote, memoryTag, mergeSince, nextLink, onRoom, progressView, rowAction, sectionInView, sinceGroups } from "./desk.js";
import { TASK_VIEWS, elsewhere, filtered, olderPart, remoteActs, remoteId, remoteSent, taskFacets, taskLabel, taskViews, triageWords } from "./tasks.js";
import { RUN_FACETS, RUN_VIEWS, duration, runState, runViews, tokenText } from "./runs.js";
import { PANELS, backTo, pageTitle } from "./pages.js";
import { QUEUE_VIEWS, moveOptions, queueBands, waitReason } from "./queue.js";

const ICONS = {
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
// Agent text with the factory's autolinks; `linked` escapes it.
const autolinked = (text, repo) => raw(linked(text ?? "", state.data?.links || [], repo ?? null));
const md = (text, repo) => html`<div class="md">${raw(markdown(text, state.data?.links || [], repo ?? null))}</div>`;
const icon = (name, size = "") => raw(`<svg class="ico ${size}" viewBox="0 0 24 24" aria-hidden="true">${ICONS[name]}</svg>`);

// Task and run states: role and icon (DESIGN.md, State tile).
const STATES = {
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
const role = (state) => (STATES[state] || ["muted", "circle-help"])[0];
function tile(state, size = "") {
  const [kind, name] = STATES[state] || ["muted", "circle-help"];
  return html`<span class="tile ${kind} ${size}" role="img" aria-label="${state}">${name ? icon(name) : raw('<i class="dot"></i>')}</span>`;
}
const stateWord = (state) => {
  const [kind, name] = STATES[state] || ["muted", "circle-help"];
  return html`<span class="sti t-${kind}">${name ? icon(name, "s15") : raw('<i class="dot"></i>')}${state}</span>`;
};

const CLI = { terminal: "priory attach" };

// The last listings read and when, kept on this device, so an offline start shows them with their age.
const kept = JSON.parse(localStorage.getItem("priory.snapshot") || "null");
const state = {
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
  // The last list the person opened; a task or batch page opened next keeps it in its history entry.
  from: null,
  snapshot: null,
  rendered: 0,
  scrolls: new Map(),
  observers: [],
  // The answer waiting out its Undo, then in flight: { id, text, timer, sending }.
  pending: null,
  // The command option the first tap armed, until the second tap or 4 s: { id, text, at, timer }.
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
  push: { status: null, busy: false, error: null, check: 0, checking: false },
  // The browser's install prompt, kept until the person taps Install.
  install: null,
  // Since you looked: the events the Desk opened on and each newer one, the visit they start
  // from, when the first live read began, and whether POST /api/seen went.
  since: { events: [], from: null, opened: null, sent: false },
  add: { phase: "empty", error: null, defaults: null, task: null, draft: { text: "" }, ...JSON.parse(localStorage.getItem("priory.add") || "{}") },
};

// Times: relative under 24 h, then a date (DESIGN.md, Copy).
function age(iso) {
  if (!iso) return "";
  const elapsed = Date.now() - Date.parse(iso);
  if (elapsed < 60000) return "just now";
  const minutes = Math.round(elapsed / 60000);
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
  if (!response.ok) throw Object.assign(new Error(`${path} answered ${response.status}`), { status: response.status });
  return response.json();
}

async function refresh() {
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
  try {
    // The server reads the views once, when it starts.
    const views = (!state.cached && state.data?.views) || (await api("/api/views"));
    const links = (!state.cached && state.data?.links) || (await api("/api/links")).links;
    const full = (listing) => (state.all[listing] ? `/api/${listing}?all=1` : `/api/${listing}`);
    // No task has written a report until the first `priory report put`.
    // Taken before the read, so an event between that read and the visit still shows next time.
    const read = new Date().toISOString().replace(/\.\d+Z$/, "Z");
    const shown = api(state.reportShown == null ? "/api/reports/latest" : `/api/reports/${state.reportShown}`)
      .catch((error) => (error.status === 404 ? null : Promise.reject(error)));
    const listed = api("/api/reports").then((read) => read.reports);
    const [desk, asksList, queue, tasksList, runsList, report, reports] = await Promise.all([...["/api/desk", full("asks"), "/api/queue", full("tasks"), full("runs")].map(api), shown, listed]);
    // A later call, such as the one that loads older rows, supersedes this one.
    if (call !== state.refreshes) return true;
    const [asks, tasks, runs] = [asksList.asks, tasksList.tasks, runsList.runs];
    const older = { asks: asksList.older, tasks: tasksList.older, runs: runsList.older };
    if (!state.since.opened) Object.assign(state.since, { opened: read, from: desk.last_visit });
    state.since.events = mergeSince(state.since.events, desk.since_you_looked);
    // The listings are read one after another; the oldest revision makes the next wait catch what they missed.
    state.revision = Math.min(desk.revision, asksList.revision, queue.revision, tasksList.revision, runsList.revision, report?.revision ?? Infinity);
    const snapshot = JSON.stringify([desk, asks, queue, tasks, runs, older, report, reports]);
    state.updated = Date.now();
    state.failed = null;
    state.cached = false;
    if (current() === "costs") loadCosts();
    if (current() === "inbox") loadUnrouted();
    // Ages read in minutes, so an unchanged state needs a new paint once a minute at most.
    if (state.data && snapshot === state.snapshot && !failed && Date.now() - state.rendered < 60000) return keep(), true;
    const before = state.data ? new Set(state.data.asks.filter((ask) => ask.state === "open").map((ask) => ask.id)) : null;
    // The Costs panel reads its own reports; a refresh keeps the last ones.
    state.data = { views, links, desk, asks, bands: asksList.bands, queue, tasks, runs, older, report, reports, spend: state.data?.spend ?? null, flows: state.data?.flows ?? null };
    acceptReport();
    state.task = null;
    if (state.batch) state.batch.stale = true;
    state.snapshot = snapshot;
    keep();
    announce(before);
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

function announce(before) {
  if (!before) return;
  const fresh = state.data.asks.find((ask) => ask.state === "open" && !before.has(ask.id));
  if (fresh) document.getElementById("announce").textContent = `New ask from ${fresh.key ? taskLabel(fresh.key) : `task ${fresh.task}`}.`;
}

// The listings and their time go in one value, so a full storage never pairs old data with a new time.
function keep() {
  try {
    localStorage.setItem("priory.snapshot", JSON.stringify({ data: state.data, updated: state.updated }));
  } catch {}
}

// The service worker cannot read local storage, so it gets its own copy of the token to tell the
// daemon about a subscription the push service replaced. Signing out drops it.
function keepTokenForWorker(token) {
  if (!("caches" in window)) return;
  caches.open("priory-push")
    .then((cache) => (token ? cache.put("/api/worker/token", new Response(token)) : cache.delete("/api/worker/token")))
    .catch(() => {});
}
if (state.token) keepTokenForWorker(state.token);

function signOut(reason) {
  for (const key of ["priory.token", "priory.snapshot"]) localStorage.removeItem(key);
  keepTokenForWorker(null);
  // A refresh still in flight must not store what this device just dropped.
  state.refreshes++;
  watch.stop();
  // Before the token goes: giving the keyboard back needs it.
  keyboard.leave();
  Object.assign(state, { token: null, data: null, snapshot: null, revision: undefined, signInError: reason, flowsError: null, all: { asks: false, tasks: false, runs: false }, costs: { by: "workflow", loading: false, slow: false, error: null } });
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
  const todos = items.filter((item) => item.kind === "todo");
  const other = items.filter((item) => !["ask", "blocked", "failed", "prompt", "command", "report", "todo"].includes(item.kind));
  // The capsule opens the ask that ranks first, not only the oldest one.
  const first = desk.ranked?.find((item) => item.kind === "ask");
  return { next: desk.next, open, first: first ? byId.get(first.id) || open[0] : open[0], stopped, todos, reports, other, asleep: desk.counts.asleep, asleepTasks: queue.asleep, banner: desk.banner, intake: desk.intake, running: queue.running, queue: queue.waiting, tasks, upNext: queue.up_next, needs: items.length };
}
const openAsks = () => deskModel().open;
// The open asks the list shows, so previous, next and the ask after an answer stay in its facets.
const listedAsks = () => faceted(openAsks(), facetPicks());

const routedTag = (ask) => routedMark(ask) ? html`<span class="tag routed">${routedMark(ask)}</span>` : "";
// Only another runner's ask carries its runner. A phone row's tag keeps the name, in warn text while
// the runner is away, and says "last seen" on line 3.
const runnerTag = (ask, full = true) => {
  const away = seenAgo(ask, Date.now(), age);
  if (!runnerOf(ask)) return "";
  return full ? html`<span class="tag runner">${runnerOf(ask)}${away ? `, ${away}` : ""}</span>` : html`<span class="tag runner ${away ? "t-warn" : ""}">${runnerOf(ask)}</span>`;
};
const sentLine = (ask) => {
  const note = sentNote(ask);
  return note ? html`<span class="${note.refused ? "t-warn" : "t-action"}">${note.text}</span>` : "";
};
const byLine = (ask) => answeredBy(ask.answered_by) ? ` by ${answeredBy(ask.answered_by)}` : "";

// Rows
function askRow(ask, current) {
  const options = ask.options || [];
  const line3 = ask.state === "answered"
    ? html`<span>Answered <b>${ask.answer}</b>${byLine(ask)}</span>`
    : ask.state === "withdrawn"
      ? html`<span>Withdrawn${ask.withdrawn_why ? `: ${ask.withdrawn_why}` : ""}</span>`
      : snoozed(ask, Date.now()) ? html`<span>${icon("clock", "s15")}${snoozedUntil(ask)}</span>`
        : sentNote(ask) ? sentLine(ask)
        : ask.recommendation ? html`<span>Recommends <b>${ask.recommendation}</b></span>` : html`<span>${options.length} options</span>`;
  const when = ask.state === "answered" && ask.answered_at
    ? timeTag(ask.answered_at)
    : timeTag(ask.since);
  return html`<a class="row ${ask.state !== "open" ? "past" : ""}" href="/asks/${ask.id}" aria-current="${current ? "true" : "false"}">
    ${tile(ask.state === "open" ? "ask" : ask.state)}
    <span class="l1"><span class="mono">${taskLabel(ask.key)}</span><span>${ask.step}</span>${ask.command ? html`<span class="tag">Command</span>` : ""}${runnerTag(ask, false)}${routedTag(ask)}<span class="age">${when}</span></span>
    <span class="l2">${ask.title || ask.question}</span>
    <span class="l3">${ask.state === "open" && !sentNote(ask) && seenAgo(ask, Date.now(), age) ? html`<span class="t-warn">${runnerOf(ask)} ${seenAgo(ask, Date.now(), age)}</span>` : ""}${line3}</span>
    <span class="chev">${icon("chevron-right", "s15")}</span></a>`;
}

const snoozedUntil = (ask) => `Snoozed until ${today(ask.snoozed_until) ? clock(ask.snoozed_until) : date(ask.snoozed_until)}`;

// The last row of a list that left out older rows; scrolling to it loads them.
function olderRow(listing, count, wide, columns = 6) {
  if (!count) return "";
  const text = `Loading older ${listing}…`;
  return wide ? html`<tr class="grp" data-older="${listing}"><td colspan="${columns}">${text}</td></tr>` : html`<p class="empty" data-older="${listing}">${text}</p>`;
}

function itemRow(item) {
  if (item.kind === "memories") return memoriesRow(item);
  // An uncertain command, like an uncertain prompt, waits on a person to check what happened.
  const kind = ["prompt", "command"].includes(item.kind) ? "prompt" : ["blocked", "failed", "held", "batch"].includes(item.kind) ? item.kind : "queued";
  const stopped = ["blocked", "failed"].includes(item.kind);
  const key = item.kind === "batch" ? html`<a class="mono" href="/batches/${item.id}">${taskLabel(item.key)}</a>`
    : stopped ? html`<a class="mono" href="/tasks/${item.task}" title="task ${item.task}">${taskLabel(item.key)}</a>` : html`<span class="mono">${taskLabel(item.key)}</span>`;
  const { line, next } = item.kind === "batch" ? batchParts(item.what) : { line: item.what, next: null };
  return html`<div class="row">
    ${tile(kind)}
    <span class="l1">${key}<span>${item.step || ""}</span><span>${item.kind}</span>${item.task && item.kind !== "batch" ? prsTag({}, item.task) : ""}<span class="age">${timeTag(item.since)}</span></span>
    <span class="l2 r t-${role(kind)}">${autolinked(line, null)}</span>${next ? html`<span class="l3">${autolinked(next, null)}</span>` : ""}${stopped ? taskActs(item.task, item.kind) : rowActs(item)}</div>`;
}

// The one row of every memory candidate that waits, which opens their review list.
function memoriesRow(item) {
  const count = item.candidates?.length ?? 0;
  return html`<a class="row" href="/memories">
    ${tile("memories")}
    <span class="l1"><span>memories</span><span>${count} ${count === 1 ? "candidate" : "candidates"}</span><span class="age">${timeTag(item.since)}</span></span>
    <span class="l2 r">${(item.candidates || []).map((candidate) => candidate.name).join(", ")}</span>
    <span class="l3"><span>Accept or drop each before other tasks get it</span></span>
    <span class="chev">${icon("chevron-right", "s15")}</span></a>`;
}

// The review list (DESIGN.md, Memory candidates): each candidate's full text, then Accept and Drop.
function memoriesPage() {
  const back = html`<div class="top"><a class="back" href="/">${icon("chevron-left", "s22")}Desk</a></div>`;
  const candidates = state.data.desk.items.find((item) => item.kind === "memories")?.candidates || [];
  const n = candidates.length;
  const intro = html`<div class="ask mhead"><h1 class="question">${n === 1 ? "1 memory candidate" : `${n} memory candidates`}</h1>
    <p class="note">${icon("brain", "s15")}<span>An agent wrote each one. Accept gives it to every task's runs, on every runner. Drop discards it, and the task that wrote it stops getting it.</span></p></div>`;
  const list = n ? candidates.map(memoryCandidate) : html`<p class="empty">No memory candidate waits. Agents write them with the remember and forget tools.</p>`;
  return html`${back}<div class="scroll">${intro}${list}</div>`;
}

function memoryCandidate(candidate) {
  const tag = memoryTag(candidate);
  const body = candidate.forget
    ? html`<p class="text">Forgets ${candidate.name}: runs stop getting it.</p>`
    : html`<p class="desc">${candidate.description}</p><p class="text">${candidate.text}</p>`;
  const current = candidate.current_text == null ? ""
    : html`<details class="current" ${state.memoryCurrent.has(candidate.id) ? raw("open") : ""}><summary data-action="memory-current" data-id="${candidate.id}">Current text</summary><p class="text">${candidate.current_text}</p></details>`;
  return html`<section class="sect mem" aria-label="${candidate.name}"><div class="sh">${tile("memories", "sm")}<span class="mono">${candidate.name}</span><span class="tag ${tag.warn ? "t-warn" : ""}">${tag.label}</span><span class="meta">${timeTag(candidate.written)}</span></div>
    ${body}${current}
    <p class="by">Written by <span class="mono">${candidate.task}</span>${candidate.runner ? ` on ${candidate.runner}` : ""}, candidate <span class="mono">${candidate.id}</span></p>
    ${memoryActs(candidate.id)}</section>`;
}

// Accept arms on the first tap and sends on the second, as Cancel does; Drop sends at once.
function memoryActs(id) {
  const busy = state.memoryBusy !== null || state.memorySent.has(id) || Boolean(state.failed);
  const armed = state.memoryArmed?.id === id;
  const accept = armed ? html`<span><small>Tap again to accept</small>Accept</span><span class="drain" aria-hidden="true"></span>` : "Accept";
  const button = (verb, kind, label) => html`<button id="memory-${verb}-${id}" class="btn ${kind} act ${verb === "accept" && armed ? "armed" : ""}" data-action="memory-act" data-id="${id}" data-verb="${verb}" aria-disabled="${busy ? "true" : "false"}">${label}</button>`;
  const notice = state.memoryNotices.get(id);
  return html`<span class="task-acts">${button("accept", "primary", accept)}${button("drop", "secondary", "Drop")}</span>${notice ? html`<p class="note acts-note" role="status">${icon("corner-down-right", "s15")}<span>${notice}</span></p>` : ""}`;
}

function disarmMemory() {
  if (!state.memoryArmed) return;
  clearTimeout(state.memoryArmed.timer);
  state.memoryArmed = null;
}

async function memoryAct(target) {
  if (target.getAttribute("aria-disabled") === "true") return;
  const { id, verb } = target.dataset;
  const armed = state.memoryArmed?.id === id;
  disarmMemory();
  if (verb === "accept" && !armed) {
    state.memoryArmed = { id, at: Date.now(), timer: setTimeout(() => { state.memoryArmed = null; render(); }, ARM_MS) };
    return render();
  }
  state.memoryBusy = id;
  render();
  let reply;
  try {
    reply = await post(`/api/memories/${encodeURIComponent(id)}/${verb}`, {});
  } catch {
    reply = { status: 0, body: { error: "the connection dropped" } };
  }
  state.memoryBusy = null;
  const { status, body } = reply;
  if (status === 401) return signOut("The daemon refused this token. Paste the current one.");
  if (status === 200) state.memorySent.add(id);
  state.memoryNotices.set(id, memoryNote(verb, status, body));
  render();
  refresh();
}

// Read on a report and Release on a held task, the twins of `priory inbox read` and `priory release`.
function rowActs(item) {
  const action = rowAction(item, state.revision);
  if (!action) return "";
  const id = `${item.kind}-${item.id}`;
  // A cached desk has no revision yet: Release waits for the live read, or it would skip the 409 check.
  const busy = state.rowBusy !== null || Boolean(state.failed) || Object.values(action.body).includes(undefined);
  const notice = state.rowNotices.get(id);
  return html`<span class="task-acts"><button id="row-${id}" class="btn secondary act" data-action="row-act" data-row="${id}" data-kind="${item.kind}" data-item="${item.id}" data-task="${item.task ?? ""}" aria-disabled="${busy ? "true" : "false"}">${action.label}</button></span>${notice ? html`<p class="note acts-note" role="status">${notice}</p>` : ""}`;
}

async function rowAct(target) {
  if (target.getAttribute("aria-disabled") === "true") return;
  const { row, kind, item, task } = target.dataset;
  const action = rowAction({ kind, id: item, task: task || null }, state.revision);
  state.rowBusy = row;
  render();
  let reply;
  try {
    reply = await post(action.path, action.body);
  } catch {
    reply = { status: 0, body: { error: "the connection dropped" } };
  }
  state.rowBusy = null;
  const { status, body } = reply;
  if (status === 401) return signOut("The daemon refused this token. Paste the current one.");
  if (status === 200) state.rowNotices.delete(row);
  else state.rowNotices.set(row, actionNote(action.label, status, body));
  render();
  refresh();
}

// A to-do's text with its due time in local time; the `<time>` keeps the UTC time (DESIGN.md, Copy).
function todoText(item) {
  const due = dueParts(item.what, Date.now());
  return due ? html`${autolinked(due.text, null)}, due ${timeTag(due.iso, due.label)}` : autolinked(item.what, null);
}

// A person's to-do: its text and due time, and Done, the twin of `priory todo done`.
function todoRow(item) {
  return html`<div class="row">
    ${tile("todo")}
    <span class="l1"><span>to-do</span><span class="age">${timeTag(item.since)}</span></span>
    <span class="l2 r">${todoText(item)}</span>${todoDone(item.id)}</div>`;
}

// `place` keeps the next thing card's button apart from the row's, so focus returns to the one tapped.
function todoDone(id, place = "row") {
  const busy = state.todoBusy !== null || Boolean(state.failed);
  const notice = state.todoNotices.get(id);
  return html`<span class="task-acts"><button id="todo-${place}-${id}" class="btn secondary act" data-action="todo-done" data-todo="${id}" aria-disabled="${busy ? "true" : "false"}">Done</button></span>${notice ? html`<p class="note acts-note" role="status">${notice}</p>` : ""}`;
}

async function markDone(target) {
  const id = Number(target.dataset.todo);
  if (target.getAttribute("aria-disabled") === "true") return;
  state.todoBusy = id;
  render();
  let reply;
  try {
    reply = await post(`/api/todos/${id}/done`, {});
  } catch {
    reply = { status: 0, body: { error: "the connection dropped" } };
  }
  state.todoBusy = null;
  const { status, body } = reply;
  if (status === 401) return signOut("The daemon refused this token. Paste the current one.");
  if (status === 200) state.todoNotices.delete(id);
  else state.todoNotices.set(id, status === 503 ? "Could not send: no priory daemon runs beside this server. Start priory daemon, then send again."
    : status === 504 || status === 0 ? "The daemon may have applied this but did not confirm it. Check the desk before you send again."
      : `Could not mark it done: ${body.error || `the server answered ${status}`}.`);
  render();
  refresh();
}

// An asleep task with what each of its open waits waits on, and since when.
function asleepRow(task) {
  const waits = (task.waits || []).map((wait) => `${wait.kind} ${wait.on}, ${age(wait.since)}`).join("; ");
  return html`<a class="row" href="/tasks/${task.id}">
    ${tile("asleep")}
    <span class="l1"><span class="mono">${taskLabel(task.key)}</span><span>${task.workflow} ${task.step}</span><span>asleep</span>${prsTag(task)}<span class="age">${timeTag(task.since)}</span></span>
    <span class="l2 r">${task.title || task.lane || ""}</span>
    <span class="l3"><span>${waits ? `Waits on ${waits}` : "Waits on an ask"}</span></span>
    <span class="chev">${icon("chevron-right", "s15")}</span></a>`;
}

// "3 PRs" for a task with a stack of two or more. The queue and desk carry no stacks; the tasks listing does.
const prs = (task, id = task.id) => prStack(task.stack ?? state.data.tasks.find((each) => each.id === id)?.stack).label;
const prsTag = (task, id) => {
  const label = prs(task, id);
  return label ? html`<span class="tag">${label}</span>` : "";
};

function taskRow(task) {
  return html`<div class="row">
    ${tile(task.state)}
    <span class="l1"><span class="mono">${taskLabel(task.key)}</span><span>${task.workflow} ${task.step}</span><span>${task.state}</span>${prsTag(task)}<span class="age">${timeTag(task.since)}</span></span>
    <span class="l2 r ${task.reason ? `t-${role(task.state)}` : ""}">${autolinked(task.reason || task.tldr || task.title || task.lane || "", task.repo)}</span></div>`;
}

function runRow(run) {
  return html`<div class="row">
    ${tile("running")}
    <span class="l1"><span class="mono">${taskLabel(run.key)}</span><span>${run.step}</span><span>running</span><span class="age">${timeTag(run.started)}</span></span>
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

const countLine = (model) => countWords({ oldest: model.open[0] ? age(model.open[0].since) : null, stopped: model.stopped.length, todos: model.todos.length, reports: model.reports.length, other: model.other.length });

function upNext(model, queueLink = true) {
  const next = (model.upNext || []).slice(0, 3);
  const when = (run) => {
    if (run.kind === "queued") return onRoom(run) ? "now, when a runner has room" : "now";
    const at = new Date(run.at);
    const time = clock(run.at);
    return today(run.at) ? time : `${time} ${at.toLocaleDateString("en", { weekday: "short" })}`;
  };
  const what = (run) => html`<span class="mono">${run.key ? taskLabel(run.key) : run.trigger}</span> ${run.why}`;
  const body = next.length
    ? html`<ol>${next.map((run) => html`<li>${timeTag(run.at, when(run))}${run.task ? html`<a href="/tasks/${run.task}" title="task ${run.task}">${what(run)}</a>` : html`<span>${what(run)}</span>`}</li>`)}</ol>`
    : html`<p>Nothing is queued or scheduled to run next.</p>`;
  const link = queueLink ? html`<a class="open" href="/queue">Open the queue</a>` : "";
  return html`<div class="sec">Up next<span>next 3 runs</span></div><div class="strip">${body}${link}</div>`;
}

// The one next thing (DESIGN.md, Desk): the desk's `next`, an alarm or the ask that ranks first, and why.
function nextCard(model) {
  const item = model.next;
  if (!item) return "";
  const ask = item.kind === "ask" ? state.data.asks.find((each) => String(each.id) === String(item.id)) : null;
  const task = state.data.tasks.find((each) => each.id === item.task);
  const text = ask ? ask.title || ask.question : ["blocked", "failed"].includes(item.kind) && task?.reason ? task.reason : item.what;
  const kind = ["prompt", "command"].includes(item.kind) ? "prompt" : item.kind === "report" ? "lost" : item.kind;
  const href = nextLink(item);
  const verb = ask ? "Answer…" : href?.startsWith("#") ? "See the banner" : "Open task";
  const button = item.kind === "todo" ? todoDone(item.id, "next") : href ? html`<a class="btn ${ask ? "primary" : "secondary"}" href="${href}">${verb}</a>` : "";
  return html`<section class="next" aria-label="The next thing"><p class="l1">${tile(kind, "sm")}<span class="mono">${taskLabel(item.key)}</span><span>${item.step || ""}</span><span class="age">${timeTag(item.since)}</span></p>
    <p class="q">${item.kind === "todo" ? todoText(item) : autolinked(text, ask?.repo || task?.repo)}</p>${ask && routedMark(ask) ? html`<p class="why t-live">${routedMark(ask)}</p>` : ""}${runnerOf(ask) ? html`<p class="why">${runnerTag(ask)} ${sentLine(ask)}</p>` : ""}<p class="why t-${role(kind)}">${item.why}</p>
    ${ask?.recommendation ? html`<p class="rec">${icon("corner-down-right", "s15")}<span>${ask.recommendation}</span></p>` : ""}
    ${button}</section>`;
}

// Since you looked (DESIGN.md, Desk): what happened since the last visit, one group per task.
function sinceYouLooked() {
  // Until the first live read, the desk kept on this device: an offline start shows its events.
  const { desk } = state.data;
  const [events, from] = state.since.opened ? [state.since.events, state.since.from] : [desk.since_you_looked || [], desk.last_visit];
  const groups = sinceGroups(events);
  const body = groups.length
    ? groups.map((group) => html`<div class="sgroup">${group.task ? html`<a class="mono" href="/tasks/${group.task}" title="task ${group.task}">${taskLabel(group.key)}</a>` : html`<span class="mono">factory</span>`}
      <ol>${group.events.map((event) => html`<li>${timeTag(event.time)}<span>${sinceWords(event.what)}</span></li>`)}</ol></div>`)
    : html`<p>Nothing happened since you looked.</p>`;
  return html`<div class="sec">Since you looked<span>${from ? `since ${today(from) ? clock(from) : date(from)}` : "latest 50"}</span></div><div class="strip since" data-since>${body}</div>`;
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
  const banner = [[model.banner, "hard-drive"], [model.intake, "unplug"]]
    .filter(([text]) => text)
    .map(([text, name]) => html`<div class="banner" role="status" ${name === "unplug" ? raw('id="intake"') : ""}>${icon(name, "s20")}<span>${text}</span></div>`);
  let rows;
  if (current === "running") rows = model.running.length ? model.running.map(runRow) : html`<p class="empty">No run is live. Runs show here while an agent works.</p>`;
  else if (current === "queue") rows = listOrEmpty(model.queue.map(taskRow), html`The queue is empty. A task waits here for its turn; see <code>priory queue</code>.`);
  else if (current === "asleep") rows = listOrEmpty(model.asleepTasks.map(asleepRow), "No task is asleep. A task sleeps here while it waits on an event.");
  else if (current === "all") rows = model.tasks.length || olderTasks() ? [...model.tasks.map(taskRow), olderRow("tasks", olderTasks(), false)] : listOrEmpty([], html`No tasks yet. Add one with the + button, or run <code>priory add</code>.`);
  else rows = needsMe(model, wide);
  return { model, views, banner, rows };
}

// Unrouted (DESIGN.md, Desk): events that reached no task, by resource, each with a way to route the next one.
async function loadUnrouted() {
  const view = state.unrouted;
  if (view.loading || !state.token || !state.data || state.failed) return;
  view.loading = true;
  try {
    const body = await api("/api/inbox?unrouted=1");
    Object.assign(view, { groups: body.unrouted, revision: body.revision, error: null });
  } catch (error) {
    view.error = error.message;
  }
  view.loading = false;
  render();
}

function unroutedRows() {
  const view = state.unrouted;
  if (view.error) return html`<p class="empty">Could not read the unrouted events: ${view.error}. <button class="btn secondary" data-action="retry">Try again</button></p>`;
  if (!view.groups) {
    queueMicrotask(loadUnrouted);
    return html`<p class="empty">Loading the unrouted events…</p>`;
  }
  const { owned, other, unheard = [] } = view.groups;
  const rest = neverOwned(other);
  const restLine = rest ? html`<p class="meta unowned">${rest}</p>` : "";
  const heard = unheard.length ? html`<div class="sec" id="unheard">Unheard<span>no agent read them</span></div>${unheard.map(unheardGroup)}<div class="sec">Unrouted<span>reached no task</span></div>` : "";
  if (!owned.length) return html`${heard}<p class="empty">No event of a task's resource went unrouted. An event about an ended task's pull request or issue shows here.</p>${restLine}`;
  return [heard, ...owned.map((group) => {
    const { task } = group;
    const notice = view.notices.get(group.resource);
    return html`<section class="ugroup" aria-label="${group.resource}"><p class="l1"><span class="mono">${group.resource}</span><span class="num">${group.events.length}</span></p>
      <p class="meta">${tile(task.state, "sm")}<a class="mono" href="/tasks/${task.id}" title="task ${task.id}">${taskLabel(task.key)}</a> ${task.state}</p>
      ${eventRows(group.events, "queued")}
      <div class="routeform">${routeButton(group, routeAction(task))}</div>
      ${notice ? html`<p class="note" role="status">${notice}</p>` : ""}</section>`;
  }), restLine];
}

// A group's 5 newest events as history rows, then how many are older.
function eventRows(events, state) {
  return html`<ol class="hist">${events.slice(0, 5).map((event) => html`<li>${tile(state, "sm")}<span><b title="${event.type}">${eventWords(event.type)}</b> ${event.subject || ""}${event.line ? html`<span class="meta">${event.line}</span>` : ""}</span>${timeTag(event.time)}</li>`)}</ol>
    ${events.length > 5 ? html`<p class="meta">and ${events.length - 5} older</p>` : ""}`;
}

// The one button that routes a group's next event: revive, request a task or subscribe.
function routeButton(group, action) {
  const { task } = group;
  if (action === "revive") return taskActs(task.id, task.state);
  if (action === "request") return group.request_text
    ? html`<button type="button" class="btn secondary" data-action="request-task" data-text="${group.request_text}" data-workflow="${task.workflow}">Request a task</button>`
    : html`<p class="meta">No new task can take this resource from here: start one from its thread.</p>`;
  const busy = state.unrouted.busy === group.resource ? raw('aria-disabled="true"') : "";
  return html`<button type="button" class="btn secondary" data-action="subscribe-task" data-resource="${group.resource}" data-task="${task.id}" ${busy}>Subscribe ${taskLabel(task.key)}</button>`;
}

// One task's unheard events (DESIGN.md, Inbox), with what gets them to an agent.
function unheardGroup(group) {
  const { task } = group;
  const [first, ...rest] = unheardActs(task);
  const busy = state.taskBusy !== null || Boolean(state.failed);
  const act = first === "say"
    ? html`<button type="button" class="btn secondary" data-action="unheard-say" data-task="${task.id}" data-text="${group.say_text}" aria-disabled="${busy ? "true" : "false"}">Say to ${taskLabel(task.key)}</button>${taskActs(task.id, task.state, rest)}
      <p class="meta">Say sends “${group.say_text}”${rest.includes("resume") ? " It reads them when it resumes." : ""}</p>`
    : first ? routeButton({ ...group, resource: task.key }, first)
      : html`<p class="meta">Its next run reads them.</p>`;
  return html`<section class="ugroup" aria-label="${taskLabel(task.key)}"><p class="l1"><a class="mono" href="/tasks/${task.id}" title="task ${task.id}">${taskLabel(task.key)}</a><span class="num">${group.events.length}</span></p>
    <p class="meta">${tile(task.state, "sm")}${task.state}, ${task.workflow}</p>
    ${eventRows(group.events, "asleep")}
    <div class="routeform">${act}</div></section>`;
}

// The Inbox panel (DESIGN.md, Inbox): its one saved view, Unrouted, counts the events of a task's resources.
function inboxViews() {
  const groups = state.unrouted.groups;
  const count = groups ? groups.owned.reduce((sum, group) => sum + group.events.length, 0) : null;
  return savedViews([["unrouted", "Unrouted", count, true]], "/inbox");
}

// Subscribes a task to the resource, as `priory subscribe`, with the revision the page showed.
async function subscribeTask(resource, task) {
  const view = state.unrouted;
  view.busy = resource;
  render();
  let reply;
  try {
    // The revision the Inbox groups were read at, so a group shown from an older read is refused.
    reply = await post("/api/subscribe", { task, resource, revision: view.revision });
  } catch {
    reply = { status: 0, body: { error: "the connection dropped" } };
  }
  view.busy = null;
  const { status, body } = reply;
  if (status === 401) return signOut("The daemon refused this token. Paste the current one.");
  view.notices.set(resource, status === 200 ? `Subscribed: ${body.ok}. Its next event goes to ${taskName(task)}.`
    : status === 409 ? "The state changed since this page loaded. Check the task, then subscribe again."
      : status === 503 ? "Could not send: no priory daemon runs beside this server. Start priory daemon, then send again."
        : status === 504 || status === 0 ? "The daemon may have applied this but did not confirm it. Check the task before you send again."
          : `Could not subscribe: ${body.error || `the server answered ${status}`}.`);
  render();
  refresh();
}

// The Add sheet with the server's text, whose URL keys the new task by the group's resource.
function requestTask(text, workflow) {
  state.add.draft.text = text;
  if (workflow) state.add.draft.workflow = workflow;
  keepDraft();
  openAdd();
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
    if (model.todos.length) sections.push(html`<div class="sec">To-dos<span>${model.todos.length}</span></div>${model.todos.map(todoRow)}`);
    if (model.other.length) sections.push(html`<div class="sec">Also waiting<span>${model.other.length}</span></div>${model.other.map(itemRow)}`);
  }
  sections.push(upNext(model), sinceYouLooked());
  if (model.asleep) sections.push(html`<div class="sec"></div><a class="row one" href="/?view=asleep">${tile("asleep")}<span>${model.asleep} tasks asleep on waits</span><span class="num">${model.asleep}</span><span class="chev">${icon("chevron-right", "s15")}</span></a>`);
  const unrouted = unroutedNote(state.data.desk.counts?.unrouted);
  if (unrouted) sections.push(html`<a class="row one calm" href="/inbox?view=unrouted">${tile("queued")}<span>${unrouted}</span><span class="num">${state.data.desk.counts.unrouted}</span><span class="chev">${icon("chevron-right", "s15")}</span></a>`);
  const unheard = unheardNote(state.data.desk.counts?.unheard);
  if (unheard) sections.push(html`<a class="row one calm" href="/inbox?view=unrouted#unheard">${tile("asleep")}<span>${unheard}</span><span class="num">${state.data.desk.counts.unheard}</span><span class="chev">${icon("chevron-right", "s15")}</span></a>`);
  if (model.reports.length) sections.push(html`<div class="sec">Reports<span>${model.reports.length}</span></div>${model.reports.map(itemRow)}`);
  return sections;
}

// The fifth column's content, again under the question: CSS shows it when that column folds. Its
// ids take a prefix, so each stays unique and focus comes back to the copy that was used.
const fold = (content) => (content ? html`<div class="fifth">${raw(html`${content}`.text.replaceAll(' id="', ' id="fold-'))}</div>` : "");
const review = html`<span class="task-acts"><a class="btn secondary act" href="/memories">Review</a></span>`;
const itemActs = (item) => (["blocked", "failed"].includes(item.kind) ? taskActs(item.task, item.kind) : rowActs(item));

function deskTable(model, selected) {
  const itemLine = (item) => item.kind === "memories" ? html`<tr>
    <td>${stateWord("memories")}</td><td class="mono"></td><td></td><td class="q full"><span>${item.candidates?.length ?? 0} memory candidates: ${(item.candidates || []).map((candidate) => candidate.name).join(", ")}</span>${fold(review)}</td>
    <td class="recw">${review}</td><td class="r">${timeTag(item.since)}</td></tr>` : html`<tr>
    <td>${stateWord(item.kind === "prompt" ? "prompt" : item.kind)}</td><td class="mono">${item.kind === "batch" ? html`<a href="/batches/${item.id}">${taskLabel(item.key)}</a>` : ["blocked", "failed"].includes(item.kind) ? html`<a href="/tasks/${item.task}" title="task ${item.task}">${taskLabel(item.key)}</a>` : taskLabel(item.key)}</td><td>${item.step || ""}</td>
    <td class="q full t-${role(item.kind)}">${item.task ? prsTag({}, item.task) : ""}<span>${item.what}</span>${fold(itemActs(item))}</td><td class="recw">${itemActs(item)}</td><td class="r">${timeTag(item.since)}</td></tr>`;
  const todoLine = (item) => html`<tr>
    <td>${stateWord("todo")}</td><td class="mono"></td><td></td><td class="q full"><span>${todoText(item)}</span>${fold(todoDone(item.id))}</td><td class="recw">${todoDone(item.id)}</td><td class="r">${timeTag(item.since)}</td></tr>`;
  const group = (title, count) => html`<tr class="grp"><td colspan="6">${title}<span>${count}</span></td></tr>`;
  return html`<table class="tbl">
    <thead><tr><th>State</th><th>Task</th><th>Step</th><th>Waits on you</th><th>Recommends</th><th class="r">Age</th></tr></thead>
    <tbody>${model.open.map((ask) => askTableRow(ask, selected))}
    ${model.stopped.length ? [group("Stopped", model.stopped.length), model.stopped.map(itemLine)] : ""}
    ${model.todos.length ? [group("To-dos", model.todos.length), model.todos.map(todoLine)] : ""}
    ${model.other.length ? [group("Also waiting", model.other.length), model.other.map(itemLine)] : ""}
    </tbody></table>`;
}

const addButton = (kind) => html`<button class="${kind}" data-action="add-open" aria-label="Add a task" aria-keyshortcuts="N" aria-haspopup="dialog">${icon("plus", "s20")}${kind === "addtop" ? "Add" : ""}</button>`;

// On a phone the Add button sits beside the capsule, or alone where the capsule hides.
function capsule(model) {
  const n = model.open.length;
  const answer = n && deskView() === "needs" ? html`<a class="capsule" href="/asks/${model.first.id}">${icon("ask", "s20")}Answer ${n} ${n === 1 ? "ask" : "asks"}</a>` : "";
  return html`<div class="fade"></div><div class="dock">${answer}${addButton("fab")}</div>`;
}

// Asks list
function askView() {
  const name = new URLSearchParams(listSearch()).get("view");
  return ASK_VIEWS.includes(name) ? name : "open";
}

function askTableRow(ask, selected) {
  const answered = ask.state === "answered";
  const when = timeTag(answered && ask.answered_at ? ask.answered_at : ask.since);
  const status = answered ? ask.answer + byLine(ask) : ask.state === "withdrawn" ? ask.withdrawn_why || "" : snoozed(ask, Date.now()) ? snoozedUntil(ask) : sentNote(ask) ? sentLine(ask) : null;
  const folded = status ?? (ask.recommendation ? html`Recommends <b>${ask.recommendation}</b>` : "");
  return html`<tr data-href="/asks/${ask.id}" aria-current="${String(ask.id) === selected ? "true" : "false"}">
    <td>${stateWord(ask.state === "open" ? "ask" : ask.state)}</td><td class="mono">${taskLabel(ask.key)}</td><td>${ask.step}</td><td class="q"><span>${ask.title || ask.question}</span>${runnerTag(ask)}${routedTag(ask)}${fold(folded)}</td>
    <td class="recw">${status ?? ask.recommendation ?? ""}</td><td class="r">${when}</td></tr>`;
}

// Each facet's key, label, an ask's values and the order the picker offers them. An ask with no
// band, such as an outside ask, shows under `none`, and so does an ask with no risk.
const FACETS = [
  ["band", "Band", (ask) => [ask.band ?? "none"], () => [...(state.data.bands || []), "none"]],
  ["risk", "Risk", (ask) => (ask.risk?.length ? ask.risk : ["none"]), () => ["irreversible", "high", "medium", "low", "red", "yellow", "green", "none"]],
  ["runner", "Runner", (ask) => [ask.runner ?? "none"], null, (name) => (state.data.asks.some((ask) => ask.runner === name && !runnerOf(ask)) ? `${name} (here)` : name)],
];
// The Runner facet shows only in a factory of several runners, where every ask names one.
const shownFacets = () => FACETS.filter(([key]) => key !== "runner" || state.data.asks.some((ask) => ask.runner));
const facetPicks = (facets = FACETS) => {
  const search = new URLSearchParams(listSearch());
  return new Map(facets.map(([key]) => [key, search.get(key)]).filter(([, value]) => value));
};
const faceted = (asks, picks) => filtered(asks, picks, FACETS);
const facetQuery = (picks) => [...picks].map(([key, value]) => `&${key}=${encodeURIComponent(value)}`).join("");

// A facet chip of the list at `base` over `items`; a facet with no order offers its values A to Z.
function facet(view, picks, [key, label, values, order, named = (name) => name], base = "/asks", items = state.data.asks) {
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
const listView = (views) => {
  const name = new URLSearchParams(location.search).get("view");
  return views.includes(name) ? name : views[0];
};

// Tasks (DESIGN.md, Tasks panel): every task, by saved view and facets; each row opens its page.
// A task by its id, named by its key once the task list holds it.
const taskName = (id) => {
  const task = state.data?.tasks?.find((each) => each.id === id);
  return task ? taskLabel(task.key) : `task ${id}`;
};
const batchTag = (task) => (task.batch == null ? "" : html`<a class="tag" href="/batches/${task.batch}">${icon("layers", "s13")}batch ${task.batch}</a>`);
const reasonShown = (task) => Boolean(task.reason) && ["blocked", "held", "failed", "dropped"].includes(task.state);

// "12 min ago", "just now", or the date for a time over a day ago.
const ago = (iso) => {
  const text = age(iso);
  if (text === "just now") return text;
  return / (min|h)$/.test(text) ? `${text} ago` : `on ${text}`;
};

// Another runner's task (DESIGN.md, Task row and run row): its hold for a state, and no page here.
// A tap opens its actions in place; the newest action sent to it takes a line of its own.
const TONE = { action: "t-action", warn: "t-warn", quiet: "" };
const toggle = (id, open, key) => html`<button type="button" id="remote-${id}" class="chev" data-action="remote-open" data-task="${id}" aria-expanded="${open ? "true" : "false"}" aria-label="${open ? "Close" : "Open"} the actions of ${key}">${icon(open ? "chevron-up" : "chevron-down", "s15")}</button>`;
const sentOf = (task) => {
  const sent = remoteSent(task);
  return sent ? html`<span class="${TONE[sent.tone]}">${sent.text}</span>` : "";
};
function remoteActions(task) {
  const id = remoteId(task);
  const said = state.taskBusy === id ? raw('aria-disabled="true"') : "";
  const say = task.hold === "kept" ? html`<form class="writeform" data-action="say" data-task="${id}"><label class="sr" for="say">Say to the agent</label>
    <textarea id="say" name="text" rows="2" placeholder="Say to the agent">${state.sayDrafts.get(id) || ""}</textarea><button class="btn primary" ${said}>Say</button></form>` : "";
  return html`${taskActs(id, null, remoteActs(task))}${say}<p class="note acts-note">${icon("corner-down-right", "s15")}<span>This task runs on ${task.runner}. An action goes there through the log, and ${task.runner} applies it when it reads it.</span></p>`;
}

function remoteListRow(task) {
  const id = remoteId(task);
  const open = state.remoteOpen === id;
  const sent = sentOf(task);
  return html`<div class="row remote" data-action="remote-open" data-task="${id}">${tile("elsewhere")}
    <span class="l1"><span class="mono">${taskLabel(task.key)}</span><span>${task.workflow || ""}</span><span>${task.hold}</span><span class="age">${timeTag(task.since)}</span></span>
    <span class="l2 r">${task.title || taskLabel(task.key)}</span>${sent ? html`<span class="l3">${sent}</span>` : ""}<span class="l3"><span>${elsewhere(task, ago)}</span></span>
    ${toggle(id, open, taskLabel(task.key))}${open ? remoteActions(task) : ""}</div>`;
}

function remoteTableRow(task) {
  const id = remoteId(task);
  const open = state.remoteOpen === id;
  const sent = sentOf(task);
  const row = html`<tr class="remote" data-action="remote-open" data-task="${id}"><td><span class="sti">${tile("elsewhere", "sm")}${task.hold}</span></td><td class="mono">${taskLabel(task.key)}</td>
    <td class="q full"><span>${task.title || taskLabel(task.key)}</span>${sent ? html`<span class="why">${sent}</span>` : ""}<span class="why t3">${elsewhere(task, ago)}</span></td>
    <td>${task.workflow || ""}</td><td>${task.lane || ""}</td><td>${task.band || ""}</td><td class="r">${timeTag(task.since)} ${toggle(id, open, taskLabel(task.key))}</td></tr>`;
  return open ? html`${row}<tr class="remote-acts"><td></td><td></td><td colspan="5">${remoteActions(task)}</td></tr>` : row;
}

function taskListRow(task) {
  if (elsewhere(task, ago)) return remoteListRow(task);
  const line3 = reasonShown(task) ? html`<span class="t-${role(task.state)}">${task.reason}</span>${batchTag(task)}` : html`<span>${task.lane || ""}</span><span>${task.band || ""}</span>${batchTag(task)}`;
  return html`<div class="row" data-href="/tasks/${task.id}">${tile(task.state)}
    <span class="l1"><a class="mono" href="/tasks/${task.id}" title="task ${task.id}">${taskLabel(task.key)}</a><span>${task.workflow} ${task.step}</span><span>${task.state}</span>${prsTag(task)}<span class="age">${timeTag(task.since)}</span></span>
    <span class="l2 r">${task.title || task.tldr || ""}</span><span class="l3">${line3}</span><span class="chev">${icon("chevron-right", "s15")}</span></div>`;
}

function taskTableRow(task) {
  if (elsewhere(task, ago)) return remoteTableRow(task);
  return html`<tr data-href="/tasks/${task.id}"><td>${stateWord(task.state)}</td><td class="mono"><a href="/tasks/${task.id}" title="task ${task.id}">${taskLabel(task.key)}</a></td>
    <td class="q full"><span>${task.title || task.tldr || ""}${task.batch == null ? "" : " "}${batchTag(task)}</span>${reasonShown(task) ? html`<span class="why t-${role(task.state)}">${task.reason}</span>` : ""}</td>
    <td>${task.workflow} <span class="t3">${task.step}</span></td><td>${task.lane || ""}</td><td>${task.band || ""}</td><td class="r">${timeTag(task.since)}</td></tr>`;
}

function tasksBody(wide) {
  const all = state.data.tasks;
  const specs = taskFacets(all);
  const picks = facetPicks(specs);
  const current = listView(TASK_VIEWS);
  const views = taskViews(filtered(all, picks, specs));
  const { counted, unloaded } = olderPart({ view: current, history: "ended", older: olderTasks(), picked: picks.size > 0 });
  const chips = savedViews([
    ["active", "Active", views.active.length, current === "active"],
    ["blocked", "Blocked", views.blocked.length, current === "blocked"],
    ["ended", "Ended", views.ended.length + counted, current === "ended"],
  ], "/tasks", facetQuery(picks));
  const facets = html`<div class="facets" aria-label="Facets">${specs.map((spec) => facet(current, picks, spec, "/tasks", all))}</div>`;
  const list = views[current];
  const empty = { active: html`No active tasks. Add one with the + button, or run <code>priory add</code>.`, blocked: "No blocked tasks. A task shows here when it needs a person to go on.", ended: "No task ended in the last 7 days. A task shows here once it is done, failed or dropped." }[current];
  if (!list.length && !unloaded) return { views: html`${chips}${facets}`, rows: html`<p class="empty">${empty}</p>` };
  const rows = wide
    ? html`<table class="tbl tasks"><thead><tr><th>State</th><th>Task</th><th>Title</th><th>Workflow and step</th><th>Lane</th><th>Band</th><th class="r">Age</th></tr></thead><tbody>${list.map(taskTableRow)}${olderRow("tasks", unloaded, true, 7)}</tbody></table>`
    : [...list.map(taskListRow), olderRow("tasks", unloaded, false)];
  return { views: html`${chips}${facets}`, rows };
}

// Runs (DESIGN.md, Runs panel): the runs of the last 7 days, newest first; each row opens its task.
function runListRow(run) {
  const tokens = tokenText(run.tokens);
  return html`<div class="row" data-href="/tasks/${run.task}">${tile(runState(run))}
    <span class="l1"><a class="mono" href="/tasks/${run.task}" title="task ${run.task}">${taskLabel(run.key)}</a><span>${run.step}</span><span>${runState(run)}</span><span class="age">${timeTag(run.started)}</span></span>
    <span class="l2 r">${[run.agent, run.model].filter(Boolean).join(", ") || "no agent"}</span>
    <span class="l3"><span>run ${run.run}</span>${run.outcome ? html`<span>${run.outcome}</span>` : ""}<span>${duration(run, Date.now())}</span>${tokens ? html`<span>${tokens} tokens</span>` : ""}</span>
    <span class="chev">${icon("chevron-right", "s15")}</span></div>`;
}

function runTableRow(run) {
  return html`<tr data-href="/tasks/${run.task}"><td>${stateWord(runState(run))}</td><td class="r">${run.run}</td><td class="mono"><a href="/tasks/${run.task}" title="task ${run.task}">${taskLabel(run.key)}</a></td><td>${run.step}</td>
    <td>${run.agent || ""} <span class="t3">${run.model || ""}</span></td><td>${run.outcome || ""}</td><td class="r">${duration(run, Date.now())}</td><td class="r">${tokenText(run.tokens)}</td><td class="r">${timeTag(run.started)}</td></tr>`;
}

function runsBody(wide) {
  const picks = facetPicks(RUN_FACETS);
  const current = listView(RUN_VIEWS);
  const all = state.data.runs ?? [];
  const views = runViews(filtered(all, picks, RUN_FACETS));
  const { counted, unloaded } = olderPart({ view: current, history: "recent", older: state.data.older.runs, picked: picks.size > 0 });
  const chips = savedViews([["recent", "Recent", views.recent.length + counted, current === "recent"], ["live", "Live", views.live.length, current === "live"]], "/runs", facetQuery(picks));
  const facets = html`<div class="facets" aria-label="Facets">${RUN_FACETS.map((spec) => facet(current, picks, spec, "/runs", all))}</div>`;
  const list = views[current];
  const empty = current === "live" ? "No run is live. A run shows here while an agent works on a step." : "No run in the last 7 days. A run shows here once a task's step starts an agent.";
  if (!list.length && !unloaded) return { views: html`${chips}${facets}`, rows: html`<p class="empty">${empty}</p>` };
  const rows = wide
    ? html`<table class="tbl runs"><thead><tr><th>State</th><th class="r">Run</th><th>Task</th><th>Step</th><th>Agent and model</th><th>Outcome</th><th class="r">Duration</th><th class="r">Tokens</th><th class="r">Started</th></tr></thead><tbody>${list.map(runTableRow)}${olderRow("runs", unloaded, true, 9)}</tbody></table>`
    : [...list.map(runListRow), olderRow("runs", unloaded, false)];
  return { views: html`${chips}${facets}`, rows };
}

function asksBody(wide) {
  const picks = facetPicks();
  const asks = faceted(state.data.asks, picks);
  const current = askView();
  const { open, snoozed: asleep, answered, withdrawn } = askGroups(asks, Date.now());
  // shortcut: older counts are not per facet, so a facet counts the loaded rows only; split `older` by facet if that misleads.
  const pending = state.data.older.asks;
  const left = picks.size ? { answered: 0, withdrawn: 0 } : pending;
  const total = { open: open.length, snoozed: asleep.length, answered: answered.length + left.answered, all: asks.length + left.answered + left.withdrawn };
  const unloaded = { open: 0, snoozed: 0, answered: pending.answered, all: pending.answered + pending.withdrawn }[current];
  const views = savedViews([
    ["open", "Open", total.open, current === "open"],
    ["snoozed", "Snoozed", total.snoozed, current === "snoozed"],
    ["answered", "Answered", total.answered, current === "answered"],
    ["all", "All", total.all, current === "all"],
  ], "/asks", facetQuery(picks));
  const facets = html`<div class="facets" aria-label="Facets">${shownFacets().map((spec) => facet(current, picks, spec))}</div>`;
  const selected = selectedAsk();
  const section = (title, list, note, count = list.length) => {
    if (!count) return "";
    if (wide) return html`<tr class="grp"><td colspan="6">${title}<span>${note}</span></td></tr>${list.map((ask) => askTableRow(ask, selected))}`;
    return html`<div class="sec">${title}<span>${note}</span></div>${list.map((ask) => askRow(ask, String(ask.id) === selected))}`;
  };
  const answeredToday = answered.filter((ask) => ask.answered_at && today(ask.answered_at));
  const answeredEarlier = answered.filter((ask) => !answeredToday.includes(ask));
  const shows = (group) => shownGroups(current).includes(group);
  const sections = [
    shows("open") ? section("Open", open, `${open.length}, oldest first`) : "",
    shows("snoozed") ? section("Snoozed", asleep, asleep.length) : "",
    shows("answered") ? section("Answered today", answeredToday, answeredToday.length) : "",
    shows("answered") ? section("Answered earlier", answeredEarlier, answeredEarlier.length + left.answered, answeredEarlier.length + left.answered) : "",
    shows("withdrawn") ? section("Withdrawn", withdrawn, withdrawn.length + left.withdrawn, withdrawn.length + left.withdrawn) : "",
    olderRow("asks", unloaded, wide),
  ];
  const empty = { open: "No open asks. Agents ask here when they need you.", snoozed: "No snoozed asks. A snoozed ask waits here until its time.", answered: "No answered asks yet. An ask moves here once someone answers it.", all: "No asks yet. Agents ask here when they need you." }[current];
  const rows = wide ? html`<table class="tbl"><thead><tr><th>State</th><th>Task</th><th>Step</th><th>Question</th><th>${current === "open" ? "Recommends" : "Answer"}</th><th class="r">Age</th></tr></thead><tbody>${sections}</tbody></table>` : sections;
  return { views: html`${views}${facets}`, rows: total[current] || unloaded ? rows : html`<p class="empty">${empty}</p>`, open };
}

// Ask detail
const selectedAsk = () => {
  const [first, id] = route();
  return first === "asks" && id ? decodeURIComponent(id) : null;
};

// One button per option, as the detail and the answer sheet draw them. A command option armed by its first tap shows its command.
function optionList(ask, wide) {
  const pending = state.pending?.id === ask.id ? state.pending : null;
  const armed = state.armed?.id === ask.id && state.armed.revision === ask.revision ? state.armed.text : null;
  const locked = lockOf(ask);
  const bound = `${runnerOf(ask) ? `Runs on ${runnerOf(ask)}. ` : ""}Bound to revision ${ask.revision}.`;
  return (ask.options || []).map((option, i) => {
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
}

function askDetail(id, wide) {
  const ask = state.data.asks.find((candidate) => String(candidate.id) === id);
  const { answered, withdrawn } = state.data.older.asks;
  const heading = layout() === "compact" ? html`<h1 class="sr">Ask #${id}</h1>` : "";
  if (!ask && answered + withdrawn) {
    loadOlder("asks");
    return html`${heading}<p class="empty">Loading ask ${id}…</p>`;
  }
  if (!ask) return html`${heading}<p class="empty">No ask ${id} here. It may belong to another factory. Check <code>priory asks --all</code>.</p>`;
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
  const armed = state.armed?.id === ask.id && state.armed.revision === ask.revision ? state.armed.text : null;
  const batch = goBatch(ask);
  const locked = lockOf(ask);
  const bound = `${runnerOf(ask) ? `Runs on ${runnerOf(ask)}. ` : ""}Bound to revision ${ask.revision}.`;
  const optionButtons = optionList(ask, wide);
  const notice = state.notices.get(ask.id);
  const lockedNote = locked === "offline" ? html`<p class="note" id="answer-locked">${icon("unplug", "s15")}<span>Reconnect to answer.</span></p>`
    : locked ? html`<p class="note" id="answer-locked">${icon("layers", "s15")}<span>${state.batch?.error ? `Could not read the plan: ${state.batch.error}. It must load before you answer.` : `Reading the plan of revision ${ask.revision} before you answer.`}</span></p>` : "";
  const writing = state.writing.has(ask.id) && !locked;
  const write = !takesText(ask)
    ? ""
    : writing
    ? html`<form class="writeform" data-action="send-text"><label class="sr" for="draft">Your answer</label><textarea id="draft" name="text" rows="3">${state.drafts.get(ask.id) || ""}</textarea><button class="btn primary" ${pending ? raw('aria-disabled="true"') : ""}>Send</button></form>`
    : html`<button class="ghost write" data-action="write" aria-disabled="${locked || pending ? "true" : "false"}">${icon("pencil", "s15")}Write an answer</button>`;
  const busy = pending ? "true" : "false";
  const snoozeButtons = !ask.task || runnerOf(ask) || locked === "offline"
    ? ""
    : ask.snoozed_until
      ? html`<button class="ghost" data-action="unsnooze" aria-disabled="${busy}">Unsnooze</button>`
      : html`<button class="ghost" data-action="snooze" data-for="1h" aria-disabled="${busy}">Snooze 1 h</button><button class="ghost" data-action="snooze" data-tomorrow="9" aria-disabled="${busy}">Snooze until tomorrow 09:00</button>`;
  const editable = Boolean(batch) && ask.state === "open" && batch.state === "asked" && !locked;
  const editPlanLink = editable ? html`<a class="ghost" href="#batchplan">${icon("pencil", "s15")}Edit the plan</a>` : "";
  const more = html`<div class="more">${editPlanLink}${write}${locked || !takesText(ask) ? "" : html`<button class="ghost" data-action="dismiss" aria-disabled="${busy}">Dismiss</button>`}${snoozeButtons}</div>`;
  const owner = runnerOf(ask);
  const away = seenAgo(ask, Date.now(), age);
  const sent = sentNote(ask);
  // A closed ask still says why its runner refused an answer sent to it.
  const sentShown = !sent || notice || (ask.state !== "open" && !sent.refused) ? "" : sent.refused
    ? html`<p class="warnline" role="status">${sent.text}.</p>`
    : html`<p class="routednote" role="status">${icon("corner-down-right", "s15")}<span>${sent.text}. It applies when ${owner} reads the log.${away ? ` ${owner} was ${away}.` : ""}</span></p>`;
  const elsewhere = owner ? html`<p class="note">${icon("corner-down-right", "s15")}<span>This ask belongs to ${owner}. Your answer goes there through the log, and ${owner} applies it when it reads it. Snooze stays on ${owner}.</span></p>` : "";
  const decided = ask.state !== "open"
    ? html`${notice ? html`<p class="warnline" role="status">${notice}</p>` : ""}${sentShown}<div class="rec">${ask.state === "answered" ? html`<span><b>Answered: ${ask.answer}.</b> ${ask.answered_at ? ago(ask.answered_at) : ""}${byLine(ask) ? ` By ${answeredBy(ask.answered_by)}.` : ""}</span>` : html`<span><b>Withdrawn${ask.withdrawn_why ? `: ${ask.withdrawn_why}` : ""}.</b></span>`}</div>${ranLine(ask.ran)}`
    : html`${notice ? html`<p class="warnline" role="status">${notice}</p>` : ""}${sentShown}${routedNote(ask) ? html`<p class="routednote">${icon("corner-down-right", "s15")}${routedNote(ask)}</p>` : ""}${ask.recommendation ? html`<div class="rec"><span><b>Recommended: ${ask.recommendation}.</b></span>
        ${ask.next_action ? html`<span class="then">${icon("corner-down-right", "s15")}<span>${ask.next_action}</span></span>` : ""}</div>` : ""}
      <div class="opts" role="group" aria-label="Options">${optionButtons}</div>${lockedNote}${more}${elsewhere}`;
  const question = autolinked(ask.question || ask.title, ask.repo);
  const top = html`<div class="ask">
    <a class="ctx" href="${ask.task && !owner ? `/tasks/${ask.task}` : "/asks"}" title="${ask.task && !owner ? `task ${ask.task}` : ""}">${wide ? "" : tile("ask", "sm")}<span class="mono">${taskLabel(ask.key)}</span><span>${ask.step}</span><span>${ask.lane || ""}</span>${ask.command ? html`<span class="tag">Command</span>` : ""}${runnerTag(ask)}${wide ? "" : html`<span>${timeTag(ask.since)}</span><span>revision ${ask.revision}</span>`}</a>
    ${ask.risk_estimate ? html`<p class="note">${icon("lock", "s15")}<span>${ask.risk_estimate.line}</span></p>` : ""}
    ${layout() === "compact" ? html`<h1 class="question">${question}</h1>` : html`<h2 class="question">${question}</h2>`}${decided}</div>`;
  // A go ask reads its batch's plan first; the planning task's own plan result says the same.
  const planned = batch ? [
    ["batchplan", "Plan", html`<section class="sect" id="batchplan"><div class="sh">${icon("layers", "s15")}Plan, ${batch.entries.length} tasks in start order<span class="meta">rev ${batch.plan_revision}</span></div>${laneKey()}${laneGraph(batch, editable)}${planEditOf(batch).note ? html`<p class="note" role="status">${planEditOf(batch).note}</p>` : ""}</section>`],
    ...(batch.closes?.length ? [["closes", "Closes", closesSection(batch)]] : []),
    ...(splitsOf(batch).length ? [["splits", "Splits", splitsSection(batch)]] : []),
  ] : [];
  const read = taskSections(taskOf(ask)).filter(([key]) => !batch || key !== "plan");
  const sections = [...planned.map(([key, label]) => [key, label]), ["why", "Why"], ...read.map(([key, label]) => [key, label]), ["history", "History"], ...(ask.command ? [["command", "Command"]] : [])];
  const jump = html`<nav class="jump" aria-label="Sections">${sections.map(([key, label], i) => html`<a href="#${key}" aria-current="${i === 0 ? "true" : "false"}">${label}</a>`)}</nav>`;
  const why = html`<section class="sect" id="why"><div class="sh">${icon("message-text", "s15")}What the agent asks<span class="meta">${ask.run ? html`run <span class="mono">${ask.run}</span>` : ""}</span></div>
    ${md(ask.text || ask.question, ask.repo)}
    ${ask.tradeoff ? html`<p class="fact">${icon("scale", "s15")}<span><b>Trade-off.</b> ${ask.tradeoff}</span></p>` : ""}
    ${ask.what_waits ? html`<p class="fact">${icon("hourglass", "s15")}<span><b>What waits.</b> ${ask.what_waits}</span></p>` : ""}</section>`;
  const command = !ask.command ? "" : typeof ask.command === "string"
    ? html`<section class="sect" id="command"><div class="sh">${icon("terminal", "s15")}Command the asker attached</div>
      <div class="cmd"><span class="mono">$ ${ask.command}</span><small>Priory stores this command and never runs it: the ask came from outside Priory.</small></div></section>`
    : html`<section class="sect" id="command"><div class="sh">${icon("terminal", "s15")}Command an answer runs</div>
      ${commandBlock(ask, `${owner ? `Runs on ${owner} when` : "Runs when"} answered ${ask.command.runs.join(" or ")}. Bound to revision ${ask.revision}. Runs at most once.`)}</section>`;
  // Every revision and refused answer, newest first; an ask listed before Priory kept them shows its current revision.
  const dated = [
    ...(ask.revisions || [{ revision: ask.revision, at: ask.since, answer: ask.answer, answered_at: ask.answered_at }]).flatMap((revision) => [
      [revision.at, html`<li>${tile("ask", "sm")}<span>Asked revision <b>${revision.revision}</b>${revision.question ? `: ${revision.question}` : ""}</span>${timeTag(revision.at)}</li>`],
      ...(revision.answer == null ? [] : [[revision.answered_at || revision.at, html`<li>${tile("answered", "sm")}<span>Answered <b>${revision.answer}</b>${ask.state === "answered" && revision.revision === ask.revision ? byLine(ask) : ""}</span>${timeTag(revision.answered_at || revision.at)}</li>`]]),
    ]),
    ...(ask.refused || []).map((refusal) => [refusal.at, html`<li>${tile("refused", "sm")}<span class="t-warn"><b>Refused:</b> ${refusal.answer}, sent for revision ${refusal.revision} from ${refusal.by}: ${refusal.why}</span>${timeTag(refusal.at)}</li>`]),
    ...submitted(ask).map((sent) => [sent.at, html`<li>${tile(sent.tile, "sm")}<span class="${sent.tile === "refused" ? "t-warn" : ""}">${sent.text}</span>${timeTag(sent.at)}</li>`]),
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
      ${locked === "offline" ? html`<p class="note">${icon("unplug", "s15")}<span>Reconnect to answer.</span></p>` : ""}</div>`
    : "";
  const reading = read.map(([, , section]) => section);
  const plan = planned.map(([, , section]) => section);
  if (wide) return html`<div class="pinned">${header}${top}${sending}</div><div class="scroll" data-sections>${jump}${plan}${why}${reading}${history}${command}</div>`;
  return html`${header}<div class="scroll" data-sections>${top}${jump}${plan}${why}${reading}${history}${command}</div>${bar}${sending}`;
}

// The ask's task as /api/tasks/{id} serves it, read once per ask and again after each refresh.
function taskOf(ask) {
  if (!ask.task || runnerOf(ask)) return null;
  if (state.task?.id === ask.task) return state.task.body;
  state.task = { id: ask.task, body: null };
  api(`/api/tasks/${ask.task}`).then((body) => {
    if (state.task?.id !== ask.task) return;
    state.task.body = body;
    render();
  }, (error) => {
    if (state.task?.id !== ask.task) return;
    state.task.error = error.message;
    render();
  });
  return null;
}

// A batch as /api/batches/{id} serves it: by id on its page, by its planning task's key in a go ask.
// `version` is the go ask's revision: a plan read for another revision is never drawn beside it, so a
// Go never answers a plan the page did not show. A refresh marks it stale; a newer read supersedes.
let batchCalls = 0;
function batchOf(which, version = null) {
  const kept = state.batch?.which === which ? state.batch : null;
  if (kept && !kept.stale && kept.version === version) return kept.body;
  const call = ++batchCalls;
  state.batch = { which, version, call, body: kept?.version === version ? kept.body : null, error: null, stale: false };
  // A batch id or a `batch:YYYY-MM-DD-n` key: path-safe, and the server does not decode the path.
  api(`/api/batches/${which}`).then((body) => {
    if (state.batch?.call !== call) return;
    state.batch.body = body;
    render();
  }, (error) => {
    if (state.batch?.call !== call) return;
    state.batch.error = error.message;
    render();
  });
  return state.batch.body;
}

// Why the options of an ask are disabled: offline, or a batch's ask whose plan is not read yet for
// this revision, so a Go never answers a plan the page did not show.
function lockOf(ask) {
  if (state.failed) return "offline";
  if (runnerOf(ask)) return null;
  const read = state.batch?.which === ask.key && state.batch.version === ask.revision && state.batch.body;
  return ask.state === "open" && ask.key?.startsWith("batch:") && !read ? "plan" : null;
}

// The go ask's batch, once the batch names this ask as its go, read for this revision of the ask.
function goBatch(ask) {
  // Another runner's batch lives on that runner; this server cannot read its plan.
  if (!ask.key?.startsWith("batch:") || runnerOf(ask)) return null;
  const batch = batchOf(ask.key, ask.revision);
  return batch?.ask === ask.id ? batch : null;
}

// An open menu or an armed Drop holds for one batch at one plan revision; the last edit's note, for the batch.
const planEditAt = (batch) => `${batch.id}:${batch.plan_revision}`;
function planEditOf(batch) {
  const edit = state.planEdit;
  if (edit.at === planEditAt(batch)) return edit;
  return { open: null, armed: null, busy: false, note: edit.batch === batch.id ? edit.note : null };
}

// The plan's splits as the planner sent them, each kept only when it names an issue.
const splitsOf = (batch) => (batch.splits || []).filter((split) => typeof split?.issue === "string");

const laneKey = () => html`<p class="key-line"><span class="moment"><i class="mk stack" aria-hidden="true"></i>stacks: starts once the one above waits on review</span><span class="moment"><i class="mk" aria-hidden="true"></i>starts once the one above merges</span></p>`;

// The lane graph (DESIGN.md, Lane graph): one row per entry in start order. Tiles and numbers sit in
// each row; drawLanes adds the lines once the rows are laid out.
function laneGraph(batch, editable) {
  const entries = batch.entries;
  const { rows, lanes, edges } = laneLayout(entries, layout() === "compact" ? 3 : Infinity);
  const edit = planEditOf(batch);
  const items = rows.map(({ entry, lane }, i) => {
    const task = entry.task ? state.data.tasks.find((each) => each.id === entry.task) : null;
    const node = entry.state ? tile(entry.state, "sm") : html`<span class="node num">${i + 1}</span>`;
    const pr = task?.stack?.at(-1);
    const status = task ? [pr && `#${pr.number} ${pr.state}`, task.reason || [task.step, age(task.since)].filter(Boolean).join(", ")].filter(Boolean).join(", ") : "";
    const open = editable && edit.open === entry.entry;
    const id = (action) => focusId("batch-edit", batch.id, entry.entry, action);
    const item = (action, glyph, label) => html`<button class="btn secondary" id="${id(action)}" data-action="batch-edit" data-edit="${action}" data-entry="${entry.entry}" aria-disabled="${edit.busy || (action !== "drop" && !editBody(0, entries, entry.entry, action)) ? "true" : "false"}">${icon(glyph, "s15")}${label}</button>`;
    const armed = open && edit.armed === entry.entry;
    const after = armed ? successors(entries, entry.entry).map((other) => issueName(entries.find((each) => each.entry === other))) : [];
    return html`<li class="erow ${entry.stage === "dropped" ? "dropped" : ""}" data-lane="${lane}">
      <span class="slot">${node}</span>
      <span class="l1">${entry.task ? html`<a class="mono" href="/tasks/${entry.task}" title="task ${entry.task}">${issueName(entry)}</a>` : html`<span class="mono">${issueName(entry)}</span>`}${entry.part != null ? html`<span class="tag">split</span>` : ""}${entry.task ? "" : html`<span>${(entry.files || []).join(", ")}</span>`}${entry.state ? html`<span class="sti t-${role(entry.state)}">${entry.state}</span>` : ""}</span>
      <span class="l2">${entry.title}</span>
      <span class="l3"><span>${momentWords(entry, entries)}</span>${status ? html`<span>${autolinked(status, batch.repo)}</span>` : ""}</span>
      ${editable ? html`<button class="iconbtn menu" id="${id("menu")}" data-action="entry-menu" data-entry="${entry.entry}" aria-expanded="${open ? "true" : "false"}" aria-label="Edit ${issueName(entry)}">${icon("more", "s20")}</button>` : ""}
      ${open ? html`<span class="emenu">${item("up", "arrow-up", "Move up")}${item("down", "arrow-down", "Move down")}${armed
        ? html`<button class="btn secondary armed" id="${id("drop")}" data-action="batch-edit" data-edit="drop" data-entry="${entry.entry}">${icon("minus", "s15")}Tap again to drop it and ${after.join(", ")}</button>`
        : item("drop", "minus", "Drop")}</span>` : ""}</li>`;
  });
  return html`<ol class="bgraph" data-lanes="${lanes}" data-edges="${JSON.stringify(edges)}" aria-label="Entries in start order">${items}</ol>`;
}

// Lines between the rows' nodes: down the predecessor's lane, then into the entry's lane just above it.
const LANE = 18, LANE_PAD = 14, NODE_Y = 19;
function drawLanes(app) {
  for (const graph of app.querySelectorAll(".bgraph")) {
    // Rows reflow without a render, as when the width changes or a font loads.
    if (!graph.dataset.watched) {
      graph.dataset.watched = "1";
      track(new ResizeObserver(() => drawLanes(graph.parentElement))).observe(graph);
    }
    const rows = [...graph.querySelectorAll(":scope > li")];
    const lanes = Number(graph.dataset.lanes);
    const x = (row) => LANE_PAD + Number(row.dataset.lane) * LANE;
    const y = (row) => row.offsetTop + NODE_Y;
    graph.style.setProperty("--gw", `${LANE_PAD * 2 + (lanes - 1) * LANE}px`);
    rows.forEach((row) => row.style.setProperty("--lane", row.dataset.lane));
    const paths = JSON.parse(graph.dataset.edges).map(({ from, to, moment }) => {
      const [x1, y1, x2, y2] = [x(rows[from]), y(rows[from]), x(rows[to]), y(rows[to])];
      const d = x1 === x2 ? `M${x1} ${y1}V${y2}` : `M${x1} ${y1}V${y2 - 40}C${x1} ${y2 - 16} ${x2} ${y2 - 24} ${x2} ${y2}`;
      return `<path class="halo" d="${d}"/>${moment === "waiting" ? `<path class="stack" d="${d}"/><path class="stack-in" d="${d}"/>` : `<path class="base" d="${d}"/>`}`;
    });
    graph.querySelector("svg.lanes")?.remove();
    graph.insertAdjacentHTML("afterbegin", `<svg class="lanes" aria-hidden="true" width="${LANE_PAD * 2 + (lanes - 1) * LANE}" height="${graph.offsetHeight}">${paths.join("")}</svg>`);
  }
}

// The batch's entries by stage, as one bar and as words with counts, and the issues closed at the go.
function stageBar(batch) {
  const parts = stages(batch.counts);
  const closed = (batch.closes || []).filter((close) => close.closed).length;
  return html`<div class="stagebar" aria-hidden="true">${parts.map(([stage, count]) => html`<i class="s-${stage}" data-grow="${count}"></i>`)}</div>
    <p class="stagewords">${parts.map(([stage, count]) => html`<span><b>${count}</b> ${stage}</span>`)}${closed ? html`<span><b>${closed}</b> closed at the go</span>` : ""}</p>`;
}

function closesSection(batch) {
  const closes = batch.closes || [];
  if (!closes.length) return "";
  return html`<section class="sect" id="closes"><div class="sh">${icon("check", "s15")}Closes ${closes.length} ${closes.length > 1 ? "issues" : "issue"}</div><ol class="ilist">${closes.map((close) => html`<li>${icon(close.closed ? "check" : "circle-dashed", "s15")}<span><span class="mono">${close.issue}</span> ${autolinked(close.why, batch.repo)}${close.evidence ? html` ${autolinked(close.evidence, batch.repo)}` : ""}${close.closed ? html`. Closed ${timeTag(close.closed, ago(close.closed))}.` : ""}</span></li>`)}</ol></section>`;
}

function splitsSection(batch) {
  const splits = splitsOf(batch);
  if (!splits.length) return "";
  const parts = (issue) => batch.entries.filter((entry) => entry.issue === issue).map(issueName).join(", ");
  return html`<section class="sect" id="splits"><div class="sh">${icon("layers", "s15")}Splits ${splits.length} ${splits.length > 1 ? "issues" : "issue"}</div><ol class="ilist">${splits.map((split) => html`<li>${icon("layers", "s15")}<span><span class="mono">${split.issue}</span> into ${parts(split.issue)}. ${autolinked(split.why || "", batch.repo)}</span></li>`)}</ol></section>`;
}

// The batch page (DESIGN.md, Batch page): /batches/<id>.
function batchPage(id) {
  const [href, label] = backTo(history.state?.from ?? null);
  const back = html`<div class="top"><a class="back" href="${href}">${icon("chevron-left", "s22")}${label}</a></div>`;
  const batch = batchOf(id);
  if (!batch) {
    const failed = state.batch?.error;
    return html`${back}<h1 class="sr">Batch ${id}</h1><p class="empty">${failed ? html`Could not load batch ${id}: ${failed}. <code>priory batch show ${id}</code> shows it.` : `Loading batch ${id}…`}</p>`;
  }
  return html`${back}<div class="scroll"><div class="ask bhead">
    <p class="ctx">${tile("batch", "sm")}<a class="mono" href="/tasks/${batch.task}" title="task ${batch.task}">${taskLabel(batch.key)}</a><span>${batch.state}</span><span>${batch.lane || ""}</span>${batch.went ? html`<span>went ${timeTag(batch.went, ago(batch.went))} by ${batch.went_by}</span>` : ""}<span>plan rev ${batch.plan_revision}</span></p>
    <h1 class="question">${batch.title || taskLabel(batch.key)}</h1>${stageBar(batch)}</div>
    <section class="sect" id="entries"><div class="sh">${icon("layers", "s15")}${batch.entries.length} entries in start order</div>${laneKey()}${laneGraph(batch, false)}</section>
    ${closesSection(batch)}${splitsSection(batch)}</div>`;
}

// One tap of an entry's menu. A drop that takes successors with it arms first and names them.
async function editPlan(target) {
  const batch = state.batch?.body;
  if (!batch || state.planEdit.at !== planEditAt(batch)) return;
  const edit = state.planEdit;
  if (edit.busy || target.getAttribute("aria-disabled") === "true") return;
  const { entry, edit: action } = target.dataset;
  if (action === "drop" && successors(batch.entries, entry).length && edit.armed !== entry) return (edit.armed = entry), render();
  const body = editBody(batch.plan_revision, batch.entries, entry, action);
  if (!body) return;
  edit.busy = true;
  render();
  const { status, body: reply } = await post(`/api/batches/${batch.id}/edit`, body).catch((error) => ({ status: 0, body: { error: error.message } }));
  const note = status === 200 ? reply.ok : status === 409 ? "The plan changed before this edit. Read the new revision and edit again." : writeRefusal(status, reply);
  state.planEdit = { at: null, batch: batch.id, open: null, armed: null, busy: false, note };
  if (state.batch) state.batch.stale = true;
  render();
  refresh();
}

// Desk actions on a task, each the twin of its CLI verb. Cancel and revive arm on the first tap and
// send on the second, as a command ask does.
const ACTS = { resume: "Resume", revive: "Revive", cancel: "Cancel", release: "Release" };
const DONE_AS = { resume: "Resumed", revive: "Revived", cancel: "Cancelled", release: "Released", say: "Said" };
const ARMS = ["revive", "cancel"];
const ENDED = ["done", "failed", "dropped"];
const actsOf = (taskState) => (taskState === "blocked" ? ["resume", "cancel"] : ["failed", "dropped"].includes(taskState) ? ["revive"] : taskState === "done" ? [] : ["cancel"]);

// The action buttons of a task in `taskState`, or the `acts` given, and its last action's note.
function taskActs(task, taskState, acts = actsOf(taskState)) {
  const busy = state.taskBusy !== null || Boolean(state.failed);
  const buttons = acts.map((act) => {
    const armed = state.taskArmed?.task === task && state.taskArmed.act === act;
    const label = armed ? html`<span><small>Tap again to ${act}</small>${ACTS[act]}</span><span class="drain" aria-hidden="true"></span>` : ACTS[act];
    return html`<button id="act-${task}-${act}" class="btn secondary act ${armed ? "armed" : ""}" data-action="task-act" data-task="${task}" data-act="${act}" aria-disabled="${busy ? "true" : "false"}">${label}</button>`;
  });
  const notice = state.taskNotices.get(task);
  return html`<span class="task-acts">${buttons}</span>${notice ? html`<p class="note acts-note" role="status">${notice}</p>` : ""}`;
}

function disarmTask() {
  if (!state.taskArmed) return;
  clearTimeout(state.taskArmed.timer);
  state.taskArmed = null;
}

// A task's id from the page: its number here, or `runner:key` for another runner's task.
const taskIdOf = (value) => (/^\d+$/.test(value) ? Number(value) : value);
// The body an action carries: another runner's task names the generation the page drew.
const remoteGeneration = (task) => state.data?.tasks.find((each) => each.id == null && remoteId(each) === task)?.generation;
const remoteBody = (task, body) => (typeof task === "string" ? { ...body, generation: remoteGeneration(task) } : body);

function taskAct(target) {
  const task = taskIdOf(target.dataset.task);
  const act = target.dataset.act;
  if (target.getAttribute("aria-disabled") === "true") return;
  const armed = state.taskArmed?.task === task && state.taskArmed.act === act;
  disarmTask();
  if (ARMS.includes(act) && !armed) {
    state.taskArmed = { task, act, generation: remoteGeneration(task), at: Date.now(), timer: setTimeout(() => { state.taskArmed = null; render(); }, ARM_MS) };
    return render();
  }
  // A cancel carries the revision the page showed, as `priory cancel --expected` does.
  taskWrite(task, act, typeof task === "string" ? remoteBody(task, {}) : act === "cancel" ? { revision: state.revision } : {});
}

async function taskWrite(task, act, body) {
  state.taskBusy = task;
  render();
  let reply;
  try {
    reply = await post(`/api/tasks/${task}/${act}`, body);
  } catch {
    reply = { status: 0, body: { error: "the connection dropped" } };
  }
  state.taskBusy = null;
  const { status, body: answered } = reply;
  if (status === 401) return signOut("The daemon refused this token. Paste the current one.");
  // Text typed while the say was in flight stays as the next draft.
  if ([200, 202].includes(status) && act === "say" && (state.sayDrafts.get(task) || "").trim() === body.text) state.sayDrafts.delete(task);
  const remote = typeof task === "string";
  // A sent action shows as the task's newest submission once the list reads it again.
  if (status === 202) state.taskNotices.delete(task);
  else state.taskNotices.set(task, status === 200 ? `${DONE_AS[act]}: ${answered.ok}.`
    : remote && status === 409 ? `Not sent: ${answered.error}. The list reads it again: check the task, then send again.`
    : remote && status === 503 ? `Could not send: ${answered.error}.`
    : status === 409 ? "The state changed since this page loaded. Read the task again, then tap again."
      : status === 503 ? "Could not send: no priory daemon runs beside this server. Start priory daemon, then send again."
        : status === 504 || status === 0 ? "The daemon may have applied this but did not confirm it. Check the task's state before you send again."
          : `Could not ${act}: ${answered.error || `the server answered ${status}`}.`);
  // A refused action changes no listing, so the refresh may not paint: the note shows now.
  render();
  refresh();
}

// A task's page: its state, reason, actions, a box to say something to its agent, then its brief,
// plan, pull request and trace.
function taskPage(id) {
  const [href, label] = backTo(history.state?.from ?? null);
  const back = html`<div class="top"><a class="back" href="${href}">${icon("chevron-left", "s22")}${label}</a></div>`;
  const task = taskOf({ task: id });
  if (!task) {
    const failed = state.task?.error;
    return html`${back}<h1 class="sr">Task ${id}</h1><p class="empty">${failed ? `Could not load task ${id}: ${failed}.` : `Loading task ${id}…`}</p>`;
  }
  const said = state.taskBusy === task.id ? raw('aria-disabled="true"') : "";
  const say = ENDED.includes(task.state) ? "" : html`<form class="writeform" data-action="say" data-task="${task.id}"><label class="sr" for="say">Say to the agent</label>
    <textarea id="say" name="text" rows="2" placeholder="Say to the agent">${state.sayDrafts.get(task.id) || ""}</textarea><button class="btn primary" ${said}>Say</button></form>`;
  const sections = taskSections(task).map(([, , section]) => section);
  return html`${back}<div class="scroll"><div class="ask task">
    <p class="ctx">${tile(task.state, "sm")}<span class="mono">${taskLabel(task.key)}</span><span>${task.state}</span><span>${task.workflow} ${task.step}</span><span>${task.lane || ""}</span><span>${task.band || ""}</span>${task.batch == null ? "" : html`<a href="/batches/${task.batch}">batch ${task.batch}</a>`}</p>
    <h1 class="question">${autolinked(task.tldr || task.brief?.title || task.brief?.request?.split("\n")[0] || taskLabel(task.key), task.repo)}</h1>
    ${task.reason ? html`<p class="text t-${role(task.state)}">${autolinked(task.reason, task.repo)}</p>` : ""}
    ${taskAsks(task)}${taskActs(task.id, task.state)}${say}</div>${sections}${terminalSection(task)}</div>`;
}

// The task's open asks, each with a way to answer it (DESIGN.md, Task detail).
function taskAsks(task) {
  const asks = openAsksOf(state.data.asks, task.id);
  if (!asks.length) return "";
  return html`<ul class="taskasks" aria-label="Open asks">${asks.map((ask) => html`<li>${tile("ask", "sm")}<span>${ask.title || ask.question}<span class="age">${timeTag(ask.since)}</span></span><a class="btn primary" href="/asks/${encodeURIComponent(ask.id)}">Answer…</a></li>`)}</ul>`;
}

// Brief, Plan, PR and Trace, each left out when the task lacks it. Agent text shows with its autolinks.
function taskSections(task) {
  if (!task) return [];
  const sections = [];
  const sect = (key, ico, title, meta, body) => html`<section class="sect" id="${key}"><div class="sh">${icon(ico, "s15")}${title}<span class="meta">${meta}</span></div>${body}</section>`;
  const { request, result } = task.brief || {};
  if (request || result) {
    sections.push(["brief", "Brief", sect("brief", "file-text", "Brief", result ? `${result.step}, run ${result.run}` : "",
      html`${request ? html`<p class="meta">The request</p>${md(request, task.repo)}` : ""}${result ? md(result.summary, task.repo) : ""}`)]);
  }
  const progress = progressView(task.progress);
  if (progress) {
    const line = (label, text) => (text ? html`<p class="meta">${label}</p><p class="text">${autolinked(text, task.repo)}</p>` : "");
    const criteria = progress.criteria.length ? html`<p class="meta">Acceptance criteria</p><ol class="hist crit">${progress.criteria.map((criterion) => html`<li><span class="mono">${criterion.id}</span><span>${autolinked(criterion.description, task.repo)}</span><span class="tag">${criterion.state}</span></li>`)}</ol>` : "";
    sections.push(["progress", "Progress", sect("progress", "check", "Progress", "", html`${line("Goal", progress.goal)}${criteria}${line("Latest change", progress.latestChange)}${line("Next", progress.nextAction)}`)]);
  }
  if (task.plan) sections.push(["plan", "Plan", sect("plan", "check", "Plan", `${task.plan.step}, run ${task.plan.run}`, md(task.plan.summary, task.repo))]);
  const pr = task.pull_request;
  if (pr) {
    sections.push(["pr", "PR", sect("pr", "workflow", `#${pr.number} ${pr.title || ""}`, stateWords(pr.review),
      html`<p class="meta">${pr.branch} into ${pr.base}, head <span class="mono">${String(pr.head_sha).slice(0, 7)}</span>, ${pr.state}</p>
      <ol class="hist">${pr.checks.map((check) => html`<li>${tile(checkState(check.conclusion), "sm")}<span>${check.name}</span><span>${check.conclusion ? stateWords(check.conclusion) : "running"}</span></li>`)}</ol>${prStackList(task.stack, pr)}${triageList(task.triage)}`)]);
  } else if (prs(task)) {
    // The summary needs the pull request's events; the stack does not.
    sections.push(["pr", "PR", sect("pr", "workflow", prs(task), "", html`${prStackList(task.stack, null)}${triageList(task.triage)}`)]);
  } else if (task.triage?.length) {
    sections.push(["pr", "Feedback", sect("pr", "workflow", "Feedback", "", triageList(task.triage))]);
  }
  if (task.inbox?.length) {
    const shown = task.inbox.slice(0, INBOX_SHOWN);
    const more = task.inbox.length > INBOX_SHOWN ? html`<p class="meta">Latest ${INBOX_SHOWN} of ${task.inbox.length}. <code>priory task ${task.id}</code> lists every event.</p>` : "";
    sections.push(["inbox", "Inbox", sect("inbox", "message-text", "Inbox", "newest first", html`<ol class="hist" aria-label="Events routed to this task, newest first">${shown.map((event) =>
      html`<li>${tile(INBOX_TILES[event.prompt] || (event.outcome === "kept" && event.task === task.id ? "asleep" : "ready"), "sm")}<span><b title="${event.type}">${eventWords(event.type)}</b> ${event.subject || ""}<span class="meta">${handled(event, task.id, taskName)}</span></span>${timeTag(event.time)}</li>`)}</ol>${more}`)]);
  }
  if (task.runs?.length) {
    sections.push(["trace", "Trace", sect("trace", "runs", "Trace", "", html`<ol class="hist">${task.runs.map((run) => {
      const tokens = tokenText(run.tokens);
      const minutes = Math.round(((run.ended ? Date.parse(run.ended) : Date.now()) - Date.parse(run.started)) / 60000);
      const scores = (task.scores || []).filter((score) => score.run === String(run.id));
      const scored = scores.length ? html`<ol class="scores" aria-label="Scores of run ${run.id}">${scores.map((score) => html`<li>${tile(score.verdict, "sm")}<span><span class="mono">${score.scorer}</span> <span class="v t-${role(score.verdict)}">${score.verdict}</span><span class="meta">${score.reason}</span></span></li>`)}</ol>` : "";
      return html`<li>${tile(run.ended ? run.end || "moved" : "running", "sm")}<span><b>${run.step}</b>, run <span class="mono">${run.id}</span>, ${minutes} min${run.outcome ? `, ${run.outcome}` : ""}${tokens ? `, ${tokens} tokens` : ""}${scored}</span>${timeTag(run.started)}</li>`;
    })}</ol>`)]);
  }
  return sections;
}

// The run's terminal: its screen polled while shown (terminal.js), never kept on this device.
const watch = screenWatch({
  read: (run, have, history) => api(`/api/runs/${run}/screen?${new URLSearchParams({ history, ...(have ? { have } : {}) })}`).catch((error) => {
    if (error.status === 401) signOut("The daemon refused this token. Paste the current one.");
    throw error;
  }),
  changed: () => paintTerminal(document.getElementById("app")),
});
// `left` is the pane's sideways scroll for run `run`, kept while a refresh redraws the task page.
// `typing` opens the Say line, `line` holds its text, `said` its last note and whether it was said.
// `menu` opens the header's More, and `keys` holds the text the keys bar pastes.
const terminal = { fit: false, copied: null, run: null, left: 0, onScreen: false, typing: false, line: "", said: null, saying: false, menu: false, keys: "" };
// This page's hold on the pane's keyboard (terminal.js). Its writes keep going while the page unloads.
const keyboard = keyboardSession({
  post: (path, body) => fetch(path, {
    method: "POST",
    keepalive: true,
    headers: { Authorization: `Bearer ${state.token}`, "Content-Type": "application/json" },
    body: JSON.stringify(body),
  }).then(async (response) => ({ status: response.status, body: await response.json().catch(() => ({})) })),
  signOut: () => signOut("The daemon refused this token. Paste the current one."),
  changed: () => paintTerminal(document.getElementById("app"), true),
  format: (iso) => clock(iso),
});
const screenText = (screen) => screenRows(screen.lines).map((parts) => parts.map((part) => part.text).join("")).join("\n").trimEnd();

// The live run, else the run whose last screen this page still holds.
function terminalRun(task) {
  const runs = task.runs || [];
  const kept = watch.state();
  return runs.findLast((run) => !run.ended) || runs.find((run) => run.id === kept.run && kept.screen) || null;
}

// An empty box that paintTerminal fills, so a new screen repaints only the pane.
function terminalSection(task) {
  const run = terminalRun(task);
  if (!run) return "";
  const end = run.ended ? run.end || "moved" : "";
  return html`<section class="sect" id="terminal"><div class="sh">${icon("terminal", "s15")}Terminal<span class="meta">${stateWord(end || "running")}</span></div>
    <div class="termbox" data-term="${run.id}" data-task="${task.id}" data-tmux="${run.tmux}" data-end="${end}"></div></section>`;
}

function terminalBody({ term, task, tmux, end }) {
  const run = Number(term);
  const kept = watch.state();
  const { screen, at, stale, gone } = kept.run === run ? kept : {};
  const notes = [];
  if (screen?.redacted) notes.push(["warn", "shield", `${screen.redacted} ${screen.redacted === 1 ? "secret" : "secrets"} hidden on this screen`]);
  if (stale && !end && !gone) notes.push(["warn", "unplug", screen ? `Reconnecting. Screen from ${clock(new Date(at).toISOString())}, ${Math.round((Date.now() - at) / 1000)} s ago.` : "Reconnecting. The screen could not be read yet."]);
  if (end) notes.push(["", (STATES[end] || [, "circle-help"])[1], screen ? `Run ${run} ended as ${end}. This is the last screen this page saw.` : `Run ${run} ended as ${end}.`]);
  else if (gone) notes.push(["warn", "unplug", `This server reads no screen of run ${run}. Run priory screen ${task} on the runner to see why.`]);
  else if (!screen && !stale) notes.push(["", "clock", "Reading the screen…"]);
  const keys = keyboard.state();
  const held = end ? null : keys.held;
  const offer = !end && !held && Boolean(screen?.keyboard);
  if (keys.ended && !held) notes.push(["", "clock", keys.ended, offer]);
  const earlier = screen?.earlier || 0;
  const top = end || !screen ? "" : kept.more ? html`<button class="term-more" data-action="term-earlier">${icon("chevron-up", "s15")}Load earlier${earlier ? html` <span class="dim">(${earlier} shown)</span>` : ""}</button>`
    : earlier >= EARLIER_CAP ? html`<p class="term-edge">The last ${EARLIER_CAP.toLocaleString("en")} lines are as far back as this pane reads.</p>` : "";
  const typing = terminal.typing && !end;
  const copied = (what, text) => (terminal.copied === what ? "Copied" : text);
  const tag = held ? html`<span class="ro keys">keys</span>` : html`<span class="ro${typing ? " on" : ""}">${typing ? "say" : "read-only"}</span>`;
  const menu = terminal.menu ? html`<div class="term-menu" id="term-more">
      ${offer ? html`<button data-action="term-take-ask">${icon("keyboard", "s15")}<span>Take the keyboard<small>Send keys straight into the pane</small></span></button>` : ""}
      ${held ? html`<button data-action="term-give-ask">${icon("keyboard", "s15")}<span>Give the keyboard back</span></button>` : ""}
      <button data-action="term-attach" data-task="${task}">${icon("eye", "s15")}<span>${copied("attach", "Copy attach command")}</span></button></div>` : "";
  return html`${notes.map(([kind, name, text, again]) => html`<p class="note ${kind}" role="status">${icon(name, "s15")}<span>${text}${again ? html` <button class="linkish" data-action="term-take-ask">Take it again</button>` : ""}</span></p>`)}
    <div class="term${terminal.fit ? " fit" : ""}${end ? " ended" : ""}${held ? " held" : ""}" role="region" aria-label="Terminal of run ${run}">
      <div class="term-head">${icon("terminal", "s15")}<span class="mono">${tmux}</span><span class="rid">run ${run}</span>${end ? tile(end, "sm") : ""}${tag}<span class="sp"></span>
        ${end ? "" : html`<button data-action="term-type" aria-label="Type" aria-pressed="${String(typing)}">${icon("message-text", "s15")}<span>Type</span></button>`}
        <button data-action="term-copy" aria-label="Copy screen" ${screen ? "" : raw('aria-disabled="true"')}>${icon("copy", "s15")}<span>${copied("screen", "Copy screen")}</span></button>
        <button data-action="term-fit" aria-label="Fit" aria-pressed="${String(terminal.fit)}">${icon("fit", "s15")}<span>Fit</span></button>
        <button data-action="term-menu" aria-label="More" aria-controls="term-more" aria-expanded="${String(terminal.menu)}">${icon("more", "s15")}</button></div>
      ${menu}${top}<pre id="term-screen" tabindex="0" aria-label="Screen${screen ? `, ${screen.width} by ${screen.height}` : ""}"></pre>
      ${typing ? html`<div class="term-type"><form data-action="term-say" data-task="${task}"><span class="pr" aria-hidden="true">›</span><label class="sr" for="term-line">Say to the agent</label>
        <input id="term-line" autocomplete="off" placeholder="Say to the agent"><button class="send" ${terminal.line.trim() && !terminal.saying ? "" : raw('aria-disabled="true"')}>Say</button></form>
        ${terminal.said ? html`<p class="tn ${terminal.said.said ? "ok" : "no"}" role="status">${icon(terminal.said.said ? "check" : "circle-x", "s15")}<span>${terminal.said.note}</span></p>` : ""}</div>` : ""}
      ${held ? keysBar(held, keys) : ""}</div>`;
}

// The markup of each row of the screen. The cursor shows only on a live run's screen with no earlier rows above it.
function terminalRows(screen, end) {
  if (!screen) return [];
  // Each part's classes come from screenRows' fixed set; its text stays text.
  const inner = (part) => (part.hidden ? html`<span class="hid">${part.text}</span>` : part.cls ? html`<span class="${part.cls}">${part.text}</span>` : part.text);
  const part = (part) => (part.cursor ? html`<span class="cur">${inner(part)}</span>` : inner(part));
  const earlier = screen.earlier || 0;
  return screenRows(screen.lines, earlier || end ? null : screen.cursor).map((parts, at) => html`${at < earlier ? html`<span class="earlier">${parts.map(part)}</span>` : parts.map(part)}\n${at === earlier - 1 ? raw('<span class="fold" aria-hidden="true"></span>') : ""}`.text);
}

// The bar of a held keyboard: whose and until when, a text pasted without Enter, and one button a key.
function keysBar(held, { note, returning, pasting }) {
  const row = KEYS.map((key) => (key ? html`<button data-action="term-key" data-key="${key[0]}" aria-label="${key[0]}"${key[2] ? raw(' class="hot"') : ""}>${key[1]}</button>` : raw('<span class="gap" aria-hidden="true"></span>')));
  return html`<div class="keys-bar"><p class="who">${icon("keyboard", "s15")}<span><b>Your keys</b> until ${clock(held.ends)}</span><span class="sp"></span><button class="give" data-action="term-give-ask" ${returning ? raw('aria-disabled="true"') : ""}>${returning ? "Giving back…" : "Give back"}</button></p>
    <form data-action="term-paste"><span class="pr" aria-hidden="true">›</span><label class="sr" for="term-keys">Text to paste, without Enter</label>
      <input id="term-keys" autocomplete="off" autocapitalize="off" spellcheck="false" placeholder="Text, sent without Enter"><button class="send" ${pasting || returning ? raw('aria-disabled="true"') : ""}>Send</button></form>
    <div class="keys-row" role="group" aria-label="Keys">${row}</div>
    ${note ? html`<p class="tn" role="status">${icon("check", "s15")}<span>${note}</span></p>` : ""}</div>`;
}

const keyboardSheet = () => document.getElementById("keyboard");

// The confirm sheet before a person takes the keyboard, or gives it back (DESIGN.md, Terminal pane).
function openKeyboardSheet(kind) {
  const sheet = keyboardSheet();
  const run = keyboard.state().run;
  // The sheet acts on the run it names, never on one the pane switched to under it.
  sheet.dataset.run = run;
  const tail = (watch.state().screen?.lines || []).map((line) => line.replace(/\x1b\[[0-9;:]*m/g, "")).filter((line) => line.trim()).slice(-2).join("\n");
  sheet.innerHTML = (kind === "take"
    ? html`<div class="shead"><h2 id="keyboard-title">Take the keyboard of run ${run}?</h2><button class="iconbtn" data-action="keyboard-close" aria-label="Close">${icon("x", "s20")}</button></div>
      <p class="warnline">${icon("shield", "s15")}<span>Your keys reach the agent and its shell directly. Priory's guard does not check them.</span></p>
      <ul class="facts"><li>${icon("clock", "s15")}<span>For 15 minutes, or until you give it back.</span></li>
        <li>${icon("pause", "s15")}<span>The agent's queued prompts and your Says wait until then.</span></li>
        <li>${icon("pencil", "s15")}<span>Priory records each key and the length of each text, never the text.</span></li></ul>
      <div class="acts"><button class="btn secondary" data-action="keyboard-close">Keep watching</button><button class="btn danger" data-action="term-take">${icon("keyboard", "s15")}Take the keyboard</button></div>`
    : html`<div class="shead"><h2 id="keyboard-title">Give the keyboard back?</h2><button class="iconbtn" data-action="keyboard-close" aria-label="Close">${icon("x", "s20")}</button></div>
      <p class="line">Anything you typed and did not submit stays in the agent's input box, and its next prompt joins it. Clear it with ⌃C or submit it with ⏎ first.</p>
      ${tail ? html`<pre class="peek">${tail}</pre>` : ""}
      <div class="acts"><button class="btn secondary" data-action="keyboard-close">Keep typing</button><button class="btn primary" data-action="term-give">Give back</button></div>`).text;
  if (!sheet.open) sheet.showModal();
  sheet.querySelector(".acts .btn:last-child")?.focus();
}

async function keyboardAct(action) {
  const sheet = keyboardSheet();
  const run = keyboard.state().run;
  sheet.close();
  if (String(run) !== sheet.dataset.run) return;
  if (action === "term-take") {
    terminal.typing = false;
    await keyboard.take(run);
    return document.getElementById("term-keys")?.focus();
  }
  await keyboard.giveBack();
  watch.revision();
}

// Sends one input; a text leaves the field once it was sent, unless the person typed on.
async function sendInput(input) {
  const sent = await keyboard.send(input);
  if (sent && input.text !== undefined && terminal.keys === input.text) terminal.keys = "";
  if (sent) watch.revision();
  paintTerminal(document.getElementById("app"), true);
}

// Repaints the pane when what it shows changed, or always after a render, keeping its sideways
// scroll and focus. Fit sizes the text to the pane, between 9 px and the pane's own size.
function paintTerminal(app, rendered = false) {
  const box = app?.querySelector("[data-term]");
  // A page without the pane gives back a keyboard it held.
  if (!box) return app && keyboard.leave();
  const kept = watch.state();
  // The watch may still hold the run shown before, until it polls the one in this box.
  const screen = kept.run === Number(box.dataset.term) ? kept.screen : null;
  // A grant this page holds on a run it no longer shows goes back, so that run's outbox resumes.
  keyboard.at(box.dataset.end ? null : Number(box.dataset.term), `${box.dataset.task}/${box.dataset.tmux}`);
  if (terminal.run !== box.dataset.term) Object.assign(terminal, { run: box.dataset.term, left: 0, typing: false, line: "", said: null, saying: false, menu: false, keys: "" });
  const frame = terminalBody(box.dataset).text;
  const key = JSON.stringify([frame, innerWidth]);
  // A new screen in the same frame rewrites only its changed rows, so a selection elsewhere stays.
  if (box.dataset.painted === key && !rendered) return paintRows(box.querySelector("pre"), terminalRows(screen, box.dataset.end));
  const active = document.activeElement;
  const focused = !box.contains(active) ? null : active.dataset.key ? `[data-key="${CSS.escape(active.dataset.key)}"]` : active.id ? `#${active.id}` : active.dataset.action ? `[data-action="${active.dataset.action}"]` : "pre";
  const caret = ["term-line", "term-keys"].includes(active?.id) ? [active.id, active.selectionStart, active.selectionEnd] : null;
  box.innerHTML = frame;
  box.dataset.painted = key;
  const pre = box.querySelector("pre");
  paintRows(pre, terminalRows(screen, box.dataset.end));
  const own = parseFloat(getComputedStyle(pre).fontSize);
  const fit = terminal.fit && screen;
  pre.style.fontSize = fit ? `${Math.min(own, Math.max(9, (pre.clientWidth - 24) / (screen.width * 0.6)))}px` : "";
  pre.style.lineHeight = fit ? "1.33" : "";
  pre.scrollLeft = terminal.left;
  pre.addEventListener("scroll", () => { terminal.left = pre.scrollLeft; });
  if (focused) (focused === "pre" ? pre : box.querySelector(focused))?.focus({ preventScroll: true });
  // Typed text stays out of the frame, so typing alone repaints nothing.
  for (const [id, text] of [["term-line", terminal.line], ["term-keys", terminal.keys]]) {
    const input = box.querySelector(`#${id}`);
    if (input) input.value = text;
  }
  if (caret) box.querySelector(`#${caret[0]}`)?.setSelectionRange(caret[1], caret[2]);
}

// Polls while a live run's pane is on screen and the page is shown.
function syncTerminal(app) {
  const box = app.querySelector("[data-term]");
  if (!box || box.dataset.end || document.hidden || !state.token || !terminal.onScreen) return watch.hide();
  watch.show(Number(box.dataset.term));
}

// Tells syncTerminal whether the pane is on screen; render disconnects the observer.
function observeTerminal(app) {
  const box = app.querySelector("[data-term]");
  terminal.onScreen = false;
  if (!box) return watch.hide();
  track(new IntersectionObserver(([entry]) => {
    terminal.onScreen = entry.isIntersecting;
    syncTerminal(app);
  })).observe(box);
}

function copyTerminal(text, what) {
  navigator.clipboard?.writeText(text).then(() => {
    terminal.copied = what;
    paintTerminal(document.getElementById("app"));
    setTimeout(() => {
      terminal.copied = null;
      paintTerminal(document.getElementById("app"));
    }, 2000);
  }, () => {});
}

// The Terminal panel: each running task, whose page shows its terminal.
function terminalsPanel() {
  const running = (state.data?.tasks || []).filter((task) => task.state === "running");
  if (!running.length) return html`<p class="empty">No agent runs now. A running task's terminal shows here.</p>`;
  return html`<ol class="hist" aria-label="Running tasks">${running.map((task) => html`<li>${tile("running", "sm")}<a href="/tasks/${task.id}#terminal" title="task ${task.id}"><span class="mono">${taskLabel(task.key)}</span> ${task.title || ""}</a></li>`)}</ol>`;
}

const INBOX_SHOWN = 20;
// A prompt's state as a tile: delivered, waiting to be typed, being typed, or withdrawn.
const INBOX_TILES = { confirmed: "done", queued: "queued", attempted: "running", uncertain: "prompt", withdrawn: "dropped" };

const TRIAGE_TILES = { trivial: "ready", "nothing-to-do": "dropped", "persons-call": "ask" };

// How the agent sorted each review feedback item: trivial, nothing-to-do or persons-call.
function triageList(triage) {
  if (!triage?.length) return "";
  const label = (item) => item.replace(/^https:\/\/github\.com\//, "");
  const named = (item) => (item.startsWith("https://") ? html`<a class="autolink" href="${item}" rel="noopener noreferrer" target="_blank">${label(item)}</a>` : item);
  return html`<p class="meta">Feedback, as the agent sorted it</p><ol class="hist" aria-label="Review feedback items">${triage.map((entry) => html`<li>${tile(TRIAGE_TILES[entry.class] || "queued", "sm")}<span>${named(entry.item)}</span><span>${triageWords(entry.class)}</span></li>`)}</ol>`;
}

const PULL_TILES = { merged: "done", closed: "dropped" };

function prStackList(stack, latest) {
  const { label, entries } = prStack(stack, latest);
  if (!label) return "";
  return html`<p class="meta">Stack, bottom first: ${label}</p><ol class="hist" aria-label="Pull requests, bottom first">${entries.map((pull) => html`<li>
    ${tile(PULL_TILES[pull.state] || (pull.next ? "ready" : "queued"), "sm")}
    <span><a class="autolink mono" href="${pull.url}" rel="noopener noreferrer" target="_blank">#${pull.number}</a> ${pull.branch} into ${pull.base}${pull.checks ? `, ${pull.checks}` : ""}</span>
    <span class="${pull.next ? "t-live" : ""}">${pull.next ? `${pull.state}, next to merge` : pull.state}</span></li>`)}</ol>`;
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
  state.armed = { id: ask.id, revision: ask.revision, text: option, at: Date.now(), timer: setTimeout(() => { state.armed = null; render(); }, ARM_MS) };
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
  if (status === 200 || status === 202) {
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
  if (status === 409 && fresh) state.data.asks = state.data.asks.map((candidate) => (candidate.id === fresh.id ? fresh : candidate));
  state.notices.set(ask.id, writeNote(status, answered, age, ask));
  if (status === 504 || status === 0) refresh();
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
    if (stillArmed(state.armed, ask, option, Date.now())) return runCommand(ask, option);
    return arm(ask, option);
  }
  const action = target.dataset.action;
  if (action === "undo") return undo();
  if (state.pending) return;
  if (action === "options") return openSheet();
  if (action === "write") {
    state.writing.add(ask.id);
    render();
    return document.getElementById("draft")?.focus();
  }
  if (action === "dismiss") return dismiss(ask);
  if (action === "snooze" || action === "unsnooze") return snooze(ask, target);
}

// Costs (DESIGN.md, Costs panel): what the factory spent this week against last week, and what it
// cost the person, from /api/costs and /api/attention.
async function loadCosts() {
  const view = state.costs;
  // A read scheduled before a sign-out, or before the connection dropped, does not go.
  if (view.loading || !state.token || !state.data || state.failed) return;
  const by = view.by;
  view.loading = true;
  const slow = setTimeout(() => { view.slow = true; render(); }, 300);
  try {
    const [costs, attention] = await Promise.all([api(`/api/costs?by=${by}`), api("/api/attention")]);
    // A sign-out while the read was in flight replaced the Costs state and drops this read.
    if (view !== state.costs) return;
    state.data.spend = { costs, attention, updated: Date.now() };
    view.error = null;
    keep();
  } catch (error) {
    if (view !== state.costs) return;
    if (error.status === 401) return signOut("The daemon refused this token. Paste the current one.");
    view.error = error instanceof TypeError ? "the connection dropped" : error.message;
  } finally {
    clearTimeout(slow);
    Object.assign(view, { loading: false, slow: false });
  }
  if (view !== state.costs) return;
  render();
  // The person picked another split while this read was in flight.
  if (!view.error && state.data?.spend?.costs.by !== view.by) loadCosts();
}

function costsPage(compact) {
  const spend = state.data.spend;
  const wide = layout() === "wide";
  const shown = new URLSearchParams(location.search).get("view") === "attention" ? "attention" : "costs";
  const head = compact ? html`<div class="top"><h1>Costs</h1><span class="sp"></span>${connection()}</div>` : html`<div class="lhead"><h1>Costs</h1></div>`;
  const chips = wide ? "" : savedViews([["costs", "Costs", null, shown === "costs"], ["attention", "Attention", null, shown === "attention"]], "/costs");
  const failed = state.costs.error
    ? html`<p class="note warn" role="alert">${icon("circle-x", "s15")}<span>Could not load costs: ${state.costs.error}. Check that the daemon runs on its runner, then retry.</span><button class="ghost" data-action="costs-retry">Retry</button></p>`
    : "";
  const asOf = spend && (state.failed || state.costs.error || Date.now() - spend.updated > 60000) ? html`<p class="note asof"><span>Costs as read ${timeTag(new Date(spend.updated).toISOString(), date(new Date(spend.updated).toISOString()))}.</span></p>` : "";
  let body;
  if (spend) body = wide ? html`<div class="spend"><div>${costsPart(spend.costs)}</div><div>${attentionPart(spend.attention)}</div></div>` : shown === "costs" ? costsPart(spend.costs) : attentionPart(spend.attention);
  else body = state.costs.slow ? html`<div class="sk" aria-hidden="true"><span></span><span></span><span></span></div>` : "";
  return html`${head}<div class="panel" role="region" aria-label="Costs" data-costs>${chips}<div class="scroll">${failed}${asOf}${body}</div></div>`;
}

const TRENDS = { up: "trending-up", down: "trending-down" };
const deltaLine = (delta) => (delta ? html`<span class="delta">${TRENDS[delta.direction] ? icon(TRENDS[delta.direction], "s13") : ""}${delta.text}</span>` : html`<span class="delta">No change to show</span>`);
const statTile = (tile) => html`<div class="stat"><span class="lbl">${tile.label}</span><span class="val num">${tile.value}</span>${"delta" in tile ? deltaLine(tile.delta) : html`<span class="delta">${tile.note}</span>`}</div>`;

function costsPart(costs) {
  const table = splitTable(costs);
  const offline = Boolean(state.failed);
  const pick = html`<label class="by"><span>Split by</span><select data-action="costs-by" aria-disabled="${offline}" ${offline ? raw("disabled") : ""}>${SPLITS.map((by) => html`<option value="${by}" ${by === costs.by ? raw("selected") : ""}>${by}</option>`)}</select></label>`;
  if (!costs.days.length && !costs.split.length) {
    // Scores can land in a window with no cost, so they keep their table and its Split by.
    const scores = scoresPart(costs, pick);
    return html`<p class="empty">No cost in the last two weeks. Costs appear once a model step records its tokens. Price each model in <code>prices.toml</code> in the factory folder.</p>${scores ? html`<section class="sect" aria-labelledby="costs-scores">${scores}</section>` : ""}`;
  }
  const unpriced = table.unpriced.length || table.unknown
    ? html`<div class="note"><p><b>Not in the cost.</b> ${table.unpriced.length ? html`Tokens with no price: ${table.unpriced.map((entry, i) => html`${i ? ", " : ""}<span class="mono">${entry.model}</span> ${entry.tokens} tokens`)}. Add each model to <code>prices.toml</code> to price them.` : ""} ${table.unknown ? `${table.unknown}, so their cost is unknown.` : ""}</p></div>`
    : "";
  return html`<section class="sect" aria-labelledby="costs-week"><h2 class="sh" id="costs-week">This week<span class="meta">against last week</span></h2>
    <div class="stats">${costTiles(costs).map(statTile)}</div>${unpriced}</section>
    <section class="sect" aria-labelledby="costs-days"><h2 class="sh" id="costs-days">Each day</h2>${dayChart(costs)}</section>
    <section class="sect" aria-labelledby="costs-split"><h2 class="sh" id="costs-split">This week by ${costs.by}${pick}</h2>
      <div class="tscroll"><table class="ctbl"><thead><tr><th scope="col">${table.dimension}</th><th scope="col" class="r">Cost</th><th scope="col" class="r">Tokens</th><th scope="col">Notes</th></tr></thead>
      <tbody>${table.rows.map((row) => html`<tr><th scope="row" class="mono">${row.name}</th><td class="r">${row.cost}</td><td class="r">${row.tokens}</td><td class="n">${row.note}</td></tr>`)}</tbody></table></div>${scoresPart(costs)}</section>`;
}

function scoresPart(costs, pick = "") {
  const table = scoreTable(costs);
  if (!table.rows.length) return "";
  const head = ["Pass rate", "pass", "fail", "error", "not applicable"].map((label) => html`<th scope="col" class="r">${label}</th>`);
  return html`<h3 class="sh" id="costs-scores">Scores by ${costs.by}${pick}</h3><p class="cap">The pass rate leaves out errors and not applicable.</p>
    <div class="tscroll"><table class="ctbl stbl" aria-labelledby="costs-scores"><thead><tr><th scope="col">${table.dimension}</th><th scope="col">Scorer</th>${head}</tr></thead>
    <tbody>${table.rows.map((row) => html`<tr><th scope="row" class="mono">${row.name}</th><td class="mono">${row.scorer}</td><td class="r c rate">${row.rate}</td><td class="r c">${row.pass}</td><td class="r c">${row.fail}</td><td class="r c">${row.error}</td><td class="r c">${row.notApplicable}</td><td class="line">${row.line}</td></tr>`)}</tbody></table></div>`;
}

// Bars in percent of the box, so the chart takes any width without stretching its text.
function dayChart(costs) {
  const bars = dayBars(costs);
  const top = Math.max(0, ...costs.days.map((day) => day.cost_usd ?? 0));
  const slot = 100 / bars.length;
  const height = 96;
  const split = bars.findIndex((bar) => bar.current);
  const week = split > 0 ? html`<line class="week" x1="${split * slot}%" x2="${split * slot}%" y1="0" y2="${height + 52}"/><text class="wk" x="${split * slot}%" dx="4" y="10">this week</text>` : "";
  const columns = bars.map((bar, i) => {
    const h = Math.max(bar.fraction * height, 1);
    const x = i * slot;
    return html`<g class="${bar.current ? "cur" : "prev"}"><title>${bar.text}</title>
      <rect x="${x + slot * 0.18}%" y="${16 + height - h}" width="${slot * 0.64}%" height="${h}" rx="2"/>
      <text class="d" x="${x + slot / 2}%" y="${height + 32}" text-anchor="middle"><tspan class="wd">${bar.weekday} </tspan>${bar.date}</text>
      <text class="m" x="${x + slot / 2}%" y="${height + 48}" text-anchor="middle">${bar.merges}</text></g>`;
  });
  return html`<figure class="days"><svg width="100%" height="${height + 56}" role="img" aria-label="Cost per day, last week then this week. ${bars.map((bar) => bar.text).join(". ")}.">
    <text class="axis" x="0" y="10">${top ? money(top) : ""}</text>
    <line class="base" x1="0" x2="100%" y1="${16 + height}" y2="${16 + height}"/>${columns}${week}</svg>
    <figcaption>Bars show each day's cost${split > 0 ? ", and this week starts at the line" : ""}. The figure under each day counts its merges.</figcaption></figure>`;
}

function attentionPart(report) {
  const view = attentionView(report);
  if (!report.acts && !report.tasks.length && !report.waits.acted && !report.waits.open) {
    return html`<p class="empty">No attention counted this week. Your answers, dismissals and snoozes on asks count here, and each task's time once it runs.</p>`;
  }
  const { headline } = view;
  return html`<section class="sect" aria-labelledby="attention-head"><h2 class="sh" id="attention-head">Your attention<span class="meta">this week</span></h2>
    <div class="headline"><span class="val num">${headline.value}</span><span class="w">${headline.label}<span>${headline.note}</span></span></div>
    <div class="stats">${view.waits.map(statTile)}</div></section>
    <section class="sect" aria-labelledby="attention-tasks"><h2 class="sh" id="attention-tasks">Where each task's time went</h2>
      <ul class="legend">${PART_KEYS.map(([part, label]) => html`<li><svg width="18" height="10" aria-hidden="true"><rect class="p-${part}" x="0.5" y="0.5" width="17" height="9" rx="2"/></svg>${label}</li>`)}</ul>
      <ol class="tsplit">${view.tasks.map((task) => html`<li><p><a class="mono" href="/tasks/${task.id}" title="task ${task.id}">${taskLabel(task.key)}</a><span>${task.workflow}</span><span class="sp"></span><span class="num">${task.wall}</span></p>${taskBar(task)}</li>`)}</ol></section>
    <svg width="0" height="0" class="defs" aria-hidden="true"><defs><pattern id="stripe-wait" width="6" height="6" patternUnits="userSpaceOnUse" patternTransform="rotate(45)"><rect class="stripe" width="3" height="6"/></pattern></defs></svg>`;
}

const PART_KEYS = [["working", "Working"], ["person", "Waiting on you"], ["world", "Waiting on the world"], ["queued", "Queued"]];

function taskBar(task) {
  let at = 0;
  const parts = task.parts.filter((part) => part.fraction > 0).map((part) => {
    const x = at * 100;
    at += part.fraction;
    return html`<rect class="p-${part.part}" x="${x}%" y="0.5" width="${part.fraction * 100}%" height="11"><title>${part.label}: ${part.value}</title></rect>`;
  });
  // A 2 px line of the page ground between parts keeps them apart in every encoding.
  let edge = 0;
  const gaps = task.parts.filter((part) => part.fraction > 0).slice(0, -1).map((part) => {
    edge += part.fraction;
    return html`<line class="gap" x1="${edge * 100}%" x2="${edge * 100}%" y1="0" y2="12"/>`;
  });
  return html`<svg class="tbar" width="100%" height="12" role="img" aria-label="${task.text}">${parts}${gaps}</svg>`;
}

// Reports (DESIGN.md, Reports panel): the report written last, its sections in its own order, picks
// with an Undo, and threads on rows through the report endpoints.
const reportRows = (model) => model.sections.flatMap((section) => section.rows);

function reportsPage(compact) {
  const read = state.data.report;
  const head = compact ? html`<div class="top"><h1>Reports</h1><span class="sp"></span>${connection()}</div>` : html`<div class="lhead"><h1>Reports</h1></div>`;
  if (!read) return html`${head}<div class="panel" role="region" aria-label="Reports"><p class="empty">No report yet. A workflow writes one with the <code>report</code> tool or <code>priory report put</code>.</p></div>`;
  const model = dossier(read, { now: Date.now(), query: state.reportQuery ?? "" });
  const { agreed, total, label } = model.progress;
  const { older, newer } = around(state.data.reports ?? [], model.task);
  const arrow = (task, glyph, name) => html`<button class="iconbtn" data-action="report-go" data-task="${task ?? ""}" aria-label="${name}" aria-disabled="${task == null ? "true" : "false"}">${icon(glyph, "s20")}</button>`;
  const bar = html`<div class="rbar">${arrow(older, "chevron-left", "Older report")}<span class="rdate">${model.date}</span>${arrow(newer, "chevron-right", "Newer report")}<span class="sp"></span>
    ${model.views[1].count ? html`<button class="btn secondary" data-action="report-next">Next pick</button>` : ""}
    <button class="iconbtn" data-action="report-search" aria-label="Search this report" aria-pressed="${state.reportQuery == null ? "false" : "true"}">${icon("search", "s20")}</button></div>
    ${state.reportQuery == null ? "" : html`<div class="rsearch"><label class="sr" for="report-query">Search this report</label><input id="report-query" type="search" placeholder="Search this report" value="${state.reportQuery}" autocomplete="off">${model.found == null ? "" : html`<span class="num" role="status">${model.found} found</span>`}</div>`}`;
  const jump = html`<nav class="jump" aria-label="Sections">${model.sections.map((section, i) => html`<a href="#report-${i}" aria-current="${i === 0 ? "true" : "false"}">${section.title}<span class="num">${section.count}</span></a>`)}</nav>`;
  const sections = model.sections.map((section, i) => html`<section class="sect" id="report-${i}" aria-labelledby="report-${i}-title">
    <h2 class="sh" id="report-${i}-title">${section.title}<span class="meta num">${section.count}</span></h2>
    <ol class="rrows">${section.rows.map(reportRow)}</ol></section>`);
  const pending = state.reportPending;
  const sending = pending ? html`<div class="sending" role="status"><span>Picking <b>"${pending.option}"</b></span>${pending.sending ? "" : html`<button class="btn secondary" data-action="pick-undo">Undo</button>`}</div>` : "";
  const found = model.found === 0 ? html`<p class="empty">Nothing in this report holds “${state.reportQuery.trim()}”.</p>` : "";
  return html`${head}<div class="panel report" role="region" aria-label="Reports">${bar}<div class="scroll" data-sections>
    <header class="rhead"><p class="greet">${model.greeting}</p><p class="meta">${model.date}, <a class="mono" href="/tasks/${model.task}" title="task ${model.task}">${taskName(model.task)}</a>. ${model.since}</p>
      ${state.reportFlash ? html`<p class="warnline" role="status">${state.reportFlash}</p>` : ""}
      ${total ? html`<p class="prog"><progress max="${total}" value="${agreed}" aria-label="Picks agreed"></progress><span class="num">${label}</span></p>` : ""}</header>
    <div class="rbody">${jump}<div>${found}${sections}</div></div></div>${sending}</div>`;
}

function reportRow(row) {
  const facts = [row.meta, row.owner].filter(Boolean);
  const notice = state.reportNotices.get(row.id);
  return html`<li class="rrow" id="${focusId("row", row.id)}">
    <p class="rl1">${row.tile ? tile(row.tile, "sm") : ""}${row.key ? html`<span class="mono">${autolinked(row.key)}</span>` : ""}<span class="rtext">${autolinked(row.text)}</span></p>
    ${facts.length || row.time ? html`<p class="rl2">${facts.map((fact) => html`<span>${fact}</span>`)}${row.time ? html`<span>${row.time}</span>` : ""}</p>` : ""}
    ${row.pick ? reportPick(row) : ""}
    ${notice ? html`<p class="warnline" role="status">${notice}</p>` : ""}
    ${row.threads.map((thread) => reportThread(row, thread))}${reportCompose(row)}</li>`;
}

// Each button's id names its pick, thread or row, so focus finds it again after a redraw.
function reportPick(row) {
  const { pick } = row;
  const pending = state.reportPending;
  const reopened = pick.state === "agreed" && state.reportChanging === row.id;
  if (pick.state !== "open" && !reopened) {
    const change = pick.state === "agreed" && !pending && !state.failed
      ? html`<button id="${focusId("change", row.id)}" class="ghost" data-action="pick-change" data-row="${row.id}">Change</button>` : "";
    return html`<p class="note rpicked">${icon("check", "s15")}<span><b>Picked ${pick.chosen}.</b> ${pick.note}</span>${change}</p>`;
  }
  const mine = pending?.pick === pick.id ? pending : null;
  const off = Boolean(state.failed || (pending && !mine));
  return html`<div class="rpick" role="group" aria-label="Pick for ${row.key || row.text}">${pick.options.map((option) => {
    const chosen = mine ? mine.option === option.label : option.label === pick.chosen;
    return html`<button id="${focusId("pick", pick.id, option.label)}" class="${chosen ? "chosen" : ""}" data-action="pick" data-row="${row.id}" data-option="${option.label}" aria-pressed="${chosen ? "true" : "false"}" aria-disabled="${off || mine ? "true" : "false"}">${chosen ? icon("check", "s15") : ""}${option.label}</button>`;
  })}</div>${state.failed ? html`<p class="note">${icon("unplug", "s15")}<span>Reconnect to pick.</span></p>`
    : reopened ? html`<p class="note"><span>Picked ${pick.chosen}. Pick another option to change it.</span>${mine ? "" : html`<button class="ghost" data-action="pick-keep">Keep ${pick.chosen}</button>`}</p>`
    : pick.note ? html`<p class="note">${pick.note}</p>` : ""}`;
}

function reportThread(row, thread) {
  const off = state.failed || state.remarking ? "true" : "false";
  if (thread.resolved) {
    return html`<p class="note rresolved">${icon("check", "s15")}<span>Resolved: ${thread.text}</span><button id="reopen-${thread.id}" class="ghost" data-action="reopen" data-row="${row.id}" data-comment="${thread.id}" aria-disabled="${off}">Reopen</button></p>`;
  }
  const entry = (comment) => html`<li><p class="rwho"><b>${comment.author}</b>${comment.when ? html` <span>${comment.when}</span>` : ""}</p><p class="rsay">${comment.text}</p></li>`;
  return html`<div class="rthread" aria-label="Thread ${thread.id}"><ol>${entry(thread)}${thread.replies.map(entry)}</ol>
    <p class="racts"><button id="reply-${thread.id}" class="ghost" data-action="reply" data-row="${row.id}" data-comment="${thread.id}" aria-disabled="${off}">Reply</button><button id="resolve-${thread.id}" class="ghost" data-action="resolve" data-row="${row.id}" data-comment="${thread.id}" aria-disabled="${off}">Resolve</button></p></div>`;
}

const onNewest = () => state.reportShown == null && state.data.report?.report.task === state.data.reports?.[0]?.task;

// A row without its own id takes no comments. Priory gives a row id's new thread to the newest
// report holding it, so an older report takes replies only.
function reportCompose(row) {
  if (!row.target) return "";
  const compose = state.compose?.row === row.id && state.compose.task === state.data.report.report.task ? state.compose : null;
  const off = state.failed ? "true" : "false";
  if (!compose && !onNewest()) return "";
  if (!compose) return html`<p class="racts"><button id="${focusId("compose", row.id)}" class="ghost" data-action="compose" data-row="${row.id}" aria-disabled="${off}">${icon("message-text", "s15")}Comment</button></p>`;
  const verb = compose.replyTo == null ? "Comment" : "Reply";
  return html`<form class="writeform rcompose" data-action="remark">${compose.replyTo == null ? "" : html`<p class="note">Replying in thread ${compose.replyTo}</p>`}
    <label class="sr" for="report-draft">${verb} on this row</label><textarea id="report-draft" name="text" rows="2" placeholder="${compose.replyTo == null ? "Comment on this row" : "Reply in this thread"}">${compose.text}</textarea>
    <p class="racts"><button type="button" class="btn secondary" data-action="compose-cancel">Cancel</button><button class="btn primary" aria-disabled="${state.failed || state.remarking ? "true" : "false"}">${verb}</button></p></form>`;
}

// An id no other control's parts can spell: whitespace and quotes are escaped away.
const focusId = (...parts) => encodeURIComponent(JSON.stringify(parts));

// A note under its row, or above the sections once the row has left the report or another task's
// report replaced it.
function noteRow(id, text, task = state.data.report?.report.task) {
  if (task === state.data.report?.report.task && reportRowOf(id)) state.reportNotices.set(id, text);
  else state.reportFlash = text;
}

const reportRowOf = (id) => (state.data.report ? reportRows(dossier(state.data.report, { now: Date.now() })).find((row) => row.id === id) : null);

// Nothing leaves the page until the Undo time passes, as an answer waits: a pick wakes the report's agent.
function pick(target) {
  const row = reportRowOf(target.dataset.row);
  const reopened = row?.pick?.state === "agreed" && state.reportChanging === row.id;
  if (!row?.pick || (row.pick.state !== "open" && !reopened) || target.dataset.option === row.pick.chosen || state.reportPending || state.failed) return;
  state.reportNotices.delete(row.id);
  state.reportFlash = null;
  const pending = { row: row.id, task: state.data.report.report.task, pick: row.pick.id, option: target.dataset.option, from: row.pick.chosen, revision: state.data.report.report.revision, sending: false };
  pending.timer = setTimeout(() => sendPick(pending), UNDO_MS);
  state.reportPending = pending;
  render();
}

function undoPick() {
  if (!state.reportPending || state.reportPending.sending) return;
  clearTimeout(state.reportPending.timer);
  state.reportPending = null;
  render();
}

// A new read cancels a pick still waiting out its Undo once its report no longer offers it. Another
// task's report drops the notes, which belong to rows of the old one; the comment field waits for
// its own task's report.
function acceptReport() {
  const task = state.data.report?.report.task;
  if (task !== state.reportTask) {
    state.reportNotices.clear();
    state.reportFlash = null;
    state.reportTask = task;
  }
  const pending = state.reportPending;
  if (!pending || pending.sending) return;
  const row = state.data.report?.report.task === pending.task ? reportRowOf(pending.row) : null;
  if (row?.pick?.id === pending.pick && row.pick.state !== "applied" && row.pick.chosen === pending.from) return;
  clearTimeout(pending.timer);
  state.reportPending = null;
  noteRow(pending.row, `${pending.option} was not sent: the report changed before it went.`, pending.task);
}

async function sendPick(pending) {
  pending.sending = true;
  render();
  const { status, body } = await post("/api/reports/pick", { pick: pending.pick, option: pending.option, revision: pending.revision })
    .catch(() => ({ status: 0, body: { error: "the connection dropped" } }));
  state.reportPending = null;
  state.reportChanging = null;
  if (status === 401) return signOut("The daemon refused this token. Paste the current one.");
  await refresh();
  if (status === 200) return render();
  const fresh = status === 409 && state.data.report?.report.task === pending.task ? state.data.report : null;
  noteRow(pending.row, `${pending.option}: ${writeRefusal(status, body, () => refusal(pending, fresh, body.error))}`, pending.task);
  render();
}

// Why a write did not go; `refused` words a refusal the daemon gave.
function writeRefusal(status, body, refused = () => `Not sent: ${body.error || `the server answered ${status}`}.`) {
  if (status === 503) return "Could not send: no priory daemon runs beside this server. Start priory daemon, then send again.";
  if (status === 504 || status === 0) return "The daemon may have applied this but did not confirm it. Check the report before you send again.";
  return refused();
}

function compose(target) {
  const replyTo = target.dataset.comment ? Number(target.dataset.comment) : null;
  const same = state.compose?.row === target.dataset.row && state.compose.task === state.data.report.report.task;
  state.compose = { task: state.data.report.report.task, row: target.dataset.row, replyTo, text: same ? state.compose.text : "" };
  render();
  document.getElementById("report-draft")?.focus();
}

// Sends one comment verb; a refusal says why under the row and keeps the draft. `sent` runs before
// the read that follows, so a second press cannot send the same text again.
async function remark(rowId, path, body, sent = () => {}) {
  if (state.remarking || state.failed) return;
  const task = state.data.report.report.task;
  state.remarking = true;
  state.reportNotices.delete(rowId);
  render();
  const { status, body: answered } = await post(path, body).catch(() => ({ status: 0, body: { error: "the connection dropped" } }));
  if (status === 401) {
    state.remarking = false;
    return signOut("The daemon refused this token. Paste the current one.");
  }
  if (status === 200) sent();
  await refresh();
  if (status !== 200) noteRow(rowId, writeRefusal(status, answered), task);
  state.remarking = false;
  render();
}

async function submitRemark() {
  const sent = state.compose;
  const row = sent?.task === state.data.report?.report.task && reportRowOf(sent.row);
  const text = sent?.text.trim();
  if (!row?.target || !text || state.remarking || (sent.replyTo == null && !onNewest())) return;
  // Text typed while it was in flight stays as the next draft.
  const done = () => { if (state.compose === sent && sent.text.trim() === text) state.compose = null; };
  await remark(row.id, ...(sent.replyTo == null
    ? ["/api/reports/comment", { row: row.target, text, revision: state.data.report.report.revision }]
    : ["/api/reports/reply", { comment: sent.replyTo, text }]), done);
}

// The newest report is followed as it changes; an older one stays until an arrow moves on.
async function showReport(task) {
  state.reportShown = task === state.data.reports?.[0]?.task ? null : task;
  state.reportChanging = null;
  state.reportAt = null;
  await refresh();
  render();
  document.querySelector(".report .scroll")?.scrollTo({ top: 0 });
}

function reachNextPick() {
  const id = nextPick(dossier(state.data.report, { now: Date.now(), query: state.reportQuery ?? "" }), state.reportAt);
  if (!id) return;
  state.reportAt = id;
  const row = document.getElementById(focusId("row", id));
  row?.scrollIntoView({ block: "center", behavior: matchMedia("(prefers-reduced-motion: reduce)").matches ? "auto" : "smooth" });
  row?.querySelector(".rpick button")?.focus({ preventScroll: true });
}

function reportAct(target) {
  if (target.getAttribute("aria-disabled") === "true") return;
  const action = target.dataset.action;
  if (action === "pick") return pick(target);
  if (action === "pick-undo") return undoPick();
  if (action === "pick-change" || action === "pick-keep") {
    state.reportChanging = action === "pick-change" ? target.dataset.row : null;
    return render();
  }
  if (action === "report-go") return showReport(Number(target.dataset.task));
  if (action === "report-next") return reachNextPick();
  if (action === "report-search") {
    state.reportQuery = state.reportQuery == null ? "" : null;
    render();
    return document.getElementById("report-query")?.focus();
  }
  if (action === "compose" || action === "reply") return compose(target);
  if (action === "compose-cancel") {
    state.compose = null;
    return render();
  }
  remark(target.dataset.row, `/api/reports/${action}`, { comment: Number(target.dataset.comment) });
}

// Add (DESIGN.md, Add): one field and three chips; Enter queues the task through POST /api/request.
// The draft lives in localStorage until the daemon takes it, so a reload or a dropped connection keeps it.
const DRAFT_KEY = "priory.add";
const addSheet = () => document.getElementById("add");
// Only an unsent draft keeps its phase across a reload, so it still asks before it sends.
const keepDraft = () => localStorage.setItem(DRAFT_KEY, JSON.stringify({ draft: state.add.draft, phase: ["offline", "unsure"].includes(state.add.phase) ? state.add.phase : "empty" }));
// Lanes have no list of their own: the chip offers each lane a listed task carries.
const knownLanes = () => [...new Set([...state.data.tasks, ...state.data.queue.waiting].map((task) => task.lane).filter(Boolean))].sort();

async function openAdd() {
  if (!state.data || addSheet().open) return;
  if (state.add.phase === "added") Object.assign(state.add, { phase: "empty", task: null });
  if (!state.add.defaults) {
    state.add.defaults = await api("/api/request").catch(() => null);
    const { draft, defaults } = state.add;
    if (defaults && draft.workflow === undefined) Object.assign(draft, { workflow: defaults.workflow || "", band: defaults.band || "" });
  }
  renderAdd();
  addSheet().showModal();
  document.getElementById("add-text")?.focus();
}

function closeAdd() {
  addSheet().close();
}

// The answer sheet (DESIGN.md, Answer while reading): every option of the ask on screen, opened by the bar's "N options".
const answerSheet = () => document.getElementById("answer");

function openSheet() {
  if (!askOnScreen()) return;
  renderSheet();
  answerSheet().showModal();
}

// Closes itself once the ask is gone, answered, or its answer is sending.
function renderSheet() {
  const sheet = answerSheet();
  const ask = state.data && askOnScreen();
  if (!ask || ask.state !== "open" || state.pending) return sheet.close();
  const focused = document.activeElement?.closest?.("#answer") ? focusKey(document.activeElement) : null;
  sheet.innerHTML = html`<div class="shead"><h2 id="answer-title">Answer ask #${ask.id}<small>revision ${ask.revision}</small></h2><button class="iconbtn" data-action="sheet-close" aria-label="Close">${icon("x", "s20")}</button></div>
    <div class="opts" role="group" aria-label="Options">${optionList(ask, false)}</div>`.text;
  if (focused) sheet.querySelector(focused)?.focus();
}

function chip(name, label, value, values, none) {
  const options = [...new Set([...values, value].filter(Boolean))];
  return html`<label class="chip"><span>${label}</span><select name="${name}" data-add-chip="${name}">
    <option value="" ${value ? "" : raw("selected")}>${none}</option>
    ${options.map((option) => html`<option ${option === value ? raw("selected") : ""}>${option}</option>`)}</select>${icon("chevron-down", "s13")}</label>`;
}

function addedTask() {
  const { key, id, body } = state.add.task;
  const task = body || { key, state: "queued" };
  return html`<div class="added" role="status">
    <p class="line">Added. The task waits in the queue for its turn.</p>
    <div class="row">${tile(task.state)}
      <span class="l1"><span class="mono">${taskLabel(task.key)}</span><span>${[task.workflow, task.step !== task.workflow && task.step].filter(Boolean).join(", ")}</span><span class="age">${task.state}</span></span>
      <span class="l2">${task.brief?.request || task.brief?.title || (id === null ? "No task took this key yet. It shows on the Desk once one does." : "")}</span>
      <span class="l3">${[task.band, task.lane].filter(Boolean).join(", ")}</span></div>
    <div class="acts"><button class="btn secondary" data-action="add-another">Add another</button><button class="btn primary" data-action="add-done">Close</button></div></div>`;
}

const ADD_LINES = {
  offline: "Offline. The draft stays on this device, and nothing is sent until you confirm it after you reconnect.",
  unsure: "The connection dropped while sending, so Priory may have queued it. The draft stays until you send it again or discard it.",
};

function renderAdd() {
  const sheet = addSheet();
  if (!sheet) return;
  const { draft, phase, error, defaults } = state.add;
  const sending = phase === "sending";
  const back = !state.failed && ["offline", "unsure"].includes(phase);
  const status = phase === "refused"
    ? html`<p class="field-error" id="add-status">${icon("circle-x", "s15")}Could not add: ${error}.</p>`
    : back
      ? html`<p class="warnline" id="add-status">${icon("unplug", "s15")}<span>${phase === "unsure"
        ? "Reconnected. Priory may have queued this before the drop: check the Desk's All view, then send it if it is missing."
        : "Reconnected. This draft has not been sent. Send it now?"}</span></p>`
      : ADD_LINES[phase] ? html`<p class="warnline" id="add-status">${icon("unplug", "s15")}<span>${ADD_LINES[phase]}</span></p>` : "";
  const empty = !draft.text.trim();
  const label = sending ? "Adding…" : back ? "Send" : state.failed ? "Keep draft" : "Add";
  const body = phase === "added" ? addedTask() : html`<form data-action="add" ${sending ? raw('aria-busy="true"') : ""}>
    <label class="sr" for="add-text">What should the factory do?</label>
    <input id="add-text" name="text" autocomplete="off" enterkeyhint="send" placeholder="What should the factory do?" value="${draft.text}" ${sending ? raw("readonly") : ""} ${status ? raw('aria-describedby="add-status"') : ""}>
    ${status}
    <div class="chips">${chip("workflow", "Workflow", draft.workflow, defaults?.workflows || [], defaults?.workflow ? `default (${defaults.workflow})` : "pick one")}
      ${chip("band", "Band", draft.band, defaults?.bands || state.data?.bands || [], "default")}
      ${chip("lane", "Lane", draft.lane, state.data ? knownLanes() : [], "routed")}</div>
    <div class="acts">${phase === "empty" || sending ? "" : html`<button type="button" class="ghost" data-action="add-discard">Discard draft</button>`}<span class="sp"></span>
      <button class="btn primary" aria-disabled="${empty || sending ? "true" : "false"}">${label}</button></div></form>`;
  const focused = document.activeElement?.closest?.("#add") ? focusKey(document.activeElement) : null;
  const caret = document.activeElement?.id === "add-text" ? document.activeElement.selectionStart : undefined;
  state.add.shownOffline = Boolean(state.failed);
  sheet.innerHTML = html`<div class="shead"><h2 id="add-title">Add a task</h2><button class="iconbtn" data-action="add-close" aria-label="Close">${icon("x", "s20")}</button></div>${body}`.text;
  if (focused) sheet.querySelector(focused)?.focus();
  if (caret !== undefined) sheet.querySelector("#add-text")?.setSelectionRange(caret, caret);
}

async function submitAdd() {
  const { draft, phase } = state.add;
  const body = addBody(draft);
  if (!body || phase === "sending") return;
  // Offline, Enter only keeps the draft; it goes once the person confirms it after reconnecting.
  if (state.failed) {
    state.add.phase = "offline";
    keepDraft();
    return renderAdd();
  }
  // Stored as unsure first: a reload while it sends must not offer it as unsent.
  localStorage.setItem(DRAFT_KEY, JSON.stringify({ draft, phase: "unsure" }));
  state.add.phase = "sending";
  renderAdd();
  const reply = await post("/api/request", body).catch(() => ({ status: 0, body: {} }));
  const outcome = addOutcome(reply.status, reply.body);
  if (outcome.state === "signed-out") {
    state.add.phase = "empty";
    keepDraft();
    closeAdd();
    return signOut("The daemon refused this token. Paste the current one.");
  }
  if (outcome.state === "added") {
    const { defaults } = state.add;
    state.add.draft = { text: "", workflow: defaults?.workflow || "", band: defaults?.band || "", lane: "" };
    state.add.task = { key: outcome.key, id: outcome.id, body: null };
    localStorage.removeItem(DRAFT_KEY);
    if (outcome.id !== null) api(`/api/tasks/${outcome.id}`).then((task) => { if (state.add.task?.id === outcome.id) { state.add.task.body = task; renderAdd(); } }, () => {});
    refresh();
  }
  Object.assign(state.add, { phase: outcome.state, error: outcome.error || null });
  if (outcome.state !== "added") keepDraft();
  renderAdd();
  if (outcome.state === "added") addSheet().querySelector(".added .btn.primary")?.focus();
}

function discardAdd() {
  const { defaults } = state.add;
  Object.assign(state.add, { phase: "empty", error: null, draft: { text: "", workflow: defaults?.workflow || "", band: defaults?.band || "", lane: "" } });
  localStorage.removeItem(DRAFT_KEY);
  renderAdd();
  document.getElementById("add-text")?.focus();
}

// Frame pieces
function connection() {
  const off = Boolean(state.failed);
  return html`<span class="conn ${off ? "off" : ""}">${off ? "reconnecting" : "live"}</span>`;
}

function staleStrip() {
  if (!state.failed) return "";
  const since = state.updated ? `Last update ${ago(new Date(state.updated).toISOString())}.` : "";
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
  return html`<nav class="tabbar" aria-label="Panels">${shown.map(link)}<a href="/more" aria-current="${current === "more" || current === "settings" || (PANELS[current] && !shown.includes(current)) ? "page" : "false"}">${icon("more", "s20")}More</a></nav>`;
}

function laterPanel(panel) {
  if (panel === "terminal") return terminalsPanel();
  const [label] = PANELS[panel];
  return html`<p class="empty">The ${label} panel comes in a later version of the web view. Meanwhile run <code>${CLI[panel]}</code>.</p>`;
}

// Flows: each workflow drawn top to bottom. The factory is read once, so the flows load once.
const KIND_WORD = { read: "read", reversible: "reversible write", irreversible: "irreversible write", ask: "ask", answer: "answer", verdict: "verdict" };
const BY_WORD = { code: "code decides", model: "a model decides", human: "a person decides", asleep: "asleep, waiting" };
const clip = (text, size) => (text.length > size ? `${text.slice(0, size - 1)}...` : text);

async function loadFlows() {
  const token = state.token;
  if (state.flowsLoading || !token || !state.data || state.failed) return;
  state.flowsLoading = true;
  try {
    const flows = await api("/api/flows");
    if (token !== state.token || !state.data) return;
    state.data.flows = flows;
    state.flowsError = null;
    keep();
  } catch (error) {
    if (token !== state.token) return;
    if (error.status === 401) return signOut("The daemon refused this token. Paste the current one.");
    state.flowsError = error instanceof TypeError ? "the connection dropped" : error.message;
  } finally {
    state.flowsLoading = false;
  }
  render();
}

const flowNode = (node) => {
  if (node.terminal) {
    return html`<g transform="translate(${node.x} ${node.y})" class="fnode end"><title>${node.label}</title><rect width="${node.width}" height="${node.height}" rx="${node.height / 2}"/><text x="${node.width / 2}" y="${node.height / 2 + 4}" text-anchor="middle">${node.label}</text></g>`;
  }
  const shape = nodeShape(node.kind, node.width, node.height);
  const second = node.gate ? clip(node.gate, 24) : KIND_WORD[node.kind] ?? node.kind;
  const badge = lockBadge(node.width);
  const lock = node.kind === "irreversible" ? html`<svg class="ico flock" x="${badge.x}" y="${badge.y}" width="${badge.size}" height="${badge.size}" viewBox="0 0 24 24" aria-hidden="true">${raw(ICONS.lock)}</svg>` : "";
  return html`<g transform="translate(${node.x} ${node.y})" class="fnode k-${node.kind}"><title>${node.label}: ${KIND_WORD[node.kind] ?? node.kind}${node.gate ? `, ${node.gate}` : ""}</title>
    <path d="${shape.d}"/><text x="${node.width / 2}" y="${node.height / 2 - 3}" text-anchor="middle" class="fl1">${clip(node.label, 20)}</text>
    <text x="${node.width / 2}" y="${node.height / 2 + 13}" text-anchor="middle" class="fl2">${second}</text>${lock}</g>`;
};

const flowEdge = (edge) => {
  const style = edgeStyle(edge.by);
  const line = (extra) => html`<path d="${edge.d}" fill="none" stroke-width="${style.width}" ${style.dash ? raw(`stroke-dasharray="${style.dash}"`) : ""} class="${extra}"/>`;
  return html`<g class="fedge by-${edge.by}${edge.ungated ? " ungated" : ""}"><title>${edge.outcome}: ${edge.from} to ${edge.to}, ${BY_WORD[edge.by] ?? edge.by}${edge.ungated ? ", reaches an irreversible step with no person's decision" : ""}</title>
    ${edge.ungated ? html`<path d="${edge.d}" fill="none" stroke-width="${style.width + 6}" class="halo"/>` : ""}${line("fline")}
    <text x="${edge.mid.x + (edge.vertical ? 0 : 5)}" y="${edge.mid.y}" class="felabel"${edge.vertical ? raw(` transform="rotate(-90 ${edge.mid.x} ${edge.mid.y})" text-anchor="middle"`) : ""}>${clip(edge.outcome, 14)}</text></g>`;
};

const flowLegend = () => html`<section class="sect flegend" aria-labelledby="legend-title"><h2 class="sh" id="legend-title">Key</h2>
  <ul class="fkey">${KINDS.map((kind) => html`<li><svg viewBox="0 0 150 52" width="45" height="16" aria-hidden="true" class="fnode k-${kind}"><path d="${nodeShape(kind, 150, 52).d}"/></svg>${KIND_WORD[kind]}</li>`)}
  ${["code", "model", "human", "asleep"].map((by) => html`<li><svg viewBox="0 0 48 8" width="48" height="8" aria-hidden="true" class="fedge by-${by}"><path d="M0 4H48" stroke-width="${edgeStyle(by).width}" ${edgeStyle(by).dash ? raw(`stroke-dasharray="${edgeStyle(by).dash}"`) : ""} class="fline"/></svg>${BY_WORD[by]}</li>`)}
  <li><svg viewBox="0 0 48 12" width="48" height="12" aria-hidden="true" class="fedge ungated"><path d="M0 6H48" stroke-width="8" class="halo"/></svg>no person's decision before an irreversible step</li></ul></section>`;

function flowHistory(rows) {
  if (!rows.length) return "";
  const outcome = (label, [count, bound]) => html`<td data-label="${label}">${count}<span class="bound">${bound}</span></td>`;
  return html`<div class="history"><p class="note">Tasks through each irreversible step in the last 180 days, judged within 30 days of it.</p>
    <table class="ctbl rtbl"><thead><tr><th scope="col">Step</th><th scope="col" class="r">Caught</th><th scope="col">Judged wrong</th><th scope="col">Landed wrong</th></tr></thead>
    <tbody>${rows.map((row) => html`<tr><th scope="row" class="mono">${row.step}</th><td class="r" data-label="Caught">${row.caught}</td>${outcome("Judged wrong", row.judged)}${outcome("Landed wrong", row.landed)}</tr>`)}</tbody></table></div>`;
}

function flowSection(workflow, paths, irreversible) {
  const ungated = ungatedPairs(workflow.name, paths);
  const drawing = timeline(workflow, ungated);
  const open = paths.filter((entry) => entry.workflow === workflow.name);
  const id = `flow-${workflow.name.replace(/[^\w-]/g, "_")}`;
  return html`<section class="sect flow" aria-labelledby="${id}"><h2 class="sh" id="${id}">${workflow.name}<span class="meta num">${workflow.steps.length} steps</span></h2>
    <svg class="diagram" viewBox="0 0 ${drawing.width} ${drawing.height}" role="img" aria-label="${drawing.text}">
      <defs><marker id="arrow" viewBox="0 0 10 10" refX="9" refY="5" markerUnits="userSpaceOnUse" markerWidth="10" markerHeight="10" orient="auto-start-reverse"><path d="M0 1L10 5L0 9z" fill="context-stroke"/></marker></defs>
      ${drawing.edges.map(flowEdge)}${drawing.nodes.map(flowNode)}</svg>
    ${flowHistory(historyRows(irreversible ?? [], workflow.name))}
    ${open.length ? html`<div class="unattended" role="note"><p class="t-danger">${icon("lock", "s15")}Reaches an irreversible step with no person's decision:</p><ul>${open.map((entry) => html`<li class="mono">${entry.path.join(" > ")}</li>`)}</ul></div>` : ""}
    <details class="astext"><summary>Steps as text</summary><ol>${workflow.steps.map((step) => html`<li><b class="mono">${step.id}</b>, ${KIND_WORD[step.kind] ?? step.kind}, decided by ${step.decides}. ${step.edges.map((edge) => html`${edge.outcome} leads to ${edge.to} (${BY_WORD[edge.by] ?? edge.by}). `)}</li>`)}</ol></details></section>`;
}

function flowsPage(compact) {
  const head = compact ? html`<div class="top"><h1>Flows</h1><span class="sp"></span>${connection()}</div>` : html`<div class="lhead"><h1>Flows</h1></div>`;
  const flows = state.data.flows;
  if (!flows && state.flowsError) return html`${head}<p class="empty" role="alert">The flows did not load: ${state.flowsError}. Run <code>priory check --explain</code> meanwhile.</p><p class="empty"><button class="btn secondary" data-action="flows-retry">Retry</button></p>`;
  if (!flows) { setTimeout(loadFlows); return html`${head}<p class="empty" role="status">Loading the flows.</p>`; }
  const body = flows.workflows.length ? html`${flowLegend()}${flows.workflows.map((workflow) => flowSection(workflow, flows.paths, flows.irreversible))}` : html`<p class="empty">This factory has no workflows. Add one under <code>workflows/</code>.</p>`;
  return html`${head}<div class="panel" role="region" aria-label="Flows"><div class="scroll">${body}</div></div>`;
}

function morePage() {
  const rest = panelsOf().slice(4);
  return html`<div class="top"><h1>More</h1></div><div class="scroll"><ul class="links">
    ${rest.map((panel) => html`<li><a href="/${panel}">${icon(PANELS[panel][1], "s20")}${PANELS[panel][0]}<span class="chev">${icon("chevron-right", "s15")}</span></a></li>`)}
    <li><a href="/settings/appearance">${icon("settings", "s20")}Appearance<span class="chev">${icon("chevron-right", "s15")}</span></a></li>
    <li><a href="/settings/notifications">${icon("bell", "s20")}Notifications<span class="chev">${icon("chevron-right", "s15")}</span></a></li></ul>
    <div class="signout"><button class="btn secondary" data-action="sign-out">${icon("lock", "s20")}Sign out of this device</button></div></div>`;
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
    <p class="note">A theme changes color and type only. Every screen keeps the same layout.</p></div>
    <ul class="links"><li><a href="/settings/notifications">${icon("bell", "s20")}Notifications<span class="chev">${icon("chevron-right", "s15")}</span></a></li></ul></div>`;
}

// Notifications
const settingsScreen = () => (route()[1] === "notifications" ? notifications() : appearance());

// iOS lets only a Home Screen web app subscribe, so Safari in a tab lacks PushManager.
const iosTab = () => /iPhone|iPad|iPod/.test(navigator.userAgent) && !navigator.standalone;

async function pushStatus() {
  if (!window.isSecureContext) return "insecure";
  if (!("serviceWorker" in navigator) || !("PushManager" in window) || !("Notification" in window)) return iosTab() ? "install" : "unsupported";
  if (Notification.permission === "denied") return "blocked";
  const registration = await navigator.serviceWorker.ready;
  const subscription = await registration.pushManager.getSubscription();
  if (!subscription) return "off";
  // The browser keeps a subscription the daemon may have dropped or never stored: send it again.
  // Only the daemon's refusal ends it; a dropped connection or a stopped daemon proves nothing.
  const { status } = await post("/api/push/subscribe", subscription.toJSON()).catch(() => ({ status: 0 }));
  if (status === 200) return "on";
  if (status !== 422) return "unconfirmed";
  await subscription.unsubscribe().catch(() => {});
  return "off";
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
    const settle = (found) => {
      if (check !== state.push.check) return;
      Object.assign(state.push, { status: found, checking: false });
      render();
    };
    pushStatus().then(settle, () => settle("unsupported"));
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
    <p>Get a notification on this device when an agent asks you something, or when a task is blocked or failed. It shows the ask's title and opens the ask. A snoozed item sends nothing.</p>
    ${PUSH_NOTES[status] ? html`<p class="note" role="status">${icon(status === "on" ? "check" : "unplug", "s15")}<span>${PUSH_NOTES[status]}</span></p>` : ""}
    ${error ? html`<p class="note warn" role="alert">${icon("unplug", "s15")}<span>${error}</span></p>` : ""}
    ${button ? html`<div class="push-row">${button}</div>` : ""}
    ${install}</div></div>`;
}

// The VAPID key as the bytes PushManager.subscribe takes.
const keyBytes = (base64url) => Uint8Array.from(atob(base64url.replace(/-/g, "+").replace(/_/g, "/")), (char) => char.charCodeAt(0));

async function turnPush(on) {
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
        const { status, body } = await post("/api/push/subscribe", subscription.toJSON());
        if (status !== 200) throw new Error(body.error || `the server answered ${status}`);
      } catch (error) {
        await subscription.unsubscribe().catch(() => {});
        throw error;
      }
      state.push.status = "on";
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

// Screens
function deskPhone(withCapsule = true) {
  const { model, views, banner, rows } = deskBody(false);
  const n = model.open.length;
  return html`<div class="top"><span class="fac">${state.data.views.factory}</span>${connection()}</div>
    <div class="panel" role="region" aria-label="Desk"><h1 class="sr">Desk</h1><div class="scroll ${withCapsule ? "desk-end" : ""}">
      <div class="count"><span class="n num">${n}</span><span class="w">${n === 1 ? "ask needs you" : "asks need you"}<span>${countLine(model)}</span></span></div>
      ${nextCard(model)}${views}${banner}${rows}</div>${withCapsule ? capsule(model) : ""}</div>`;
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
      ${nextCard(model)}${views}${banner}<div class="scroll">${rows}</div>${wide ? keyHints() : ""}</div>`;
  }
  if (panel === "inbox") {
    return html`<div class="list" role="region" aria-label="Inbox"><div class="lhead"><h1>Inbox</h1></div>${inboxViews()}<div class="scroll">${unroutedRows()}</div>${wide ? keyHints() : ""}</div>`;
  }
  if (panel === "asks") {
    const { views, rows } = asksBody(wide);
    return html`<div class="list" role="region" aria-label="Asks"><div class="lhead"><h1>Asks</h1></div>${views}<div class="scroll">${rows}</div>${wide ? keyHints() : ""}</div>`;
  }
  if (OWN_PAGE_LISTS[panel]) {
    const { views, rows } = OWN_PAGE_LISTS[panel](wide);
    return html`<div class="list" role="region" aria-label="${PANELS[panel][0]}"><div class="lhead"><h1>${PANELS[panel][0]}</h1></div>${views}<div class="scroll">${rows}</div></div>`;
  }
  return html`<div class="list" role="region" aria-label="${PANELS[panel][0]}"><div class="lhead"><h1>${PANELS[panel][0]}</h1></div>${laterPanel(panel)}</div>`;
}

// Queue (DESIGN.md, Queue panel): the waiting tasks by band in admission order, why each waits, and
// a Move menu that sends `priory move` at the revision the page drew.
function moveMenu(task, bands) {
  const options = moveOptions(task, bands, state.data.queue.bands, state.data.queue.revision);
  const busy = state.moveBusy === task.id ? raw('aria-disabled="true"') : "";
  // A live redraw keeps the menu that is open, and the ids let focus return to it.
  return html`<details class="move" data-move="${task.id}" ${state.moveOpen === task.id ? raw("open") : ""}><summary id="move-${task.id}" class="btn secondary" aria-label="Move ${taskLabel(task.key)}">Move${icon("chevron-down", "s13")}</summary>
    <div class="pick">${options.map((option, at) => html`<button type="button" id="move-${task.id}-${at}" data-action="move" data-task="${task.id}" data-option="${at}" ${busy}>${option.label}</button>`)}</div></details>`;
}

const moveNote = (task) => (state.moveNotices.has(task.id) ? html`<p class="note warn" role="status">${icon("circle-x", "s15")}<span>${state.moveNotices.get(task.id)}</span></p>` : "");

function queueListRow(task, bands) {
  return html`<div class="row qrow" data-href="/tasks/${task.id}">${tile(task.state)}
    <span class="l1"><span class="num">#${task.band_position}</span><a class="mono" href="/tasks/${task.id}" title="task ${task.id}">${taskLabel(task.key)}</a><span>${task.state}</span><span class="age">${timeTag(task.since)}</span></span>
    <span class="l2 r">${task.title || ""}</span><span class="l3 wrap">${waitReason(task, bands)}</span>${moveMenu(task, bands)}${moveNote(task)}</div>`;
}

function queueTableRow(task, bands) {
  return html`<tr data-href="/tasks/${task.id}"><td class="r">#${task.band_position}</td><td>${stateWord(task.state)}</td><td class="mono"><a href="/tasks/${task.id}" title="task ${task.id}">${taskLabel(task.key)}</a></td>
    <td class="q full"><span>${task.title || ""}</span><span class="why t3">${waitReason(task, bands)}</span>${moveNote(task)}</td><td class="act">${moveMenu(task, bands)}</td></tr>`;
}

function queueBody(wide) {
  const { queue } = state.data;
  const current = listView(QUEUE_VIEWS);
  const bands = queueBands(queue);
  const held = queue.waiting.filter((task) => task.state === "held");
  const waiting = bands.reduce((sum, group) => sum + group.tasks.length, 0);
  const views = savedViews([["waiting", "Waiting", waiting, current === "waiting"], ["held", "Held", held.length, current === "held"], ["ended", "Last day", queue.ended.length, current === "ended"]], "/queue");
  if (current === "held") return { views, rows: listOrEmpty(held.map((task) => taskListRow({ ...task, reason: task.reason || "held" })), "No task is held. A task waits here while a person or a condition holds it.") };
  if (current === "ended") return { views, rows: listOrEmpty(queue.ended.map((task) => taskListRow({ ...task, since: task.ended })), "No task ended in the last day.") };
  const strip = upNext(deskModel(), false);
  if (!bands.length) return { views, rows: html`${strip}<p class="empty">The queue is empty. A task waits here for its turn once it is added.</p>` };
  if (wide) {
    const groups = bands.map((group) => html`<tr class="grp"><td colspan="5">Band ${group.band}<span>${group.tasks.length}</span></td></tr>${group.tasks.map((task) => queueTableRow(task, bands))}`);
    return { views, rows: html`${strip}<table class="tbl q"><thead><tr><th class="r">#</th><th>State</th><th>Task</th><th>Title and why it waits</th><th></th></tr></thead><tbody>${groups}</tbody></table>` };
  }
  return { views, rows: [strip, ...bands.map((group) => html`<div class="sec">Band ${group.band}<span>${group.tasks.length}</span></div>${group.tasks.map((task) => queueListRow(task, bands))}`)] };
}

// Sends one Move option of a task, as `priory move --expected`; a refusal shows under the row.
async function moveTask(target) {
  if (state.moveBusy !== null) return;
  const task = state.data.queue.waiting.find((each) => each.id === Number(target.dataset.task));
  const option = task && moveOptions(task, queueBands(state.data.queue), state.data.queue.bands, state.data.queue.revision)[Number(target.dataset.option)];
  if (!option) return;
  state.moveBusy = task.id;
  render();
  const { status, body } = await post(`/api/tasks/${task.id}/move`, option.body).catch(() => ({ status: 0, body: { error: "the connection dropped" } }));
  Object.assign(state, { moveBusy: null, moveOpen: null });
  if (status === 401) return signOut("The daemon refused this token. Paste the current one.");
  if (status === 200) state.moveNotices.delete(task.id);
  else state.moveNotices.set(task.id, actionNote("Move", status, body));
  render();
  document.getElementById(`move-${task.id}`)?.focus();
  refresh();
}

// Lists whose rows open a page of their own, so they take the whole width, with no drawer.
const OWN_PAGE_LISTS = { tasks: tasksBody, runs: runsBody, queue: queueBody };

const keyHints = () => html`<div class="keys"><span><kbd>J</kbd><kbd>K</kbd>move</span><span><kbd>Enter</kbd>open</span><span><kbd>Esc</kbd>close</span><span><kbd>N</kbd>add</span></div>`;

function drawer(wide, panels) {
  const id = selectedAsk();
  const body = id ? askDetail(id, wide) : html`<p class="empty">${emptyDetail(panels)}</p>`;
  return html`<aside class="drawer" aria-label="Ask">${body}</aside>`;
}

function current() {
  const [first] = route();
  if (!first || first === "index.html") return "desk";
  if (first === "settings") return "settings";
  return first;
}

const shownTab = (children, list) => children.find((child) => leaves(child).includes(list)) || children[0];

// The panels `tree` draws: each tabs node shows one child.
function shownPanels(node, list) {
  if (node.panel) return [node.panel];
  const [kind, children] = Object.entries(node)[0];
  return kind === "tabs" ? shownPanels(shownTab(children, list), list) : children.flatMap((child) => shownPanels(child, list));
}

// The view file's tree, for a layout that is not a plain set of tabs.
function tree(node, list) {
  if (node.panel) return listPanel(node.panel, true);
  const [kind, children] = Object.entries(node)[0];
  if (kind !== "tabs") return html`<div class="tree tree-${kind}">${children.map((child) => tree(child, list))}</div>`;
  const shown = shownTab(children, list);
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
const FOCUS_ATTRIBUTES = ["id", "href", "data-option", "data-pick-mode", "data-theme-pick", "data-add-chip", "data-key", "data-action"];
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
  if (state.reportPending && !state.reportPending.sending && (!state.token || current() !== "reports")) {
    clearTimeout(state.reportPending.timer);
    state.reportPending = null;
  }
  // Leaving the page closes an entry's menu and disarms its Drop.
  if (state.renderedKey && state.renderedKey !== routeKey()) Object.assign(state.planEdit, { open: null, armed: null });
  if (state.renderedKey) state.scrolls.set(state.renderedKey, [...app.querySelectorAll(".scroll")].map((element) => element.scrollTop));
  const focused = focusKey(document.activeElement);
  const typed = app.querySelector("#token")?.value;
  const field = ["draft", "report-draft", "report-query"].includes(document.activeElement?.id) ? document.activeElement : null;
  const caret = field ? [field.id, field.selectionStart, field.selectionEnd, field.selectionDirection] : undefined;
  state.observers.forEach((observer) => observer.disconnect());
  state.observers = [];
  // A new revision is a new question: a tap armed on the old one must not run it.
  if (state.armed && (!state.token || selectedAsk() !== String(state.armed.id) || askOnScreen()?.revision !== state.armed.revision)) disarm();
  // Likewise a Cancel armed on another runner's task at a generation the list no longer shows.
  if (typeof state.taskArmed?.task === "string" && remoteGeneration(state.taskArmed.task) !== state.taskArmed.generation) disarmTask();
  if (state.memoryArmed && current() !== "memories") disarmMemory();
  app.innerHTML = page().text;
  document.title = state.token && state.data ? pageTitle(location.pathname, { tasks: [...state.data.tasks, state.task?.body].filter(Boolean) }) : "Priory";
  paintTerminal(app, true);
  observeTerminal(app);
  // A render must not restart the countdown; CSP allows no inline style, so it is set here.
  app.querySelectorAll(".drain").forEach((drain) => {
    const armed = drain.closest("[data-action=memory-act]") ? state.memoryArmed : drain.closest(".act") ? state.taskArmed : state.armed;
    drain.style.animationDelay = `-${armed ? Date.now() - armed.at : 0}ms`;
  });
  state.renderedKey = routeKey();
  state.rendered = Date.now();
  const scrolls = state.scrolls.get(state.renderedKey) || [];
  app.querySelectorAll(".scroll").forEach((element, i) => { element.scrollTop = scrolls[i] || 0; });
  if (typed && app.querySelector("#token")) app.querySelector("#token").value = typed;
  // The Desk table draws some controls twice and CSS hides one copy: focus goes to the shown one.
  if (focused) [...app.querySelectorAll(focused)].find((element) => !element.closest("[inert]") && element.getClientRects().length)?.focus({ preventScroll: true });
  if (caret !== undefined) app.querySelector(`#${caret[0]}`)?.setSelectionRange(caret[1], caret[2], caret[3]);
  // A link or control inside a row, such as its key, batch or Move menu, acts on its own.
  for (const row of app.querySelectorAll("[data-href]")) row.addEventListener("click", (event) => { if (!event.target.closest("a, button, details")) go(row.dataset.href); });
  app.querySelectorAll("[data-grow]").forEach((part) => { part.style.flexGrow = part.dataset.grow; });
  drawLanes(app);
  watchSections(app);
  if (app.querySelector("[data-since]")) seen();
  // A render may call this; the read renders again once it ends.
  const spend = state.data?.spend;
  const old = !spend || Date.now() - spend.updated > 60000;
  if (app.querySelector("[data-costs]") && old && !state.failed && !state.costs.loading && !state.costs.error) setTimeout(loadCosts);
  // The sheet lives outside #app; it redraws when the connection drops or comes back under it.
  if (addSheet().open && state.add.shownOffline !== Boolean(state.failed)) renderAdd();
  if (answerSheet().open) renderSheet();
}

function page() {
  if (!state.token) return signIn();
  if (!state.data) return loading();
  const size = layout();
  const here = current();
  if (["desk", "asks"].includes(here) && !selectedAsk()) Object.assign(state, { list: here, listUrl: routeKey() });
  const taskId = here === "tasks" && /^\d+$/.test(route()[1] || "") ? Number(route()[1]) : null;
  const batchId = here === "batches" && /^\d+$/.test(route()[1] || "") ? route()[1] : null;
  if (PANELS[here] && !selectedAsk() && !taskId) state.from = routeKey();
  // A detail page keeps the list it was opened from in its history entry, so Back restores its link.
  else if ((taskId || batchId) && !history.state?.from) history.replaceState({ ...history.state, from: state.from }, "");
  if (size === "compact") {
    let body;
    if (selectedAsk()) body = askDetail(selectedAsk(), false);
    else if (here === "desk") body = deskPhone();
    else if (here === "asks") body = asksPhone();
    else if (OWN_PAGE_LISTS[here] && !taskId) {
      const { views, rows } = OWN_PAGE_LISTS[here](false);
      body = html`<div class="top"><h1>${PANELS[here][0]}</h1><span class="sp"></span>${connection()}</div>
        <div class="panel" role="region" aria-label="${PANELS[here][0]}"><div class="scroll">${views}${rows}</div></div>`;
    } else if (here === "inbox") body = html`<div class="top"><h1>Inbox</h1><span class="sp"></span>${connection()}</div>
      <div class="panel" role="region" aria-label="Inbox"><div class="scroll">${inboxViews()}${unroutedRows()}</div></div>`;
    else if (here === "more") body = morePage();
    else if (taskId) body = taskPage(taskId);
    else if (batchId) body = batchPage(batchId);
    else if (here === "memories") body = memoriesPage();
    else if (here === "reports") body = reportsPage(true);
    else if (here === "costs") body = costsPage(true);
    else if (here === "workflow") body = flowsPage(true);
    else if (here === "settings") body = html`<div class="top"><a class="back" href="/more">${icon("chevron-left", "s22")}More</a></div>${settingsScreen()}`;
    else if (PANELS[here]) body = html`<div class="top"><h1>${PANELS[here][0]}</h1></div>${laterPanel(here)}`;
    else body = notFound();
    const tab = selectedAsk() ? null : here === "settings" ? "more" : batchId ? "tasks" : here === "memories" ? "desk" : here;
    return html`${staleStrip()}${body}${tab ? tabs(tab, "bar") : ""}`;
  }
  const list = selectedAsk() ? state.list : here;
  const top = html`<header class="wtop"><span class="fac">${state.data.views.factory}</span>${connection()}<span class="sp"></span>${addButton("addtop")}</header>`;
  const wide = size === "wide";
  const utility = taskId ? taskPage(taskId) : batchId ? batchPage(batchId) : here === "memories" ? memoriesPage() : here === "reports" ? reportsPage(false) : here === "costs" ? costsPage(false) : here === "workflow" ? flowsPage(false) : here === "settings" ? settingsScreen() : here === "more" ? morePage() : PANELS[list] ? null : notFound();
  const nav = (kind) => tabs(here === "settings" && wide ? "settings" : here === "settings" ? "more" : batchId ? "tasks" : here === "memories" ? "desk" : list, kind);
  if (utility) {
    return html`${staleStrip()}${top}<div class="${size} no-drawer">${wide ? nav("rail") : ""}<div class="list">${utility}</div></div>${wide ? "" : nav("bar")}`;
  }
  if (OWN_PAGE_LISTS[list]) return html`${staleStrip()}${top}<div class="${size} no-drawer">${wide ? nav("rail") : ""}${listPanel(list, wide)}</div>${wide ? "" : nav("bar")}`;
  if (!wide) return html`${staleStrip()}${top}<div class="medium">${listPanel(list, false)}${drawer(false, [list])}</div>${nav("bar")}`;
  const layoutNode = view().layout;
  const plain = layoutNode.tabs?.every((child) => child.panel);
  const main = plain ? listPanel(list, true) : tree(layoutNode, list);
  return html`${staleStrip()}${top}<div class="wide">${nav("rail")}${main}${drawer(true, plain ? [list] : shownPanels(layoutNode, list))}</div>`;
}

const notFound = () => html`<div class="top"><h1>Page not found</h1></div><div class="scroll"><div class="empty notfound"><p>No page lives at this address.</p><a class="btn secondary" href="/">Open the Desk</a></div></div>`;

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
    let shown = null;
    const light = () => {
      const top = scroller.getBoundingClientRect().top - scroller.scrollTop;
      const tops = sections.map((section) => section.getBoundingClientRect().top - top);
      // A chip lands its section under the chip bar, at the section's scroll-margin-top.
      const bar = sections.length ? parseFloat(getComputedStyle(sections[0]).scrollMarginTop) || 0 : 0;
      const lit = sections[sectionInView({ tops, scrollTop: scroller.scrollTop, height: scroller.clientHeight, scrollHeight: scroller.scrollHeight, bar })];
      if (!lit || lit.id === shown) return;
      shown = lit.id;
      chips.forEach((chip) => chip.setAttribute("aria-current", String(chip.hash === `#${lit.id}`)));
      // On a phone the chips scroll sideways: keep the lit one in view.
      const chip = chips.find((each) => each.hash === `#${lit.id}`);
      const strip = chip?.parentElement;
      if (strip && strip.scrollWidth > strip.clientWidth) strip.scrollLeft = chip.offsetLeft - (strip.clientWidth - chip.offsetWidth) / 2;
    };
    if (chips.length) {
      scroller.addEventListener("scroll", light, { passive: true });
      light();
    }
    const options = scroller.querySelector(".opts");
    const bar = app.querySelector(".abar");
    if (!options || !bar) continue;
    track(new IntersectionObserver(([entry]) => { bar.hidden = entry.isIntersecting; }, { root: scroller })).observe(options);
  }
}

// Records the visit once, after Since you looked is drawn, at the time the first live read began.
async function seen() {
  if (state.since.sent || !state.since.opened || state.failed) return;
  state.since.sent = true;
  // As the Herdr Desk: one try. A visit not recorded shows the same events again next time.
  const { status, body } = await post("/api/seen", { at: state.since.opened }).catch((error) => ({ status: 0, body: { error: error.message } }));
  if (status !== 200) console.warn(`priory: could not record the visit: ${body.error || status}`);
}

// Events
document.addEventListener("click", (event) => {
  const target = event.target.closest("a, button, [data-action]");
  if (!target) return;
  const action = target.dataset.action;
  if (action === "retry") return refresh();
  if (action === "term-fit") return (terminal.fit = !terminal.fit), paintTerminal(document.getElementById("app"));
  if (action === "term-copy") return watch.state().screen && copyTerminal(screenText(watch.state().screen), "screen");
  if (action === "term-earlier") return watch.earlier();
  if (action === "term-type") {
    Object.assign(terminal, { typing: !terminal.typing, said: null });
    paintTerminal(document.getElementById("app"), true);
    return terminal.typing && document.getElementById("term-line")?.focus();
  }
  if (action === "term-attach") return copyTerminal(`priory attach ${target.dataset.task} --read-only`, "attach");
  if (action === "term-menu") return (terminal.menu = !terminal.menu), paintTerminal(document.getElementById("app"), true);
  if (action === "term-take-ask" || action === "term-give-ask") {
    terminal.menu = false;
    paintTerminal(document.getElementById("app"), true);
    return openKeyboardSheet(action === "term-take-ask" ? "take" : "give");
  }
  if (action === "term-take" || action === "term-give") return keyboardAct(action);
  if (action === "keyboard-close") return keyboardSheet().close();
  if (action === "term-key") return sendInput({ key: target.dataset.key });
  // A refresh that reaches the daemon reads the costs again.
  if (action === "costs-retry") return (state.costs.error = null), refresh();
  if (action === "add-open") return openAdd();
  if (action === "add-close" || action === "add-done") return closeAdd();
  if (action === "sheet-close") return answerSheet().close();
  if (action === "add-discard") return discardAdd();
  if (action === "add-another") {
    Object.assign(state.add, { phase: "empty", task: null });
    renderAdd();
    return document.getElementById("add-text")?.focus();
  }
  if (action === "flows-retry") { state.flowsError = null; return render(); }
  if (action === "sign-out") { event.preventDefault(); return signOut(null); }
  if (action === "push-on" || action === "push-off") return turnPush(action === "push-on");
  if (action === "install" && state.install) {
    state.install.prompt();
    state.install = null;
    return render();
  }
  if (action === "task-act") return taskAct(target);
  if (action === "remote-open" && !event.target.closest("form, .task-acts")) {
    state.remoteOpen = state.remoteOpen === target.dataset.task ? null : target.dataset.task;
    return render();
  }
  if (action === "entry-menu" && state.batch?.body) {
    const edit = planEditOf(state.batch.body);
    const open = edit.open === target.dataset.entry ? null : target.dataset.entry;
    state.planEdit = { ...edit, at: planEditAt(state.batch.body), batch: state.batch.body.id, open, armed: null };
    return render();
  }
  if (action === "batch-edit") return editPlan(target);
  if (action === "todo-done") return markDone(target);
  if (action === "row-act") return rowAct(target);
  if (action === "memory-act") return memoryAct(target);
  if (action === "memory-current") {
    event.preventDefault();
    const { id } = target.dataset;
    if (!state.memoryCurrent.delete(id)) state.memoryCurrent.add(id);
    return render();
  }
  if (action === "move") return moveTask(target);
  if (action === "request-task") return requestTask(target.dataset.text, target.dataset.workflow);
  if (action === "unheard-say") {
    if (target.getAttribute("aria-disabled") !== "true") taskWrite(Number(target.dataset.task), "say", { text: target.dataset.text });
    return;
  }
  if (action === "subscribe-task") {
    if (state.unrouted.busy === null) subscribeTask(target.dataset.resource, Number(target.dataset.task));
    return;
  }
  if (["pick", "pick-undo", "pick-change", "pick-keep", "report-go", "report-next", "report-search", "compose", "compose-cancel", "reply", "resolve", "reopen"].includes(action)) return reportAct(target);
  if (target.dataset.answer !== undefined || ["undo", "options", "write", "dismiss", "snooze", "unsnooze"].includes(action)) return act(target);
  if (action === "mode" || action === "theme") {
    const root = document.documentElement;
    if (action === "mode") root.dataset.mode = target.dataset.pickMode;
    else root.dataset.theme = target.dataset.themePick;
    localStorage.setItem("priory.mode", root.dataset.mode);
    localStorage.setItem("priory.theme", root.dataset.theme);
    return render();
  }
  const modified = event.metaKey || event.ctrlKey || event.shiftKey || event.altKey;
  if (target.tagName === "A" && target.origin === location.origin && !target.hash && !target.target && !modified) {
    event.preventDefault();
    go(target.pathname + target.search);
  }
  const inPage = target.origin === location.origin && target.pathname === location.pathname && target.search === location.search;
  if (target.tagName === "A" && inPage && target.hash && !target.target && !modified) {
    event.preventDefault();
    document.querySelector(target.hash)?.scrollIntoView({ behavior: matchMedia("(prefers-reduced-motion: reduce)").matches ? "auto" : "smooth" });
  }
});

// A Move menu's toggle does not bubble, so it is caught on its way down.
document.addEventListener("toggle", (event) => {
  if (!event.target.matches?.("details.move")) return;
  const task = Number(event.target.dataset.move);
  if (event.target.open) state.moveOpen = task;
  else if (state.moveOpen === task) state.moveOpen = null;
}, true);

document.addEventListener("change", (event) => {
  if (event.target.dataset.action !== "costs-by") return;
  state.costs.by = event.target.value;
  loadCosts();
});

document.addEventListener("input", (event) => {
  if (event.target.id === "add-text" || event.target.dataset.addChip) {
    state.add.draft[event.target.dataset.addChip || "text"] = event.target.value;
    keepDraft();
    // Only the button's state changes; a full redraw would reset the caret on every letter.
    addSheet().querySelector("form .btn.primary")?.setAttribute("aria-disabled", String(!state.add.draft.text.trim() || state.add.phase === "sending"));
    return;
  }
  const ask = event.target.id === "draft" && askOnScreen();
  if (ask) state.drafts.set(ask.id, event.target.value);
  if (event.target.id === "report-draft" && state.compose) state.compose.text = event.target.value;
  if (event.target.id === "report-query") {
    state.reportQuery = event.target.value;
    render();
  }
  const form = event.target.id === "say" && event.target.closest("form");
  if (form) state.sayDrafts.set(taskIdOf(form.dataset.task), event.target.value);
  if (event.target.id === "term-keys") terminal.keys = event.target.value;
  if (event.target.id === "term-line") {
    terminal.line = event.target.value;
    // Only the button changes; a repaint per letter would cost the caret.
    event.target.form.querySelector(".send").setAttribute("aria-disabled", String(!terminal.line.trim() || terminal.saying));
  }
});

document.addEventListener("submit", (event) => {
  if (event.target.dataset.action === "add") {
    event.preventDefault();
    return submitAdd();
  }
  if (event.target.dataset.action === "send-text") {
    event.preventDefault();
    const ask = askOnScreen();
    const text = (state.drafts.get(ask?.id) || "").trim();
    if (ask && text) answer(ask, text, true);
    return;
  }
  if (event.target.dataset.action === "remark") {
    event.preventDefault();
    return submitRemark();
  }
  if (event.target.dataset.action === "term-paste") {
    event.preventDefault();
    return terminal.keys && sendInput({ text: terminal.keys });
  }
  if (event.target.dataset.action === "term-say") {
    event.preventDefault();
    return sayFromPane(Number(event.target.dataset.task));
  }
  if (event.target.dataset.action === "say") {
    event.preventDefault();
    const task = taskIdOf(event.target.dataset.task);
    const text = (state.sayDrafts.get(task) || "").trim();
    if (text && state.taskBusy === null) taskWrite(task, "say", remoteBody(task, { text }));
    return;
  }
  if (event.target.dataset.action !== "sign-in") return;
  event.preventDefault();
  const token = new FormData(event.target).get("token").trim();
  if (!token) return;
  localStorage.setItem("priory.token", token);
  keepTokenForWorker(token);
  Object.assign(state, { token, signInError: null });
  refresh();
});

// Desktop shortcuts: j and k move, Enter opens, Esc closes, S snoozes the selected ask for 1 h. Never inside an input.
document.addEventListener("keydown", (event) => {
  if (event.key === "Escape" && terminal.menu && event.target.closest?.(".term")) {
    terminal.menu = false;
    paintTerminal(document.getElementById("app"), true);
    return document.querySelector('[data-action="term-menu"]')?.focus();
  }
  if (event.key === "Escape" && event.target.id === "term-line") {
    Object.assign(terminal, { typing: false, said: null });
    paintTerminal(document.getElementById("app"), true);
    return document.querySelector('[data-action="term-type"]')?.focus();
  }
  if ((event.key === " " || event.key === "Enter") && event.target.matches?.("[role=radio]")) {
    event.preventDefault();
    return event.target.click();
  }
  if (addSheet().open || answerSheet().open || keyboardSheet().open || event.target.closest("input, textarea, select") || event.metaKey || event.ctrlKey || event.altKey || !state.data) return;
  const rows = [...document.querySelectorAll(".list [href^='/asks/'], .list tr[data-href^='/asks/']")];
  const hrefOf = (row) => row.getAttribute("href") || row.dataset.href;
  const at = rows.findIndex((row) => row.getAttribute("aria-current") === "true");
  if (event.key === "n" || event.key === "N") {
    event.preventDefault();
    openAdd();
  } else if (event.key === "s" || event.key === "S") {
    const body = askOnScreen() && !state.failed && !state.pending && snoozeShortcut(askOnScreen(), Date.now());
    if (body) snooze(askOnScreen(), { dataset: { for: body.for } });
  } else if (selectedAsk() && /^[1-9]$/.test(event.key)) {
    document.querySelector(`.opt[data-answer="${Number(event.key) - 1}"]`)?.click();
  } else if (event.key === "j" || event.key === "k") {
    const next = rows[Math.min(rows.length - 1, Math.max(0, at + (event.key === "j" ? 1 : -1)))];
    if (next) go(hrefOf(next));
  } else if (event.key === "Enter" && at >= 0 && document.activeElement === document.body) {
    go(hrefOf(rows[at]));
  } else if (event.key === "Escape" && document.querySelector("details.move[open]")) {
    const menu = document.querySelector("details.move[open]");
    menu.open = false;
    menu.querySelector("summary").focus();
  } else if (event.key === "Escape" && selectedAsk()) {
    go(state.listUrl);
  }
});

// A tap on the scrim, outside the sheet's box, closes it; closing hands focus back to an Add button.
addSheet().addEventListener("click", (event) => { if (event.target === addSheet()) closeAdd(); });
answerSheet().addEventListener("click", (event) => { if (event.target === answerSheet()) answerSheet().close(); });
// A page that goes away gives its keyboard back, so the run's outbox does not wait out the 15 minutes.
addEventListener("pagehide", () => keyboard.leave());
keyboardSheet().addEventListener("click", (event) => { if (event.target === keyboardSheet()) keyboardSheet().close(); });
keyboardSheet().addEventListener("close", () => document.querySelector("#app [data-action='term-menu']")?.focus());
addSheet().addEventListener("close", () => document.querySelector("#app [data-action='add-open']")?.focus());
addEventListener("popstate", render);
for (const query of ["(min-width: 600px)", "(min-width: 1024px)"]) matchMedia(query).addEventListener("change", render);
addEventListener("resize", () => { if (terminal.fit) paintTerminal(document.getElementById("app")); });
document.addEventListener("visibilitychange", () => {
  syncTerminal(document.getElementById("app"));
  if (!document.hidden && state.token) refresh();
});
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
      if (revision !== state.revision) watch.revision();
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
// The browser offers to install only once the page handles this; the Notifications screen shows it.
addEventListener("beforeinstallprompt", (event) => {
  event.preventDefault();
  state.install = event;
  render();
});
if ("serviceWorker" in navigator) navigator.serviceWorker.register("/sw.js").catch(() => {});
follow();
refresh();
