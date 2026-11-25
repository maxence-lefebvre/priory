// The desk page's legacy drawing and actions.
import { render } from "../app.js";
import { askHeading, routedMark, runnerOf } from "../asks.js";
import { actionNote, batchParts, capsuleWords, countHead, countWords, deskBanners, dueParts, nextLink, onRoom, plural, rowAction, sinceGroups } from "../desk.js";
import { droppedNote, sinceWords, unheardNote, unroutedNote } from "../inbox.js";
import { taskLabel, workStep } from "../tasks.js";
import { addButton } from "./add.js";
import { askRow, askTableRow, runnerTag, sentLine } from "./asks.js";
import { refresh, signOut } from "./data.js";
import { connection } from "./frame.js";
import { age, ago, autolinked, clock, date, html, icon, listOrEmpty, listSearch, olderRow, olderTasks, post, prsTag, raw, repoOf, role, savedViews, selectedAsk, state, stateWord, tile, timeTag, today } from "./shared.js";
import { taskActs } from "./taskacts.js";


// Desk model
export function deskModel() {
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
  return { next: desk.next, open, first: first ? byId.get(first.id) || open[0] : open[0], stopped, todos, reports, other, asleep: desk.counts.asleep, asleepTasks: queue.asleep, banners: deskBanners(desk, ago), intake: desk.intake, running: queue.running, queue: queue.waiting, tasks, upNext: queue.up_next, needs: items.length };
}
export const openAsks = () => deskModel().open;

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
    <span class="l2 r t-${role(kind)}">${autolinked(line, repoOf(item.task))}</span>${next ? html`<span class="l3">${autolinked(next, repoOf(item.task))}</span>` : ""}${stopped ? taskActs(item.task, item.kind) : rowActs(item)}</div>`;
}

// The one row of every memory candidate that waits, which opens their review list.
function memoriesRow(item) {
  const count = item.candidates?.length ?? 0;
  return html`<a class="row" href="/memories">
    ${tile("memories")}
    <span class="l1"><span>memories</span><span>${plural(count, "candidate")}</span><span class="age">${timeTag(item.since)}</span></span>
    <span class="l2 r">${(item.candidates || []).map((candidate) => candidate.name).join(", ")}</span>
    <span class="l3"><span>Accept or drop each before other tasks get it</span></span>
    <span class="chev">${icon("chevron-right", "s15")}</span></a>`;
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

export async function rowAct(target) {
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

export async function markDone(target) {
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
    <span class="l1"><span class="mono">${taskLabel(task.key)}</span><span>${workStep(task)}</span><span>asleep</span>${prsTag(task)}<span class="age">${timeTag(task.since)}</span></span>
    <span class="l2 r">${task.title || task.lane || ""}</span>
    <span class="l3"><span>${waits ? `Waits on ${waits}` : "Waits on an ask"}</span></span>
    <span class="chev">${icon("chevron-right", "s15")}</span></a>`;
}

function taskRow(task) {
  return html`<div class="row">
    ${tile(task.state)}
    <span class="l1"><span class="mono">${taskLabel(task.key)}</span><span>${workStep(task)}</span><span>${task.state}</span>${prsTag(task)}<span class="age">${timeTag(task.since)}</span></span>
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

export function deskView() {
  const name = new URLSearchParams(listSearch()).get("view");
  return DESK_VIEWS.includes(name) ? name : "needs";
}

export const countLine = (model) => countWords({ oldest: model.open[0] ? age(model.open[0].since) : null, stopped: model.stopped.length, todos: model.todos.length, reports: model.reports.length, other: model.other.length });

export function upNext(model, queueLink = true) {
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
    : null;
  const link = queueLink ? html`<a class="open" href="/queue">Open the queue</a>` : "";
  const shown = body ? html`<div class="strip">${body}${link}</div>` : html`<p class="empty">Nothing is queued or scheduled to run next.</p>${link}`;
  return html`<div class="sec">Up next<span>next 3 runs</span></div>${shown}`;
}

// The one next thing (DESIGN.md, Desk): the desk's `next`, an alarm or the ask that ranks first, and why.
export function nextCard(model) {
  const item = model.next;
  if (!item) return "";
  const ask = item.kind === "ask" ? state.data.asks.find((each) => String(each.id) === String(item.id)) : null;
  const task = state.data.tasks.find((each) => each.id === item.task);
  const text = ask ? askHeading(ask) : ["blocked", "failed"].includes(item.kind) && task?.reason ? task.reason : item.what;
  const kind = ["prompt", "command"].includes(item.kind) ? "prompt" : item.kind === "report" ? "lost" : item.kind;
  const href = nextLink(item);
  const verb = ask ? "Answer…" : href?.startsWith("#") ? "See the banner" : "Open task";
  const button = item.kind === "todo" ? todoDone(item.id, "next") : href ? html`<a class="btn ${ask ? "primary" : "secondary"}" href="${href}">${verb}</a>` : "";
  return html`<section class="next" aria-label="The next thing"><p class="l1">${tile(kind, "sm")}<span class="mono">${taskLabel(item.key)}</span><span>${item.step || ""}</span><span class="age">${timeTag(item.since)}</span></p>
    <p class="q">${item.kind === "todo" ? todoText(item) : autolinked(text, ask?.repo || task?.repo)}</p>${ask && routedMark(ask) ? html`<p class="why t-live">${routedMark(ask)}</p>` : ""}${runnerOf(ask) ? html`<p class="why">${runnerTag(ask)} ${sentLine(ask)}</p>` : ""}<p class="why t-${role(kind)}">${autolinked(item.why, ask?.repo || task?.repo)}</p>
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
      <ol>${group.events.map((event) => html`<li>${timeTag(event.time)}<span>${autolinked(sinceWords(event.what), repoOf(event.task ?? group.task))}</span></li>`)}</ol></div>`)
    : null;
  const shown = body ? html`<div class="strip since" data-since>${body}</div>` : html`<p class="empty" data-since>Nothing happened since you looked.</p>`;
  return html`<div class="sec">Since you looked<span>${from ? `since ${today(from) ? clock(from) : date(from)}` : "latest 50"}</span></div>${shown}`;
}

export function deskBody(wide) {
  const model = deskModel();
  const current = deskView();
  const views = savedViews([
    ["needs", "Needs me", model.needs, current === "needs"],
    ["running", "Running", model.running.length, current === "running"],
    ...(wide ? [["queue", "Queue", model.queue.length, current === "queue"]] : []),
    ["asleep", "Asleep", model.asleep, current === "asleep"],
    ["all", "All", wide ? model.tasks.length + olderTasks() : null, current === "all"],
  ], "/");
  const banner = model.banners.map(({ text, icon: name, intake }) => html`<div class="banner" role="status" ${intake ? raw('id="intake"') : ""}>${icon(name, "s20")}<span>${text}</span></div>`);
  let rows;
  if (current === "running") rows = model.running.length ? model.running.map(runRow) : html`<p class="empty">No run is live. Runs show here while an agent works.</p>`;
  else if (current === "queue") rows = listOrEmpty(model.queue.map(taskRow), html`The queue is empty. A task waits here for its turn; see <code>priory queue</code>.`);
  else if (current === "asleep") rows = listOrEmpty(model.asleepTasks.map(asleepRow), "No task is asleep. A task sleeps here while it waits on an event.");
  else if (current === "all") rows = model.tasks.length || olderTasks() ? [...model.tasks.map(taskRow), olderRow("tasks", olderTasks(), false)] : listOrEmpty([], html`No tasks yet. Add one with the + button, or run <code>priory add</code>.`);
  else rows = needsMe(model, wide);
  return { model, views, banner, rows };
}

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
  if (model.asleep) sections.push(html`<div class="sec"></div><a class="row one" href="/?view=asleep">${tile("asleep")}<span>${plural(model.asleep, "task")} asleep on waits</span><span class="num">${model.asleep}</span><span class="chev">${icon("chevron-right", "s15")}</span></a>`);
  const unrouted = unroutedNote(state.data.desk.counts?.unrouted);
  if (unrouted) sections.push(html`<a class="row one calm" href="/inbox?view=unrouted">${tile("queued")}<span>${unrouted}</span><span class="num">${state.data.desk.counts.unrouted}</span><span class="chev">${icon("chevron-right", "s15")}</span></a>`);
  const unheard = unheardNote(state.data.desk.counts?.unheard);
  if (unheard) sections.push(html`<a class="row one calm" href="/inbox?view=unrouted#unheard">${tile("asleep")}<span>${unheard}</span><span class="num">${state.data.desk.counts.unheard}</span><span class="chev">${icon("chevron-right", "s15")}</span></a>`);
  const dropped = droppedNote(state.data.desk.counts?.dropped);
  if (dropped) sections.push(html`<a class="row one calm" href="/inbox?view=unrouted#dropped">${tile("dropped")}<span>${dropped}</span><span class="num">${state.data.desk.counts.dropped}</span><span class="chev">${icon("chevron-right", "s15")}</span></a>`);
  if (model.reports.length) sections.push(html`<div class="sec">Reports<span>${model.reports.length}</span></div>${model.reports.map(itemRow)}`);
  return sections;
}

// The fifth column's content, again under the question: CSS shows it when that column folds. Its
// ids take a prefix, so each stays unique and focus comes back to the copy that was used.
export const fold = (content) => (content ? html`<div class="fifth">${raw(html`${content}`.text.replaceAll(' id="', ' id="fold-'))}</div>` : "");
const review = html`<span class="task-acts"><a class="btn secondary act" href="/memories">Review</a></span>`;
const itemActs = (item) => (["blocked", "failed"].includes(item.kind) ? taskActs(item.task, item.kind) : rowActs(item));

function deskTable(model, selected) {
  if (!model.open.length && !model.stopped.length && !model.todos.length && !model.other.length) return "";
  const itemLine = (item) => item.kind === "memories" ? html`<tr>
    <td>${stateWord("memories")}</td><td class="mono"></td><td></td><td class="q full"><span>${plural(item.candidates?.length ?? 0, "memory candidate")}: ${(item.candidates || []).map((candidate) => candidate.name).join(", ")}</span>${fold(review)}</td>
    <td class="recw">${review}</td><td class="r">${timeTag(item.since)}</td></tr>` : html`<tr>
    <td>${stateWord(item.kind === "prompt" ? "prompt" : item.kind)}</td><td class="mono">${item.kind === "batch" ? html`<a href="/batches/${item.id}">${taskLabel(item.key)}</a>` : ["blocked", "failed"].includes(item.kind) ? html`<a href="/tasks/${item.task}" title="task ${item.task}">${taskLabel(item.key)}</a>` : taskLabel(item.key)}</td><td>${item.step || ""}</td>
    <td class="q full t-${role(item.kind)}">${item.task ? prsTag({}, item.task) : ""}<span>${autolinked(item.what, repoOf(item.task))}</span>${fold(itemActs(item))}</td><td class="recw">${itemActs(item)}</td><td class="r">${timeTag(item.since)}</td></tr>`;
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

// On a phone the Add button sits beside the capsule, or alone where the capsule hides.
function capsule(model) {
  const n = model.open.length;
  const words = capsuleWords(n, Boolean(state.failed));
  const answer = n && deskView() === "needs" ? html`<a class="capsule ${words.off ? "off" : ""}" href="/asks/${model.first.id}">${icon(words.icon, "s20")}${words.text}</a>` : "";
  return html`<div class="fade"></div><div class="dock">${answer}${addButton("fab")}</div>`;
}

// Screens
export function deskPhone(withCapsule = true) {
  const { model, views, banner, rows } = deskBody(false);
  const head = countHead(model.open.length);
  return html`<div class="top"><span class="fac">${state.data.views.factory}</span>${connection()}</div>
    <div class="panel" role="region" aria-label="Desk"><h1 class="sr">Desk</h1><div class="scroll ${withCapsule ? "desk-end" : ""}">
      <div class="count ${head.n ? "" : "quiet"}">${head.n ? html`<span class="n num">${head.n}</span>` : ""}<span class="w">${head.words}<span>${countLine(model)}</span></span></div>
      ${nextCard(model)}${views}${banner}${rows}</div>${withCapsule ? capsule(model) : ""}</div>`;
}

// Records the visit once, after Since you looked is drawn, at the time the first live read began.
export async function seen() {
  if (state.since.sent || !state.since.opened || state.failed) return;
  state.since.sent = true;
  // As the Herdr Desk: one try. A visit not recorded shows the same events again next time.
  const { status, body } = await post("/api/seen", { at: state.since.opened }).catch((error) => ({ status: 0, body: { error: error.message } }));
  if (status !== 200) console.warn(`priory: could not record the visit: ${body.error || status}`);
}

export function deskClick(action, target) {
  if (action === "todo-done") markDone(target);
  else if (action === "row-act") rowAct(target);
  else return false;
  return true;
}
