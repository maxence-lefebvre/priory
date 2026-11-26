// The strip, the docked panel and the side panel: every daemon call and all agent text live here.
// The state is one plain object, `S`; `render()` asks React to draw it again.
import { useSyncExternalStore } from "react";
import { createRoot } from "react-dom/client";
import { ARM_MS, age, answerBody, answerPath, choose, observe, optionKey, refusal, reply, runsCommand, dismissable } from "./lib/ask.js";
import { grade, parseGuide, scoreKey } from "./lib/guide.js";
import { hidden, live } from "./lib/live.js";
import { parsePr } from "./lib/pr.js";
import { ConnectForm } from "./lib/connect.jsx";
import { call } from "./lib/request.js";
import { clear, get, put, remove } from "./lib/store.js";
import { fixBranch, fixText, guideText, sayBody, sayRoute } from "./lib/task.js";
import { store } from "../../crates/priory-core/web/store.js";
import { AskOption, Busy } from "../../crates/priory-core/web/components/AskOption.jsx";
import { Icon, Tile } from "../../crates/priory-core/web/components/Icon.jsx";

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
  // Whether `holders` comes from a read of this frame, not from the cache of an earlier visit.
  confirmed: false,
  unreachable: false,
  tab: params.get("tab"),
  draft: null,
  index: 0,
  writing: false,
  written: "",
  say: "",
  sayNotice: null,
  sayBusy: false,
  guide: { state: "loading" },
  quiz: { answers: {}, result: null, key: null },
  command: null,
};

// Each draw carries its time, so the views stay pure. The panel's tab is settled here, outside React.
const drawn = store({ at: 0 });
function render() {
  if (VIEW === "panel") {
    const tabs = tabsOf();
    S.tab = tabs.some(([id]) => id === S.tab) ? S.tab : openAsks().length ? "ask" : "task";
  }
  drawn.set({ at: Date.now() });
}

const Mark = () => (
  <span className="mark"><svg viewBox="0 0 32 32" aria-hidden="true">
    <path className="book" fill="currentColor" fillRule="evenodd" d="M6 2h16a2 2 0 0 1 2 2v18H6Z M8 24h8v4H8Z M10 6h6a4 4 0 0 1 0 8h-3v4h-3Z M13 9h3a1 1 0 0 1 0 2h-3Z" />
    <path className="ribbon" fill="currentColor" fillRule="evenodd" d="M18 20h4v11l-2-2.5L18 31Z" />
  </svg></span>
);

const tile = (task, waits) => <Tile state={waits ? "ask" : task.state} />;

const openAsks = () => (S.task?.asks ?? []).filter((ask) => ask.state === "open").sort((a, b) => a.since.localeCompare(b.since));
const waiting = () => openAsks().filter((ask) => !(ask.snoozed_until && Date.parse(ask.snoozed_until) > Date.now()));
const api = (path, body, timeout) => call(fetch, S.config.origin, S.config.token, path, body, timeout);

// Only an answer the daemon gave says no task holds the pull request: the cache or a failed read never does.
const noTask = () => S.confirmed && !S.unreachable && S.holders?.length === 0;
const lookup = () => (S.unreachable ? "Cannot reach Priory. Retrying…" : "Reading Priory…");

let retryTimer = 0;
let retryMs = 2000;
function failed() {
  S.unreachable = true;
  clearTimeout(retryTimer);
  retryTimer = setTimeout(read, retryMs);
  retryMs = Math.min(retryMs * 2, 30000);
  render();
}

// One read at a time and at most one queued, so a slow server gets no pile of reads; after a read
// over 2 s the queued one waits as long. A hidden tab reads nothing until it shows.
let reading = null;
let queued = null;
let stale = false;
function read(quiet = false) {
  if (hidden()) {
    stale = true;
    return Promise.resolve();
  }
  if (reading) {
    queued = queued === null ? quiet : queued && quiet;
    return reading;
  }
  const started = Date.now();
  reading = readNow(quiet).finally(() => {
    reading = null;
    const took = Date.now() - started;
    if (queued === null) return;
    const next = queued;
    queued = null;
    setTimeout(() => read(next), took > 2000 ? took : 0);
  });
  return reading;
}
globalThis.document?.addEventListener?.("visibilitychange", () => {
  if (hidden() || !stale) return;
  stale = false;
  read(true);
});

// `quiet`, for a revision from the poller: a read that changed nothing leaves the view as it is.
async function readNow(quiet = false) {
  clearTimeout(retryTimer);
  if (!S.config || !S.pr) return render();
  const found = await api(`/api/tasks?pr=${encodeURIComponent(S.pr.resource)}`);
  if (found.status === 401) return signedOut();
  if (found.status !== 200) return failed();
  const holders = found.body.tasks;
  const detail = holders[0] ? await api(`/api/tasks/${holders[0].id}`) : { status: 200, body: null };
  if (!S.flows) {
    const flows = await api("/api/flows");
    S.flows = flows.status === 200 ? flows.body : null;
  }
  if (detail.status !== 200) return failed();
  retryMs = 2000;
  const seen = JSON.stringify([holders, detail.body]);
  S.readAt = Date.now();
  if (seen === S.seen && !S.unreachable) return quiet ? undefined : render();
  Object.assign(S, { seen, holders, task: detail.body, confirmed: true, unreachable: false });
  for (const ask of S.task?.asks ?? []) S.draft = observe(S.draft, ask, Date.now());
  if (S.draft?.phase === "sent" && !S.task?.asks?.some((ask) => ask.id === S.draft.id && ask.state === "open")) S.draft = null;
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

/** Forgets the token and all it read, here and with `share` in every other frame too. */
async function signedOut(share = true) {
  await clear();
  S.config = null;
  S.draft = null;
  if (share) channel.postMessage({ signedOut: true });
  render();
}

const send = (type, extra = {}) => S.tabId != null && chrome.tabs.sendMessage(S.tabId, { type, ...extra }).catch(() => {});


let armTimer = 0;
function pick(ask, answer) {
  const before = S.draft;
  S.draft = choose(S.draft, ask, answer, Date.now());
  if (S.draft === before) return;
  if (S.draft.phase === "armed") return openCommand(ask, answer.option);
  if (S.draft.phase === "posting") return post();
  // The first press arms the option until ARM_MS pass.
  clearTimeout(armTimer);
  armTimer = setTimeout(() => {
    S.draft = observe(S.draft, ask, Date.now());
    render();
  }, ARM_MS);
  render();
}

// The strip, a frame of its own, shows the answer sending too.
const sharing = (text) => S.pr && channel.postMessage({ sending: text === null ? null : { pr: S.pr.resource, text } });

async function post() {
  clearTimeout(armTimer);
  const path = answerPath(S.draft.id);
  const body = answerBody(S.draft);
  render();
  sharing(S.draft.option ?? S.draft.text);
  let result = await api(path, body);
  // A server older than call ids refuses the field: the answer goes once more, without it.
  if (result.status === 400 && /unknown field `call`/.test(result.body.error ?? "")) result = await api(path, { ...body, call: undefined });
  sharing(null);
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
  const refused = await write(`/api/tasks/${task}/say`, sayBody(S.pr, text));
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


function Strip({ now }) {
  const toggle = (tab) => <button type="button" className="btn icon" title="Open the Priory panel" onClick={() => send("open", { tab })}><Icon name="panel-right" /></button>;
  const brand = <span className="brand"><Mark />Priory</span>;
  if (!S.config) {
    return <div className="strip">{brand}<span className="t2">Sign in to see the task that owns this pull request.</span>
      <button type="button" className="btn primary" onClick={() => send("open", { tab: "task" })}>Sign in</button><span className="sp" />{toggle("task")}</div>;
  }
  const stale = S.unreachable
    ? <span className="warn"><Icon name="unplug" size="s14" />{S.readAt ? `Cannot reach Priory. Retrying… Last read ${age(new Date(S.readAt).toISOString(), now)} ago.` : "Cannot reach Priory. Retrying…"}</span>
    : null;
  if (!S.task && !noTask()) return <div className="strip">{brand}{stale ?? <span className="t3">Reading Priory…</span>}<span className="sp" />{toggle("task")}</div>;
  if (!S.task) {
    return <div className="strip">{brand}<span className="t2">No Priory task owns this pull request.</span>
      <button type="button" className="btn ghost" onClick={() => send("open", { tab: "task" })}>Add a task…</button><span className="sp" />{toggle("task")}</div>;
  }
  const asks = waiting();
  return (
    <div className="strip">{brand}{tile(S.task, asks.length > 0)}<span className="mono">{`task ${S.task.id}`}</span>
      <span className="t2">{`${S.task.state} at `}<b>{S.task.step}</b></span>
      {S.sending ? <span className="t2" role="status"><Busy />{`Sending "${S.sending}"…`}</span>
        : asks.length ? <button type="button" className="waitpill" onClick={() => send("open", { tab: "ask" })}><Icon name="ask" size="s14" />{`${asks.length} ${asks.length > 1 ? "asks wait" : "ask waits"} on you`}</button> : null}
      {stale ?? (S.task.since ? <span className="t3">{age(S.task.since, now)}</span> : null)}<span className="sp" />{toggle(asks.length ? "ask" : "task")}</div>
  );
}

function tabsOf() {
  const guideOn = S.guide.state !== "none" || Boolean(S.task);
  return [["ask", "Ask", openAsks().length], ["task", "Task", 0], ...(guideOn ? [["guide", "Guide", 0]] : [])];
}

function Panel({ now }) {
  const asks = openAsks();
  const tabs = tabsOf();
  const conn = !S.config ? <span className="conn">signed out</span>
    : S.unreachable ? <span className="conn off">unreachable</span>
      : <span className="conn live"><span className="livedot" />live</span>;
  const open = (id) => {
    S.tab = id;
    if (id === "guide") loadGuide();
    render();
  };
  return (
    <div className="panel">
      <div className="head"><Mark /><span className="name">Priory</span>{conn}
        {S.config && S.task ? <a className="btn icon" title={`Open task ${S.task.id} in Priory`} href={`${S.config.origin}/tasks/${S.task.id}`} target="_blank" rel="noreferrer"><Icon name="external-link" /></a> : null}
        <button type="button" className="btn icon" title="Close" onClick={() => send("close")}><Icon name="x" /></button></div>
      <div className="tabs" role="tablist">{tabs.map(([id, label, count]) =>
        <button type="button" key={id} role="tab" aria-selected={String(S.tab === id)} onClick={() => open(id)}>{label}{count ? <span className="badge">{count}</span> : null}</button>)}</div>
      {S.config && S.unreachable
        ? <div className="stale"><Icon name="unplug" size="s14" />{S.readAt ? `Cannot reach Priory. Retrying… Showing what it said ${age(new Date(S.readAt).toISOString(), now)} ago.` : "Cannot reach Priory. Retrying…"}
          <button type="button" className="btn secondary" onClick={() => read()}>Retry</button></div>
        : null}
      <div className="body">{S.tab === "guide" ? <GuideTab /> : !S.config ? <ConnectForm render={render} permissions={chrome.permissions} channel={channel} guide done={(config) => { S.config = config; read(); }} /> : S.tab === "ask" ? <AskTab asks={asks} now={now} /> : <TaskTab now={now} />}</div>
    </div>
  );
}

function AskTab({ asks, now }) {
  if (!S.task) return <div className="read">{noTask() ? "No Priory task owns this pull request, so nothing asks here." : lookup()}</div>;
  if (!asks.length) return <div className="read">{`No open asks. Task ${S.task.id}'s agent asks here when it needs you.`}</div>;
  const ask = asks[Math.min(S.index, asks.length - 1)];
  const draft = S.draft?.id === ask.id ? S.draft : null;
  const busy = S.draft?.phase === "posting";
  const blocked = S.unreachable || busy || draft?.phase === "unknown";
  const snoozed = ask.snoozed_until && Date.parse(ask.snoozed_until) > now;
  const step = (by) => {
    S.index = Math.min(asks.length - 1, Math.max(0, S.index + by));
    render();
  };
  const write = (event) => {
    event.preventDefault();
    if (S.written.trim()) pick(ask, { text: S.written.trim(), again: draft?.phase === "unknown" });
  };
  return (
    <>
      {asks.length > 1 ? <div className="row"><span className="t3">{`${asks.indexOf(ask) + 1}/${asks.length}`}</span>
        <button type="button" className="btn ghost" onClick={() => step(-1)}>Previous</button>
        <button type="button" className="btn ghost" onClick={() => step(1)}>Next</button></div> : null}
      <div className="ctx">{tile(S.task, true)}<span className="mono">{`task ${S.task.id}`}</span><span>{ask.step}</span><span>{S.task.lane}</span>{ask.command ? <span>Command</span> : null}<span>{age(ask.since, now)}</span><span>{`revision ${ask.revision}`}</span></div>
      <div className="q">{ask.question}</div>
      {ask.routed_to ? <div className="routed">{`${ask.routed_to} is answering. You can still answer first: the first answer stands.`}</div> : null}
      {snoozed ? <div className="note">{`Snoozed until ${new Date(ask.snoozed_until).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}.`}</div> : null}
      {ask.recommendation ? <div className="rec"><span><b>{`Recommended: ${ask.recommendation}.`}</b></span>{ask.next_action ? <span className="next"><Icon name="corner-down-right" size="s14" /><span>{`Next: ${ask.next_action}`}</span></span> : null}</div> : null}
      <div className="opts" role="group" aria-label="Options">{ask.options.map((option, i) => {
        // The pressed option shows its answer sending on itself; the others take no press meanwhile.
        const armed = (draft?.phase === "armed" || draft?.phase === "confirm") && draft.option === option;
        const sending = busy && draft?.option === option;
        const note = draft?.phase === "armed" && armed ? "Opened in the side panel: run it there" : armed ? "Press again to send" : undefined;
        return <AskOption key={i} option={option} index={i} recommended={option === ask.recommendation} runs={runsCommand(ask, option)} sending={sending} armed={armed} off={blocked} note={note} hint={i + 1} onPress={() => blocked || pick(ask, { option })} />;
      })}</div>
      {S.unreachable ? <div className="notice">Reconnect to answer.</div> : null}
      {draft?.phase === "armed" ? <CommandBlock ask={ask} /> : null}
      {S.writing
        ? <form className="field" onSubmit={write}>
          <textarea id="written" className="input" rows={3} aria-label="Your answer" value={S.written} onChange={(event) => { S.written = event.target.value; render(); }} />
          <div className="end">{busy && draft?.text !== undefined
            ? <button className="btn primary" type="submit" aria-disabled="true" aria-busy="true"><Busy />Sending…</button>
            : <button className="btn primary" type="submit" disabled={blocked}>Send</button>}</div>
        </form>
        : null}
      <div className="lowrow">
        {S.writing ? null : <button type="button" className="btn ghost" disabled={blocked} onClick={() => { S.writing = true; render(); }}><Icon name="pencil" size="s14" />Write an answer</button>}
        {dismissable(ask) ? <button type="button" className="btn ghost" disabled={blocked} onClick={() => dismiss(ask)}>Dismiss</button> : null}
        <button type="button" className="btn ghost" disabled={blocked} onClick={() => snooze(ask, snoozed)}>{snoozed ? "Unsnooze" : "Snooze 1 h"}</button>
        {draft?.phase === "unknown"
          ? <button type="button" className="btn ghost" onClick={() => (draft.dismiss ? dismiss(ask) : pick(ask, { ...(draft.option === undefined ? { text: draft.text } : { option: draft.option }), again: true }))}>Send again</button>
          : null}
      </div>
      {draft?.notice ? <div className="notice" role="alert">{draft.notice}</div> : null}
      <div className="hr" />
      {ask.text ? <div className="sect"><h3>What the agent says</h3><div className="read">{ask.text}</div></div> : null}
      {ask.tradeoff ? <div className="fact"><Icon name="scale" size="s14" /><span><b>Trade-off. </b>{ask.tradeoff}</span></div> : null}
      {ask.what_waits ? <div className="fact"><Icon name="hourglass" size="s14" /><span><b>What waits. </b>{ask.what_waits}</span></div> : null}
      {S.task.brief?.request ? <div className="sect"><h3>Brief</h3><div className="read">{S.task.brief.request}</div></div> : null}
    </>
  );
}

const CommandBlock = ({ ask, revision = ask.revision }) => (
  <div className="cmd"><div className="argv">{ask.command.argv.map((word, i) => <span key={i}>{word === "" ? "''" : word}</span>)}</div>
    <span>{`In ${ask.command.dir ?? ask.command.target}. Bound to revision ${revision}. Runs at most once.`}</span></div>
);

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

const SayNote = ({ route }) => <div className="note"><Icon name="corner-down-right" size="s14" /><span>{S.unreachable ? "Reconnect to say." : route.note}</span></div>;
const Notice = ({ text }) => (text ? <div className="notice" role="alert">{text}</div> : null);
const SignOut = () => <div className="end"><button type="button" className="btn ghost" onClick={() => signedOut()}>Sign out</button></div>;

function TaskTab({ now }) {
  if (!S.task && !noTask()) return <><div className="read">{lookup()}</div><SignOut /></>;
  if (!S.task) {
    return (
      <>
        <div className="q">No Priory task owns this pull request</div>
        <div className="read">Add a task sends a request with this pull request's address. The factory's [request] defaults pick its workflow.</div>
        <div className="end"><button type="button" className="btn primary" disabled={S.sayBusy} onClick={addTask}>Add a task for this PR</button></div>
        <Notice text={S.sayNotice} />
        <SignOut />
      </>
    );
  }
  const task = S.task;
  const route = sayRoute(task, S.flows);
  const fix = fixBranch(task, S.holders?.find((holder) => holder.id === task.id)?.holds, S.pr);
  const canSay = route.enabled && !S.sayBusy && !S.unreachable && !S.sayUnknown;
  const steps = S.flows?.workflows?.find((workflow) => workflow.name === task.workflow)?.steps ?? [];
  const at = steps.findIndex((step) => step.id === task.step);
  const submit = (event) => {
    event.preventDefault();
    if (S.say.trim()) say(S.say.trim(), true);
  };
  return (
    <>
      <div className="ctx">{tile(task, waiting().length > 0)}<span className="mono">{`task ${task.id}`}</span><span>{task.key}</span><span>{task.step}</span><span>{task.lane}</span>{task.band ? <span>{task.band}</span> : null}</div>
      <div className="q">{task.brief?.title ?? task.key}</div>
      <div className="t2">{`${task.state}${task.reason ? `: ${task.reason}` : ""}${task.since ? `, since ${age(task.since, now)}` : ""}`}</div>
      {steps.length ? <div className="grid">{steps.map((step, i) => <div key={step.id} className={[i < at ? "done" : i === at ? "cur" : "fut", step.effect === "irreversible" ? "irr" : ""].join(" ")}>{step.id}<small>{step.decides}</small></div>)}</div> : null}
      <div className="hr" />
      <div className="sect"><h3>Ask the agent</h3>
        <div className="row">
          <button type="button" className="btn secondary" disabled={!canSay || !fix} onClick={() => say(fixText("update", fix.branch, fix.base))}><Icon name="refresh-cw" size="s14" />Update branch</button>
          <button type="button" className="btn secondary" disabled={!canSay || !fix} onClick={() => say(fixText("conflicts", fix.branch, fix.base))}><Icon name="git-merge" size="s14" />Resolve conflicts</button></div>
        {fix ? null : <div className="note"><Icon name="lock" size="s14" /><span>{`Task ${task.id} does not own this pull request's branch: Update branch and Resolve conflicts stay off.`}</span></div>}
        <SayNote route={route} /></div>
      <form className="field" onSubmit={submit}>
        <label htmlFor="say">Say to the agent</label>
        <textarea id="say" className="input" rows={3} value={S.say} onChange={(event) => { S.say = event.target.value; render(); }} />
        <div className="end"><button className="btn primary" type="submit" disabled={!canSay}>Say</button></div>
      </form>
      <Notice text={S.sayNotice} />
      {S.sayUnknown
        ? <div className="lowrow">
          <button type="button" className="btn ghost" onClick={() => say(S.sayUnknown.text, false, S.sayUnknown.task)}>Send again</button>
          <button type="button" className="btn ghost" onClick={() => { S.sayUnknown = null; S.sayNotice = null; render(); }}>It reached the task</button></div>
        : null}
      <div className="hr" />
      <div className="sect"><h3>Runs<span className="t3">{`${task.runs.length} ${task.runs.length === 1 ? "run" : "runs"}`}</span></h3>
        {task.runs.slice(-4).map((run, i) => <div key={i} className="row t3"><span>{run.step}</span><span className="sp" /><span>{run.end ?? "running"}</span></div>)}</div>
      {S.holders.length > 1 ? <div className="sect"><h3>Also on this PR</h3>
        {S.holders.slice(1).map((other) => <div key={other.id} className="row">{tile(other, false)}<span className="mono">{`task ${other.id}`}</span><span className="t2">{`${other.workflow}, ${other.state}: ${other.holds.join(", ")}`}</span></div>)}</div> : null}
      <SignOut />
    </>
  );
}

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

async function answerQuiz(answers) {
  S.quiz.answers = answers;
  await put(S.quiz.key, answers);
  S.quiz.result = await grade(S.guide.guide, S.pr, answers);
  render();
}

function GuideTab() {
  const { state } = S.guide;
  if (state === "loading") return <div className="t3">Reading the description…</div>;
  if (state === "none") return <><div className="read">This description has no review guide.</div>{S.task ? <GenerateGuide /> : null}</>;
  if (state === "failed") return <><div className="notice">Could not read the description from GitHub.</div><div><button type="button" className="btn secondary" onClick={() => loadGuide()}>Retry</button></div></>;
  if (state === "error") return <div className="notice">{`The guide in this description does not parse: ${S.guide.why}.`}</div>;
  const { guide, head } = S.guide;
  const moved = head && !head.startsWith(guide.head);
  return (
    <>
      <div className="sect"><h3><Icon name="book-open" size="s14" />Review guide<span className="t3">from the PR description</span></h3>
        {moved ? <div className="notice">{`Written for ${guide.head.slice(0, 7)}. The branch moved since.`}</div> : null}
        {guide.map.map((line, i) => <div key={i} className="read">{line}</div>)}</div>
      <ol className="steps">{guide.steps.map((step, i) => <li key={i}><span>{step.title}</span><span className="f">{step.lines ? `${step.file}:${step.lines}` : step.file}</span></li>)}</ol>
      {guide.quiz ? <Quiz quiz={guide.quiz} head={head} /> : null}
    </>
  );
}

// The agent writes the block into the description; the tab reads it again when opened.
function GenerateGuide() {
  const route = sayRoute(S.task, S.flows);
  const can = route.enabled && !S.sayBusy && !S.unreachable && !S.sayUnknown;
  return (
    <>
      <div className="end"><button type="button" className="btn primary" disabled={!can} onClick={() => say(guideText(S.pr))}><Icon name="book-open" size="s14" />Generate guide</button></div>
      <SayNote route={route} />
      <Notice text={S.sayNotice} />
    </>
  );
}

function Quiz({ quiz: { pass, questions }, head }) {
  const { answers, result } = S.quiz;
  const current = questions.find((question) => answers[question.id] === undefined);
  const taken = Object.keys(answers).length > 0;
  return (
    <div className="sect"><div className="hr" />
      <h3>Quiz<span className="t3">{taken ? `${result.right} of ${questions.length} right. ${result.passed ? "Passed." : `Pass at ${pass}.`}` : `Not taken for ${(head ?? "this head").slice(0, 7)}. Pass at ${pass}.`}</span></h3>
      <div className="score">{questions.map((question) => <i key={question.id} className={answers[question.id] === undefined ? "" : result.correct[question.id] ? "r" : "w"} />)}</div>
      {questions.filter((question) => answers[question.id] !== undefined).map((question, i) =>
        <div key={question.id} className={`okline ${result.correct[question.id] ? "" : "no"}`}><Icon name={result.correct[question.id] ? "check" : "x"} size="s14" />
          <span>{`${i + 1}. ${result.correct[question.id] ? "Right" : "Not that one"}: ${question.question}`}{question.cite ? ` ${question.cite}` : ""}</span></div>)}
      {current
        ? <><div className="t3">{`Question ${questions.indexOf(current) + 1} of ${questions.length}`}</div><div className="q">{current.question}</div>
          <div className="opts">{current.options.map((option) => <button type="button" key={option} className="opt" onClick={() => answerQuiz({ ...answers, [current.id]: option })}>{option}</button>)}</div></>
        : <div className="end"><button type="button" className="btn ghost" onClick={() => answerQuiz({})}>Take it again</button></div>}
    </div>
  );
}

// How long the side panel's Run stays good after the tap in the page.
const TAP_MS = 2 * 60 * 1000;

/** What the side panel shows for the ask and option in its address. */
async function commandModel() {
  const id = params.get("ask");
  const revision = Number(params.get("revision"));
  const option = params.get("option");
  // Signed out, the side panel signs in itself: the strip may not draw on every GitHub page.
  if (!S.config) return { kind: "signin" };
  const detail = id && option ? await api(`/api/tasks/${Number(params.get("task"))}`) : null;
  const ask = detail?.body?.asks?.find((candidate) => String(candidate.id) === id);
  if (!ask || !option) return { kind: "none" };
  if (ask.ran) return { kind: "ran", ask };
  if (ask.state !== "open" || ask.revision !== revision || !runsCommand(ask, option)) return { kind: "changed", ask };
  return { kind: "run", ask, option, revision, shown: { origin: S.config.origin, command: JSON.stringify(ask.command) } };
}

async function showCommand(notice = null) {
  S.command = { ...(await commandModel()), notice };
  render();
}

// Run checks again what this view showed: the same daemon, the same open revision, the same command.
async function runCommand() {
  const { ask, option, revision, shown } = S.command;
  S.command = { ...S.command, busy: true };
  render();
  S.config = (await get("config")) ?? null;
  const fresh = S.config?.origin === shown.origin ? (await api(`/api/tasks/${ask.task}`)).body?.asks?.find((candidate) => candidate.id === ask.id) : null;
  const same = fresh?.state === "open" && fresh.revision === revision && JSON.stringify(fresh.command) === shown.command;
  if (!same) return showCommand();
  if (Date.now() - Number(params.get("at")) > TAP_MS) {
    S.command = { ...S.command, notice: "This tap is over 2 min old. Tap the option in the panel again." };
    return render();
  }
  const result = await api(answerPath(ask.id), answerBody({ option, revision }, true));
  return showCommand(result.status === 200 ? null : refusal(result, Date.now())[1]);
}

const RAN = (ran) => ({ running: "Running.", done: "Done.", failed: `Failed: ${ran.error ?? "see the task"}.`, uncertain: "Priory stopped while this ran. Check the target before you act on it." })[ran.status] ?? ran.status;

function CommandView() {
  const model = S.command;
  if (!model) return null;
  const { kind, ask } = model;
  const body = kind === "signin" ? <ConnectForm render={render} permissions={chrome.permissions} channel={channel} done={(config) => { S.config = config; showCommand(); }} />
    : kind === "none" ? <div className="read">Tap a command option in the panel on the pull request. Its command shows here to run.</div>
      : kind === "ran" ? <><div className="q">{ask.question}</div><div className={ask.ran.status === "done" ? "okline" : "notice"}>{RAN(ask.ran)}</div></>
        : kind === "changed" ? <div className="notice">{`The question changed since the tap. Read revision ${ask.revision} in the panel and answer again.`}</div>
          : <>
            <div className="ctx"><span className="mono">{`task ${ask.task}`}</span><span>{ask.step}</span><span>{`revision ${model.revision}`}</span></div>
            <div className="q">{ask.question}</div><div className="read">{`Answer "${model.option}". It runs:`}</div><CommandBlock ask={ask} revision={model.revision} />
            <div className="end"><button type="button" className="btn primary" disabled={model.busy} onClick={runCommand}>Run it</button></div>
          </>;
  return <div className="panel"><div className="head"><Mark /><span className="name">Run a command</span></div><div className="body">{body}<Notice text={model.notice} /></div></div>;
}

function Frame() {
  const { at } = useSyncExternalStore(drawn.subscribe, drawn.get);
  // Nothing draws before the first render(), as before the port: the cache or the address comes first.
  if (!at) return null;
  return VIEW === "command" ? <CommandView /> : VIEW === "strip" ? <Strip now={at} /> : <Panel now={at} />;
}

async function boot() {
  S.config = (await get("config")) ?? null;
  createRoot(document.body).render(<Frame />);
  if (VIEW === "command") {
    channel.addEventListener("message", async (event) => {
      if (!event.data.signedOut && !event.data.signedIn) return;
      S.config = event.data.signedIn ? (await get("config")) ?? null : null;
      showCommand();
    });
    return showCommand();
  }
  const tab = await chrome.tabs.getCurrent();
  S.tabId = tab?.id ?? null;
  S.pr = parsePr(tab?.url ?? "");
  if (!S.pr) return;
  const last = await get(`last:${S.pr.resource}`);
  if (last) Object.assign(S, last, { unreachable: false });
  // An earlier version kept an answer waiting on its Undo; press-twice sends at once, so none waits.
  if (VIEW === "panel") remove(`draft:${S.pr.resource}`).catch(() => {});
  render();
  live((message) => {
    if (message.signedIn) get("config").then((config) => { S.config = config ?? null; read(); });
    else if (message.signedOut) signedOut(false);
    else if (message.unreachable) { S.unreachable = true; render(); }
    else if ("sending" in message) { S.sending = message.sending?.pr === S.pr.resource ? message.sending.text : null; render(); }
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
