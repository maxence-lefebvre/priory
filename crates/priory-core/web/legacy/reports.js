// The reports page's legacy drawing and actions.
import { render } from "../app.js";
import { around, dossier, nextPick, refusal } from "../reports.js";
import { refresh, signOut } from "./data.js";
import { connection } from "./frame.js";
import { autolinked, focusId, html, icon, post, state, taskName, tile, writeRefusal } from "./shared.js";


// Reports (DESIGN.md, Reports panel): the report written last, its sections in its own order, picks
// with an Undo, and threads on rows through the report endpoints.
const reportRows = (model) => model.sections.flatMap((section) => section.rows);

export function reportsPage(compact) {
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

// A note under its row, or above the sections once the row has left the report or another task's
// report replaced it.
function noteRow(id, text, task = state.data.report?.report.task) {
  if (task === state.data.report?.report.task && reportRowOf(id)) state.reportNotices.set(id, text);
  else state.reportFlash = text;
}

const reportRowOf = (id) => (state.data.report ? reportRows(dossier(state.data.report, { now: Date.now() })).find((row) => row.id === id) : null);

// Nothing leaves the page until the Undo time passes: a pick wakes the report's agent.
const UNDO_MS = 4000;

export function pick(target) {
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
export function acceptReport() {
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

export async function submitRemark() {
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

export function reportAct(target) {
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

export function reportsClick(action, target) {
  if (!["pick", "pick-undo", "pick-change", "pick-keep", "report-go", "report-next", "report-search", "compose", "compose-cancel", "reply", "resolve", "reopen"].includes(action)) return false;
  reportAct(target);
  return true;
}

export function reportsInput(target) {
  if (target.id === "report-draft" && state.compose) state.compose.text = target.value;
  if (target.id !== "report-query") return;
  state.reportQuery = target.value;
  render();
}

export function reportsSubmit(action) {
  if (action !== "remark") return false;
  submitRemark();
  return true;
}
