// The strip, the docked panel and the side panel: every daemon call and all agent text live here.
import { ARM_MS, UNDO_MS, age, answerBody, answerPath, choose, due, leftUnsent, observe, optionKey, refusal, reply, runsCommand, undo } from "./lib/ask.js";
import { grade, parseGuide, scoreKey } from "./lib/guide.js";
import { live } from "./lib/live.js";
import { parsePr } from "./lib/pr.js";
import { address, call } from "./lib/request.js";
import { get, put, remove } from "./lib/store.js";
import { fixBranch, fixText, sayRoute } from "./lib/task.js";

const params = new URLSearchParams(location.search);
// command.html is not web-accessible, so no page can embed its Run button.
const VIEW = location.pathname === "/command.html" ? "command" : params.get("view") === "panel" ? "panel" : "strip";
const MODE = { light: "light", dark: "dark" }[params.get("mode")] ?? "system";
document.documentElement.dataset.mode = MODE;

const S = {
  config: null,
  tabId: null,
  pr: null,
  holders: null,
  task: null,
  flows: null,
  readAt: null,
  unreachable: false,
  tab: params.get("tab"),
  draft: null,
  index: 0,
  writing: false,
  written: "",
  say: "",
  sayNotice: null,
  sayBusy: false,
  connect: { address: "", token: "", error: null, busy: false },
  guide: { state: "loading" },
  quiz: { answers: {}, result: null, key: null },
};


const ICONS = {
  check: '<path d="M20 6 9 17l-5-5"/>',
  x: '<path d="M18 6 6 18"/><path d="m6 6 12 12"/>',
  ask: '<path d="M7.9 20A9 9 0 1 0 4 16.1L2 22Z"/><path d="M9.09 9a3 3 0 0 1 5.83 1c0 2-3 3-3 3"/><path d="M12 17h.01"/>',
  moon: '<path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z"/>',
  dot: '<circle cx="12" cy="12" r="10"/><circle cx="12" cy="12" r="1"/>',
  stop: '<circle cx="12" cy="12" r="10"/><path d="m15 9-6 6"/><path d="m9 9 6 6"/>',
  lock: '<rect width="18" height="11" x="3" y="11" rx="2"/><path d="M7 11V7a5 5 0 0 1 10 0v4"/>',
  key: '<circle cx="7.5" cy="15.5" r="5.5"/><path d="m21 2-9.6 9.6"/><path d="m15.5 7.5 3 3L22 7l-3-3"/>',
  panel: '<rect width="18" height="18" x="3" y="3" rx="2"/><path d="M15 3v18"/>',
  ext: '<path d="M15 3h6v6"/><path d="M10 14 21 3"/><path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/>',
  corner: '<polyline points="15 10 20 15 15 20"/><path d="M4 4v7a4 4 0 0 0 4 4h12"/>',
  unplug: '<path d="m19 5 3-3"/><path d="m2 22 3-3"/><path d="M6.3 20.3a2.4 2.4 0 0 0 3.4 0L12 18l-6-6-2.3 2.3a2.4 2.4 0 0 0 0 3.4Z"/><path d="m12 6 6 6 2.3-2.3a2.4 2.4 0 0 0 0-3.4l-2.6-2.6a2.4 2.4 0 0 0-3.4 0Z"/>',
  refresh: '<path d="M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8"/><path d="M21 3v5h-5"/><path d="M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16"/><path d="M8 16H3v5"/>',
  merge: '<circle cx="18" cy="18" r="3"/><circle cx="6" cy="6" r="3"/><path d="M6 21V9a9 9 0 0 0 9 9"/>',
  book: '<path d="M12 7v14"/><path d="M3 18a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1h5a4 4 0 0 1 4 4 4 4 0 0 1 4-4h5a1 1 0 0 1 1 1v13a1 1 0 0 1-1 1h-6a3 3 0 0 0-3 3 3 3 0 0 0-3-3z"/>',
  scale: '<path d="M7 21h10"/><path d="M12 3v18"/><path d="M3 7h2c2 0 5-1 7-2 2 1 5 2 7 2h2"/>',
  hourglass: '<path d="M5 22h14"/><path d="M5 2h14"/><path d="M17 22v-4.172a2 2 0 0 0-.586-1.414L12 12l-4.414 4.414A2 2 0 0 0 7 17.828V22"/><path d="M7 2v4.172a2 2 0 0 0 .586 1.414L12 12l4.414-4.414A2 2 0 0 0 17 6.172V2"/>',
  pencil: '<path d="M21.2 6.8a2.8 2.8 0 0 0-4-4L3.8 16.2a2 2 0 0 0-.5.8l-1.3 4.4a.5.5 0 0 0 .6.6l4.4-1.3a2 2 0 0 0 .8-.5z"/>',
};
const SVG = "http://www.w3.org/2000/svg";

function icon(name, size = "") {
  const svg = document.createElementNS(SVG, "svg");
  svg.setAttribute("class", `ico ${size}`);
  svg.setAttribute("viewBox", "0 0 24 24");
  svg.setAttribute("aria-hidden", "true");
  svg.append(...new DOMParser().parseFromString(`<svg xmlns="${SVG}">${ICONS[name]}</svg>`, "image/svg+xml").documentElement.childNodes);
  return svg;
}

function mark() {
  const span = h("span", { class: "mark" });
  const svg = document.createElementNS(SVG, "svg");
  svg.setAttribute("viewBox", "0 0 32 32");
  for (const [part, d] of [["book", "M6 2h16a2 2 0 0 1 2 2v18H6Z M8 24h8v4H8Z M10 6h6a4 4 0 0 1 0 8h-3v4h-3Z M13 9h3a1 1 0 0 1 0 2h-3Z"], ["ribbon", "M18 20h4v11l-2-2.5L18 31Z"]]) {
    const path = document.createElementNS(SVG, "path");
    path.setAttribute("class", part);
    path.setAttribute("fill", "currentColor");
    path.setAttribute("fill-rule", "evenodd");
    path.setAttribute("d", d);
    svg.append(path);
  }
  span.append(svg);
  return span;
}

function h(tag, props = {}, ...kids) {
  const element = document.createElement(tag);
  for (const [name, value] of Object.entries(props)) {
    if (name.startsWith("on")) element.addEventListener(name.slice(2), value);
    else if (name === "value") element.value = value;
    else if (value === true) element.setAttribute(name, "");
    else if (value !== false && value != null) element.setAttribute(name, value);
  }
  for (const kid of kids.flat(Infinity)) if (kid != null && kid !== false) element.append(kid instanceof Node ? kid : String(kid));
  return element;
}

const TILES = { running: ["live", "dot"], asleep: ["", "moon"], ready: ["live", "dot"], queued: ["", "dot"], blocked: ["warn", "stop"], held: ["hold", "lock"], failed: ["danger", "stop"], done: ["ok", "check"], dropped: ["", "stop"] };
const tile = (task, waits) => {
  const [kind, name] = waits ? ["wait", "ask"] : TILES[task.state] ?? ["", "dot"];
  return h("span", { class: `tile ${kind}`, title: task.state }, icon(name));
};


const openAsks = () => (S.task?.asks ?? []).filter((ask) => ask.state === "open").sort((a, b) => a.since.localeCompare(b.since));
const waiting = () => openAsks().filter((ask) => !(ask.snoozed_until && Date.parse(ask.snoozed_until) > Date.now()));
const api = (path, body, timeout) => call(fetch, S.config.origin, S.config.token, path, body, timeout);

// `quiet`, for a revision from the poller: a read that changed nothing leaves the view as it is.
async function read(quiet = false) {
  if (!S.config || !S.pr) return render();
  const found = await api(`/api/tasks?pr=${encodeURIComponent(S.pr.resource)}`);
  if (found.status === 401) return signedOut();
  if (found.status !== 200) {
    S.unreachable = true;
    return render();
  }
  const holders = found.body.tasks;
  const detail = holders[0] ? await api(`/api/tasks/${holders[0].id}`) : { status: 200, body: null };
  if (!S.flows) {
    const flows = await api("/api/flows");
    S.flows = flows.status === 200 ? flows.body : null;
  }
  if (detail.status !== 200) {
    S.unreachable = true;
    return render();
  }
  const seen = JSON.stringify([holders, detail.body]);
  S.readAt = Date.now();
  if (seen === S.seen && !S.unreachable) return quiet ? undefined : render();
  Object.assign(S, { seen, holders, task: detail.body, unreachable: false });
  for (const ask of S.task?.asks ?? []) S.draft = observe(S.draft, ask, Date.now());
  if ((S.draft?.phase === "sent" || S.draft?.restored) && !S.task?.asks?.some((ask) => ask.id === S.draft.id && ask.state === "open")) S.draft = null;
  put(`last:${S.pr.resource}`, { holders, task: S.task, flows: S.flows, readAt: S.readAt });
  wakeAtSnoozeEnd();
  render();
}

let snoozeTimer = 0;
function wakeAtSnoozeEnd() {
  clearTimeout(snoozeTimer);
  const ends = openAsks().map((ask) => Date.parse(ask.snoozed_until)).filter((at) => at > Date.now());
  if (ends.length) snoozeTimer = setTimeout(read, Math.min(...ends) - Date.now() + 500);
}

const channel = new BroadcastChannel("priory-events");

/** Forgets the token here, and with `share` in every other frame too. */
async function signedOut(share = true) {
  await remove("config");
  S.config = null;
  S.draft = null;
  if (share) channel.postMessage({ signedOut: true });
  render();
}

const send = (type, extra = {}) => S.tabId != null && chrome.tabs.sendMessage(S.tabId, { type, ...extra }).catch(() => {});


let undoTimer = 0;
function pick(ask, answer) {
  const before = S.draft;
  S.draft = choose(S.draft, ask, answer, Date.now());
  if (S.draft === before) return;
  if (S.draft.phase === "armed") return openCommand(ask, answer.option);
  clearTimeout(undoTimer);
  undoTimer = setTimeout(post, UNDO_MS);
  render();
}

async function post() {
  const next = due(S.draft, Date.now());
  if (!next) return;
  S.draft = next.draft;
  render();
  const result = await api(next.path, next.body);
  S.draft = reply(S.draft, result, Date.now());
  if (S.draft.phase === "signin") return signedOut();
  if (S.draft.phase === "sent") S.written = "";
  await read();
}

// The run happens in the browser's side panel, which no page can cover.
function openCommand(ask, option) {
  const path = `command.html?mode=${MODE}&at=${Date.now()}&ask=${encodeURIComponent(ask.id)}&task=${S.task.id}&revision=${ask.revision}&option=${encodeURIComponent(option)}`;
  chrome.sidePanel.setOptions({ tabId: S.tabId, path, enabled: true });
  chrome.sidePanel.open({ tabId: S.tabId }).catch(() => {});
  setTimeout(() => {
    S.draft = observe(S.draft, ask, Date.now());
    render();
  }, ARM_MS);
  render();
}

/** Null once the daemon applied the write, else its refusal as [phase, notice]. */
async function write(path, body) {
  const result = await api(path, body);
  if (result.status === 200) return null;
  if (result.status === 401) await signedOut();
  return refusal(result, Date.now());
}

// A lost say keeps Say off until the person sends it again or checks the task: a resend would queue it twice.
async function say(text, keep, task = S.task.id) {
  if (S.sayBusy) return;
  S.sayBusy = true;
  S.sayNotice = null;
  S.sayUnknown = null;
  render();
  const refused = await write(`/api/tasks/${task}/say`, { text });
  S.sayBusy = false;
  S.sayNotice = refused?.[1] ?? null;
  if (refused?.[0] === "unknown") S.sayUnknown = { task, text };
  if (!refused && keep) S.say = "";
  await read();
}

async function addTask() {
  S.sayBusy = true;
  render();
  const result = await api("/api/request", { text: `Take up ${S.pr.url}` });
  S.sayBusy = false;
  S.sayNotice = result.status === 200 ? null : result.status === 409 ? (result.body.id ? null : "Already sent. No active task holds it now.") : refusal(result, Date.now())[1];
  await read();
}


function strip() {
  const toggle = (tab) => h("button", { class: "btn icon", title: "Open the Priory panel", onclick: () => send("open", { tab }) }, icon("panel"));
  const brand = h("span", { class: "brand" }, mark(), "Priory");
  if (!S.config) {
    return h("div", { class: "strip" }, brand, h("span", { class: "t2" }, "Sign in to see the task that owns this pull request."),
      h("button", { class: "btn primary", onclick: () => send("open", { tab: "task" }) }, "Sign in"), h("span", { class: "sp" }), toggle("task"));
  }
  if (S.holders == null && !S.unreachable) return h("div", { class: "strip" }, brand, h("span", { class: "t3" }, "Reading Priory…"));
  const stale = S.unreachable
    ? h("span", { class: "warn" }, icon("unplug", "s14"), S.readAt ? `Priory unreachable. Last read ${age(new Date(S.readAt).toISOString(), Date.now())} ago.` : "Priory unreachable.")
    : null;
  if (!S.task) {
    return h("div", { class: "strip" }, brand, stale ?? h("span", { class: "t2" }, "No Priory task owns this pull request."),
      S.holders?.length === 0 ? h("button", { class: "btn ghost", onclick: () => send("open", { tab: "task" }) }, "Add a task…") : null, h("span", { class: "sp" }), toggle("task"));
  }
  const asks = waiting();
  return h("div", { class: "strip" }, brand, tile(S.task, asks.length > 0), h("span", { class: "mono" }, `task ${S.task.id}`),
    h("span", { class: "t2" }, `${S.task.state} at `, h("b", {}, S.task.step)),
    asks.length ? h("button", { class: "waitpill", onclick: () => send("open", { tab: "ask" }) }, icon("ask", "s14"), `${asks.length} ${asks.length > 1 ? "asks wait" : "ask waits"} on you`) : null,
    stale ?? (S.task.since ? h("span", { class: "t3" }, age(S.task.since, Date.now())) : null), h("span", { class: "sp" }), toggle(asks.length ? "ask" : "task"));
}


function panel() {
  const asks = openAsks();
  const guideOn = S.guide.state !== "none";
  const tabs = [["ask", "Ask", asks.length], ["task", "Task", 0], ...(guideOn ? [["guide", "Guide", 0]] : [])];
  S.tab = tabs.some(([id]) => id === S.tab) ? S.tab : asks.length ? "ask" : "task";
  const conn = !S.config ? h("span", { class: "conn" }, "signed out")
    : S.unreachable ? h("span", { class: "conn off" }, "unreachable")
      : h("span", { class: "conn live" }, h("span", { class: "livedot" }), "live");
  const head = h("div", { class: "head" }, mark(), h("span", { class: "name" }, "Priory"), conn,
    S.config && S.task ? h("a", { class: "btn icon", title: `Open task ${S.task.id} in Priory`, href: `${S.config.origin}/tasks/${S.task.id}`, target: "_blank", rel: "noreferrer" }, icon("ext")) : null,
    h("button", { class: "btn icon", title: "Close", onclick: () => send("close") }, icon("x")));
  const nav = h("nav", { class: "tabs", role: "tablist" }, tabs.map(([id, label, count]) =>
    h("button", { role: "tab", "aria-selected": String(S.tab === id), onclick: () => { S.tab = id; if (id === "guide") loadGuide(); render(); } }, label, count ? h("span", { class: "badge" }, count) : null)));
  const stale = S.config && S.unreachable
    ? h("div", { class: "stale" }, icon("unplug", "s14"), S.readAt ? `Priory unreachable. Showing what it said ${age(new Date(S.readAt).toISOString(), Date.now())} ago.` : "Priory unreachable.",
      h("button", { class: "btn secondary", onclick: () => read() }, "Retry"))
    : null;
  const body = S.tab === "guide" ? guideTab() : !S.config ? connectForm() : S.tab === "ask" ? askTab(asks) : taskTab();
  return h("div", { class: "panel" }, head, nav, stale, h("div", { class: "body" }, body));
}

function connectForm() {
  const c = S.connect;
  const submit = (event) => {
    event.preventDefault();
    const checked = address(c.address);
    if (checked.error) {
      c.error = checked.error;
      return render();
    }
    // Asked inside the click, which the permission prompt needs.
    chrome.permissions.request({ origins: [`${checked.origin}/*`] }).then((granted) => connect(checked.origin, granted));
  };
  return [
    h("div", { class: "q" }, "Connect to your factory"),
    h("div", { class: "read" }, "The extension reads and answers through your daemon's serve address. Your browser asks once to let it reach that address."),
    h("form", { class: "sect", onsubmit: submit },
      h("div", { class: "field" }, h("label", { for: "address" }, "Address"),
        h("input", { id: "address", class: "input", autocomplete: "off", placeholder: "https://xps13.tailnet.ts.net", value: c.address, oninput: (event) => { c.address = event.target.value; } })),
      h("div", { class: "field" }, h("label", { for: "token" }, "Token"),
        h("input", { id: "token", class: "input secret", type: "password", autocomplete: "off", value: c.token, oninput: (event) => { c.token = event.target.value; } }),
        h("div", { class: "note" }, icon("key", "s14"), h("span", {}, "The value of PRIORY_TOKEN on the daemon's machine."))),
      h("div", { class: "box" }, h("div", { class: "fact" }, icon("lock", "s14"), h("span", {}, h("b", {}, "Kept by the extension alone. "), "GitHub's pages, their scripts and other extensions cannot read it. It is sent only to this address, in a header."))),
      c.error ? h("div", { class: "notice", role: "alert" }, c.error) : null,
      h("div", { class: "end" }, h("button", { class: "btn primary", type: "submit", disabled: c.busy }, "Connect"))),
    h("div", { class: "hr" }),
    h("div", { class: "note" }, icon("book", "s14"), h("span", {}, "The Guide tab works without signing in: it reads the pull request's description from GitHub.")),
  ];
}

async function connect(origin, granted) {
  const c = S.connect;
  if (!granted) {
    c.error = `The browser did not let the extension reach ${origin}.`;
    return render();
  }
  c.busy = true;
  render();
  const checked = await call(fetch, origin, c.token.trim(), "/api/views");
  c.busy = false;
  if (checked.status !== 200) {
    c.error = checked.status === 401 ? "The daemon refused this token. Paste the current one." : `Could not reach priory serve at ${origin}. Check the address and that it runs.`;
    return render();
  }
  S.config = { origin, token: c.token.trim() };
  await put("config", S.config);
  S.connect = { address: "", token: "", error: null, busy: false };
  channel.postMessage({ signedIn: true });
  await read();
}

function askTab(asks) {
  if (!S.task) return h("div", { class: "read" }, "No Priory task owns this pull request, so nothing asks here.");
  if (!asks.length) return h("div", { class: "read" }, `No open asks. Task ${S.task.id}'s agent asks here when it needs you.`);
  const ask = asks[Math.min(S.index, asks.length - 1)];
  const draft = S.draft?.id === ask.id ? S.draft : null;
  const busy = S.draft && ["waiting", "posting"].includes(S.draft.phase);
  const blocked = S.unreachable || busy || draft?.phase === "unknown";
  const options = h("div", { class: "opts" }, ask.options.map((option, i) => {
    const recommended = option === ask.recommendation;
    const command = runsCommand(ask, option);
    const armed = draft?.phase === "armed" && draft.option === option;
    const chosen = draft && ["waiting", "posting"].includes(draft.phase) && draft.option === option;
    const sub = armed ? "Opened in the side panel: run it there" : [recommended ? "Recommended" : null, command ? "Runs a command" : null].filter(Boolean).join(". ");
    const cls = ["opt", recommended && !armed && !chosen ? "recd" : "", armed ? "armed" : "", chosen ? "chosen" : "", draft && !armed && !chosen && draft.phase !== "unknown" ? "dim" : ""].join(" ");
    return h("button", { class: cls, disabled: blocked && !chosen, onclick: () => pick(ask, { option }) }, h("span", { class: "k" }, i + 1), h("span", {}, option, sub ? h("span", { class: "sub" }, sub) : null));
  }));
  const position = asks.length > 1 ? h("div", { class: "row" }, h("span", { class: "t3" }, `${asks.indexOf(ask) + 1}/${asks.length}`),
    h("button", { class: "btn ghost", onclick: () => { S.index = Math.max(0, S.index - 1); render(); } }, "Previous"),
    h("button", { class: "btn ghost", onclick: () => { S.index = Math.min(asks.length - 1, S.index + 1); render(); } }, "Next")) : null;
  const snoozed = ask.snoozed_until && Date.parse(ask.snoozed_until) > Date.now();
  return [
    position,
    h("div", { class: "ctx" }, tile(S.task, true), h("span", { class: "mono" }, `task ${S.task.id}`), h("span", {}, ask.step), h("span", {}, S.task.lane), ask.command ? h("span", {}, "Command") : null, h("span", {}, age(ask.since, Date.now())), h("span", {}, `revision ${ask.revision}`)),
    h("div", { class: "q" }, ask.question),
    ask.routed_to ? h("div", { class: "routed" }, `${ask.routed_to} is answering. You can still answer first: the first answer stands.`) : null,
    snoozed ? h("div", { class: "note" }, `Snoozed until ${new Date(ask.snoozed_until).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}.`) : null,
    ask.recommendation ? h("div", { class: "rec" }, h("span", {}, h("b", {}, `Recommended: ${ask.recommendation}.`)), ask.next_action ? h("span", { class: "next" }, icon("corner", "s14"), h("span", {}, `Next: ${ask.next_action}`)) : null) : null,
    options,
    S.unreachable ? h("div", { class: "notice" }, "Reconnect to answer.") : null,
    draft?.phase === "armed" ? commandBlock(ask) : null,
    S.writing
      ? h("form", { class: "field", onsubmit: (event) => { event.preventDefault(); if (S.written.trim()) pick(ask, { text: S.written.trim(), again: draft?.phase === "unknown" }); } },
        h("textarea", { id: "written", class: "input", rows: 3, "aria-label": "Your answer", value: S.written, oninput: (event) => { S.written = event.target.value; } }),
        h("div", { class: "end" }, h("button", { class: "btn primary", type: "submit", disabled: blocked }, "Send")))
      : null,
    h("div", { class: "lowrow" },
      S.writing ? null : h("button", { class: "btn ghost", disabled: blocked, onclick: () => { S.writing = true; render(); } }, icon("pencil", "s14"), "Write an answer"),
      h("button", { class: "btn ghost", disabled: blocked, onclick: () => dismiss(ask) }, "Dismiss"),
      h("button", { class: "btn ghost", disabled: blocked, onclick: () => snooze(ask, snoozed) }, snoozed ? "Unsnooze" : "Snooze 1 h"),
      draft?.phase === "unknown"
        ? h("button", { class: "btn ghost", onclick: () => (draft.dismiss ? dismiss(ask) : pick(ask, { ...(draft.option === undefined ? { text: draft.text } : { option: draft.option }), again: true })) }, "Send again")
        : null,
      draft?.restored
        ? [h("button", { class: "btn ghost", onclick: () => pick(ask, draft.option === undefined ? { text: draft.text } : { option: draft.option }) }, "Send"),
          h("button", { class: "btn ghost", onclick: () => { S.draft = null; render(); } }, "Discard")]
        : null),
    draft?.notice ? h("div", { class: "notice", role: "alert" }, draft.notice) : null,
    h("div", { class: "hr" }),
    ask.text ? h("div", { class: "sect" }, h("h3", {}, "What the agent says"), h("div", { class: "read" }, ask.text)) : null,
    ask.tradeoff ? h("div", { class: "fact" }, icon("scale", "s14"), h("span", {}, h("b", {}, "Trade-off. "), ask.tradeoff)) : null,
    ask.what_waits ? h("div", { class: "fact" }, icon("hourglass", "s14"), h("span", {}, h("b", {}, "What waits. "), ask.what_waits)) : null,
    S.task.brief?.request ? h("div", { class: "sect" }, h("h3", {}, "Brief"), h("div", { class: "read" }, S.task.brief.request)) : null,
    draft?.phase === "waiting" || draft?.phase === "posting"
      ? h("div", { class: "undo", role: "status" }, `Sending "${draft.option ?? draft.text}"`, draft.phase === "waiting" ? h("button", { class: "btn", onclick: () => { clearTimeout(undoTimer); S.draft = undo(S.draft); render(); } }, "Undo") : null)
      : null,
  ];
}

function commandBlock(ask, revision = ask.revision) {
  return h("div", { class: "cmd" }, h("div", { class: "argv" }, ask.command.argv.map((word) => h("span", {}, word === "" ? "''" : word))),
    h("span", {}, `In ${ask.command.dir ?? ask.command.target}. Bound to revision ${revision}. Runs at most once.`));
}

async function dismiss(ask) {
  S.draft = { id: ask.id, revision: ask.revision, phase: "posting", notice: null };
  render();
  const refused = await write(`/api/asks/${encodeURIComponent(ask.id)}/dismiss`, { revision: ask.revision });
  S.draft = refused ? { ...S.draft, dismiss: true, phase: refused[0], notice: refused[1] } : null;
  await read();
}

async function snooze(ask, snoozed) {
  const refused = await write("/api/snooze", snoozed ? { kind: "ask", id: ask.id, clear: true } : { kind: "ask", id: ask.id, for: "1h" });
  S.draft = refused ? { id: ask.id, revision: ask.revision, phase: "unsent", notice: refused[1] } : S.draft;
  await read();
}

function taskTab() {
  if (!S.task) {
    return [
      h("div", { class: "q" }, "No Priory task owns this pull request"),
      S.holders?.length === 0
        ? [h("div", { class: "read" }, "Add a task sends a request with this pull request's address. The factory's [request] defaults pick its workflow."),
          h("div", { class: "end" }, h("button", { class: "btn primary", disabled: S.sayBusy || S.unreachable, onclick: addTask }, "Add a task for this PR"))]
        : h("div", { class: "read" }, "Priory could not say who holds it. Retry once it is reachable."),
      S.sayNotice ? h("div", { class: "notice", role: "alert" }, S.sayNotice) : null,
      signOut(),
    ];
  }
  const task = S.task;
  const route = sayRoute(task, S.flows);
  const fix = fixBranch(task, S.holders?.find((holder) => holder.id === task.id)?.holds, S.pr);
  const canSay = route.enabled && !S.sayBusy && !S.unreachable && !S.sayUnknown;
  const steps = S.flows?.workflows?.find((workflow) => workflow.name === task.workflow)?.steps ?? [];
  const at = steps.findIndex((step) => step.id === task.step);
  return [
    h("div", { class: "ctx" }, tile(task, waiting().length > 0), h("span", { class: "mono" }, `task ${task.id}`), h("span", {}, task.key), h("span", {}, task.step), h("span", {}, task.lane), task.band ? h("span", {}, task.band) : null),
    h("div", { class: "q" }, task.brief?.title ?? task.key),
    h("div", { class: "t2" }, `${task.state}${task.reason ? `: ${task.reason}` : ""}${task.since ? `, since ${age(task.since, Date.now())}` : ""}`),
    steps.length ? h("div", { class: "grid" }, steps.map((step, i) => h("div", { class: [i < at ? "done" : i === at ? "cur" : "fut", step.effect === "irreversible" ? "irr" : ""].join(" ") }, step.id, h("small", {}, step.decides)))) : null,
    h("div", { class: "hr" }),
    h("div", { class: "sect" }, h("h3", {}, "Ask the agent"),
      h("div", { class: "row" },
        h("button", { class: "btn secondary", disabled: !canSay || !fix, onclick: () => say(fixText("update", fix.branch, fix.base)) }, icon("refresh", "s14"), "Update branch"),
        h("button", { class: "btn secondary", disabled: !canSay || !fix, onclick: () => say(fixText("conflicts", fix.branch, fix.base)) }, icon("merge", "s14"), "Resolve conflicts")),
      fix ? null : h("div", { class: "note" }, icon("lock", "s14"), h("span", {}, `Task ${task.id} does not own this pull request's branch: Update branch and Resolve conflicts stay off.`)),
      h("div", { class: "note" }, icon("corner", "s14"), h("span", {}, S.unreachable ? "Reconnect to say." : route.note))),
    h("form", { class: "field", onsubmit: (event) => { event.preventDefault(); if (S.say.trim()) say(S.say.trim(), true); } },
      h("label", { for: "say" }, "Say to the agent"),
      h("textarea", { id: "say", class: "input", rows: 3, value: S.say, oninput: (event) => { S.say = event.target.value; } }),
      h("div", { class: "end" }, h("button", { class: "btn primary", type: "submit", disabled: !canSay }, "Say"))),
    S.sayNotice ? h("div", { class: "notice", role: "alert" }, S.sayNotice) : null,
    S.sayUnknown
      ? h("div", { class: "lowrow" },
        h("button", { class: "btn ghost", onclick: () => say(S.sayUnknown.text, false, S.sayUnknown.task) }, "Send again"),
        h("button", { class: "btn ghost", onclick: () => { S.sayUnknown = null; S.sayNotice = null; render(); } }, "It reached the task"))
      : null,
    h("div", { class: "hr" }),
    h("div", { class: "sect" }, h("h3", {}, "Runs", h("span", { class: "t3" }, `${task.runs.length} ${task.runs.length === 1 ? "run" : "runs"}`)),
      task.runs.slice(-4).map((run) => h("div", { class: "row t3" }, h("span", {}, run.step), h("span", { class: "sp" }), h("span", {}, run.end ?? "running")))),
    S.holders.length > 1 ? h("div", { class: "sect" }, h("h3", {}, "Also on this PR"),
      S.holders.slice(1).map((other) => h("div", { class: "row" }, tile(other, false), h("span", { class: "mono" }, `task ${other.id}`), h("span", { class: "t2" }, `${other.workflow}, ${other.state}: ${other.holds.join(", ")}`)))) : null,
    signOut(),
  ];
}

const signOut = () => h("div", { class: "end" }, h("button", { class: "btn ghost", onclick: signedOut }, "Sign out"));


let guideLoads = 0;
// Only the latest load publishes, and the guide and its quiz together, so overlapping loads never mix.
async function loadGuide() {
  const load = ++guideLoads;
  const read = await chrome.tabs.sendMessage(S.tabId, { type: "guide" }).catch(() => null);
  let guide = { state: "failed" };
  let quiz = { answers: {}, result: null, key: null };
  if (read && !read.failed && !read.json) guide = { state: "none" };
  if (read?.json) {
    const parsed = parseGuide(read.json);
    guide = parsed.error ? { state: "error", why: parsed.error } : { state: "ok", guide: parsed.guide, json: read.json, head: read.head };
  }
  if (guide.state === "ok" && guide.guide.quiz) {
    const key = await scoreKey(S.pr, guide.head ?? "unknown", guide.json);
    const answers = (await get(key)) ?? {};
    quiz = { key, answers, result: await grade(guide.guide, S.pr, answers) };
  }
  if (load !== guideLoads) return;
  Object.assign(S, { guide, quiz });
  render();
}

async function answerQuiz(id, option) {
  S.quiz.answers = { ...S.quiz.answers, [id]: option };
  await put(S.quiz.key, S.quiz.answers);
  S.quiz.result = await grade(S.guide.guide, S.pr, S.quiz.answers);
  render();
}

function guideTab() {
  const { state } = S.guide;
  if (state === "loading") return h("div", { class: "t3" }, "Reading the description…");
  if (state === "none") return h("div", { class: "read" }, "This description has no review guide.");
  if (state === "failed") return [h("div", { class: "notice" }, "Could not read the description from GitHub."), h("div", {}, h("button", { class: "btn secondary", onclick: () => loadGuide() }, "Retry"))];
  if (state === "error") return h("div", { class: "notice" }, `The guide in this description does not parse: ${S.guide.why}.`);
  const { guide, head } = S.guide;
  const moved = head && !head.startsWith(guide.head);
  return [
    h("div", { class: "sect" }, h("h3", {}, icon("book", "s14"), "Review guide", h("span", { class: "t3" }, "from the PR description")),
      moved ? h("div", { class: "notice" }, `Written for ${guide.head.slice(0, 7)}. The branch moved since.`) : null,
      guide.map.map((line) => h("div", { class: "read" }, line))),
    h("ol", { class: "steps" }, guide.steps.map((step) => h("li", {}, h("span", {}, step.title), h("span", { class: "f" }, step.lines ? `${step.file}:${step.lines}` : step.file)))),
    guide.quiz ? quiz(guide.quiz, head) : null,
  ];
}

function quiz({ pass, questions }, head) {
  const { answers, result } = S.quiz;
  const current = questions.find((question) => answers[question.id] === undefined);
  const taken = Object.keys(answers).length > 0;
  return h("div", { class: "sect" }, h("div", { class: "hr" }),
    h("h3", {}, "Quiz", h("span", { class: "t3" }, taken ? `${result.right} of ${questions.length} right. ${result.passed ? "Passed." : `Pass at ${pass}.`}` : `Not taken for ${(head ?? "this head").slice(0, 7)}. Pass at ${pass}.`)),
    h("div", { class: "score" }, questions.map((question) => h("i", { class: answers[question.id] === undefined ? "" : result.correct[question.id] ? "r" : "w" }))),
    questions.filter((question) => answers[question.id] !== undefined).map((question, i) =>
      h("div", { class: `okline ${result.correct[question.id] ? "" : "no"}` }, icon(result.correct[question.id] ? "check" : "x", "s14"),
        h("span", {}, `${i + 1}. ${result.correct[question.id] ? "Right" : "Not that one"}: ${question.question}`, question.cite ? ` ${question.cite}` : ""))),
    current
      ? [h("div", { class: "t3" }, `Question ${questions.indexOf(current) + 1} of ${questions.length}`), h("div", { class: "q" }, current.question),
        h("div", { class: "opts" }, current.options.map((option) => h("button", { class: "opt", onclick: () => answerQuiz(current.id, option) }, option)))]
      : h("div", { class: "end" }, h("button", { class: "btn ghost", onclick: async () => { S.quiz.answers = {}; await put(S.quiz.key, {}); S.quiz.result = await grade(S.guide.guide, S.pr, {}); render(); } }, "Take it again")));
}


// How long the side panel's Run stays good after the tap in the page.
const TAP_MS = 2 * 60 * 1000;

async function commandView() {
  const id = params.get("ask");
  const revision = Number(params.get("revision"));
  const option = params.get("option");
  const shell = (...kids) => h("div", { class: "panel" }, h("div", { class: "head" }, mark(), h("span", { class: "name" }, "Run a command")), h("div", { class: "body" }, kids));
  if (!S.config) return shell(h("div", { class: "read" }, "Sign in from the panel on the pull request first."));
  const detail = await api(`/api/tasks/${Number(params.get("task"))}`);
  const ask = detail.body?.asks?.find((candidate) => String(candidate.id) === id);
  if (!ask || !option) return shell(h("div", { class: "read" }, "Tap a command option in the panel on the pull request. Its command shows here to run."));
  if (ask.ran) return shell(h("div", { class: "q" }, ask.question), outcome(ask.ran));
  if (ask.state !== "open" || ask.revision !== revision || !runsCommand(ask, option)) {
    return shell(h("div", { class: "notice" }, `The question changed since the tap. Read revision ${ask.revision} in the panel and answer again.`));
  }
  const shown = { origin: S.config.origin, command: JSON.stringify(ask.command) };
  // Run checks again what this view showed: the same daemon, the same open revision, the same command.
  const run = async (button) => {
    button.disabled = true;
    S.config = (await get("config")) ?? null;
    const fresh = S.config?.origin === shown.origin ? (await api(`/api/tasks/${ask.task}`)).body?.asks?.find((candidate) => candidate.id === ask.id) : null;
    const same = fresh?.state === "open" && fresh.revision === revision && JSON.stringify(fresh.command) === shown.command;
    if (!same) return document.body.replaceChildren(await commandView());
    if (Date.now() - Number(params.get("at")) > TAP_MS) {
      return document.querySelector(".body").append(h("div", { class: "notice", role: "alert" }, "This tap is over 2 min old. Tap the option in the panel again."));
    }
    const result = await api(answerPath(ask.id), answerBody({ option, revision }, true));
    const notice = result.status === 200 ? null : refusal(result, Date.now())[1];
    document.body.replaceChildren(await commandView());
    if (notice) document.querySelector(".body").append(h("div", { class: "notice", role: "alert" }, notice));
  };
  return shell(h("div", { class: "ctx" }, h("span", { class: "mono" }, `task ${ask.task}`), h("span", {}, ask.step), h("span", {}, `revision ${revision}`)),
    h("div", { class: "q" }, ask.question), h("div", { class: "read" }, `Answer "${option}". It runs:`), commandBlock(ask, revision),
    h("div", { class: "end" }, h("button", { class: "btn primary", onclick: (event) => run(event.currentTarget) }, "Run it")));
}

function outcome(ran) {
  const words = { running: "Running.", done: "Done.", failed: `Failed: ${ran.error ?? "see the task"}.`, uncertain: "Priory stopped while this ran. Check the target before you act on it." };
  return h("div", { class: ran.status === "done" ? "okline" : "notice" }, words[ran.status] ?? ran.status);
}


// An answer waiting on Undo outlives the frame, which a page change removes with its timer.
let keptDraft = "null";
function keepDraft() {
  const kept = leftUnsent(S.draft);
  if (JSON.stringify(kept) === keptDraft) return;
  keptDraft = JSON.stringify(kept);
  (kept ? put(`draft:${S.pr.resource}`, kept) : remove(`draft:${S.pr.resource}`)).catch(() => {});
}

// A redraw keeps what the person was doing: the focused field, its selection and the scroll.
function render() {
  if (VIEW === "command") return;
  if (VIEW === "panel" && S.pr) keepDraft();
  const focused = document.activeElement;
  const { id, selectionStart, selectionEnd } = focused ?? {};
  const scroll = document.querySelector(".body")?.scrollTop ?? 0;
  document.body.replaceChildren(VIEW === "strip" ? strip() : panel());
  document.querySelector(".body")?.scrollTo(0, scroll);
  const again = id && document.getElementById(id);
  if (!again) return;
  again.focus();
  if (selectionStart != null) again.setSelectionRange(selectionStart, selectionEnd);
}

async function boot() {
  S.config = (await get("config")) ?? null;
  if (VIEW === "command") {
    channel.addEventListener("message", async (event) => {
      if (!event.data.signedOut) return;
      S.config = null;
      document.body.replaceChildren(await commandView());
    });
    document.body.replaceChildren(await commandView());
    return;
  }
  const tab = await chrome.tabs.getCurrent();
  S.tabId = tab?.id ?? null;
  S.pr = parsePr(tab?.url ?? "");
  if (!S.pr) return;
  const last = await get(`last:${S.pr.resource}`);
  if (last) Object.assign(S, last, { unreachable: false });
  if (VIEW === "panel") S.draft = (await get(`draft:${S.pr.resource}`)) ?? null;
  render();
  live((message) => {
    if (message.signedIn) get("config").then((config) => { S.config = config ?? null; read(); });
    else if (message.signedOut) signedOut(false);
    else if (message.unreachable) { S.unreachable = true; render(); }
    else read(true);
  });
  if (VIEW === "panel") {
    loadGuide();
    document.addEventListener("keydown", (event) => {
      const ask = S.tab === "ask" ? openAsks()[Math.min(S.index, openAsks().length - 1)] : null;
      const option = ask?.options[optionKey(event)];
      if (option && !event.target.closest("textarea, input") && !S.unreachable) pick(ask, { option });
    });
  }
  read();
}

boot();
