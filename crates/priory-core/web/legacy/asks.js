// The asks page's legacy drawing and actions.
import { render } from "../app.js";
import { ARM_MS, ASK_VIEWS, answeredBy, askBody, askGroups, askHeading, dismissable, optionKey, planCut, planUnread, planWait, plansFirst, press, routedMark, routedNote, runnerOf, runsCommand, seenAgo, sentNote, shownGroups, shrunkHead, snoozeShortcut, snoozed, submitted, writeNote, writeShown } from "../asks.js";
import { plural } from "../desk.js";
import { emptyDetail } from "../inbox.js";
import { taskRead } from "../pages.js";
import { go } from "../router.js";
import { taskLabel } from "../tasks.js";
import { batchOf, closesSection, laneGraph, laneKey, planEditOf, splitsOf, splitsSection } from "./batch.js";
import { refresh, signOut } from "./data.js";
import { fold, openAsks } from "./desk.js";
import { connection } from "./frame.js";
import { taskOf, taskSections } from "./sections.js";
import { age, ago, autolinked, clock, date, facet, facetPicks, facetQuery, faceted, focusKey, html, icon, layout, listSearch, loadOlder, md, olderRow, post, raw, savedViews, selectedAsk, shownFacets, state, stateWord, tile, timeTag, today } from "./shared.js";

// The open asks the list shows, so previous, next and the ask after an answer stay in its facets.
const listedAsks = () => faceted(openAsks(), facetPicks());

const routedTag = (ask) => routedMark(ask) ? html`<span class="tag routed">${routedMark(ask)}</span>` : "";
// Only another runner's ask carries its runner. A phone row's tag keeps the name, in warn text while
// the runner is away, and says "last seen" on line 3.
export const runnerTag = (ask, full = true) => {
  const away = seenAgo(ask, Date.now(), age);
  if (!runnerOf(ask)) return "";
  return full ? html`<span class="tag runner">${runnerOf(ask)}${away ? `, ${away}` : ""}</span>` : html`<span class="tag runner ${away ? "t-warn" : ""}">${runnerOf(ask)}</span>`;
};
export const sentLine = (ask) => {
  const note = sentNote(ask);
  return note ? html`<span class="${note.refused ? "t-warn" : "t-action"}">${note.text}</span>` : "";
};
const byLine = (ask) => answeredBy(ask.answered_by) ? ` by ${answeredBy(ask.answered_by)}` : "";

// Rows
export function askRow(ask, current) {
  const options = ask.options || [];
  const line3 = ask.state === "answered"
    ? html`<span>Answered <b>${ask.answer}</b>${byLine(ask)}</span>`
    : ask.state === "withdrawn"
      ? html`<span>Withdrawn${ask.withdrawn_why ? `: ${ask.withdrawn_why}` : ""}</span>`
      : snoozed(ask, Date.now()) ? html`<span>${icon("clock", "s15")}${snoozedUntil(ask)}</span>`
        : sentNote(ask) ? sentLine(ask)
        : ask.recommendation ? html`<span>Recommends <b>${ask.recommendation}</b></span>` : html`<span>${plural(options.length, "option")}</span>`;
  const when = ask.state === "answered" && ask.answered_at
    ? timeTag(ask.answered_at)
    : timeTag(ask.since);
  return html`<a class="row ${ask.state !== "open" ? "past" : ""}" href="/asks/${ask.id}" aria-current="${current ? "true" : "false"}">
    ${tile(ask.state === "open" ? "ask" : ask.state)}
    <span class="l1"><span class="mono">${taskLabel(ask.key)}</span><span>${ask.step}</span>${ask.command ? html`<span class="tag">Command</span>` : ""}${runnerTag(ask, false)}${routedTag(ask)}<span class="age">${when}</span></span>
    <span class="l2">${askHeading(ask)}</span>
    <span class="l3">${ask.state === "open" && !sentNote(ask) && seenAgo(ask, Date.now(), age) ? html`<span class="t-warn">${runnerOf(ask)} ${seenAgo(ask, Date.now(), age)}</span>` : ""}${line3}</span>
    <span class="chev">${icon("chevron-right", "s15")}</span></a>`;
}

const snoozedUntil = (ask) => `Snoozed until ${today(ask.snoozed_until) ? clock(ask.snoozed_until) : date(ask.snoozed_until)}`;

// Asks list
export function askView() {
  const name = new URLSearchParams(listSearch()).get("view");
  return ASK_VIEWS.includes(name) ? name : "open";
}

export function askTableRow(ask, selected) {
  const answered = ask.state === "answered";
  const when = timeTag(answered && ask.answered_at ? ask.answered_at : ask.since);
  const status = answered ? ask.answer + byLine(ask) : ask.state === "withdrawn" ? ask.withdrawn_why || "" : snoozed(ask, Date.now()) ? snoozedUntil(ask) : sentNote(ask) ? sentLine(ask) : null;
  const folded = status ?? (ask.recommendation ? html`Recommends <b>${ask.recommendation}</b>` : "");
  return html`<tr data-href="/asks/${ask.id}" aria-current="${String(ask.id) === selected ? "true" : "false"}">
    <td>${stateWord(ask.state === "open" ? "ask" : ask.state)}</td><td class="mono">${taskLabel(ask.key)}</td><td>${ask.step}</td><td class="q"><span>${askHeading(ask)}</span>${runnerTag(ask)}${routedTag(ask)}${fold(folded)}</td>
    <td class="recw">${status ?? ask.recommendation ?? ""}</td><td class="r">${when}</td></tr>`;
}

export function asksBody(wide) {
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

// One button per option, as the detail and the answer sheet draw them. The first tap arms an option;
// an armed command option also shows its command.
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
    const label = armed === option ? armedLabel(option, runs) : html`<span>${option}${notes ? html`<small>${notes}</small>` : ""}</span>`;
    const button = html`<button class="opt ${recommended ? "rec-opt" : ""} ${chosen ? "chosen" : ""} ${armed === option ? "armed" : ""}" data-option="${i}" data-answer="${i}" aria-disabled="${off || pending ? "true" : "false"}" ${locked ? raw('aria-describedby="answer-locked"') : ""}>
      ${chosen ? icon("check", "s20") : ""}${label}${wide ? html`<kbd>${i + 1}</kbd>` : ""}</button>`;
    return armed === option && runs ? html`${button}${commandBlock(ask, bound)}` : button;
  });
}

// An armed option: what the second tap does, the option, and the ring that drains over ARM_MS.
const armedLabel = (option, runs) =>
  html`<span><small>Tap again to ${runs ? "run" : "send"}</small>Confirm: ${option}<small>Cancels in ${ARM_MS / 1000} s</small></span><span class="drain" aria-hidden="true"></span>`;

export function askDetail(id, wide) {
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
  const shrunk = shrunkHead(ask);
  const header = html`<div class="ahead">
    ${wide ? html`<span class="ctx">${tile("ask", "sm")}<span>ask <span class="mono">#${ask.id}</span></span><span>revision ${ask.revision}</span><span>${timeTag(ask.since)}</span></span>` : html`<a class="back" href="${back[0]}" aria-label="${back[1]}">${icon("chevron-left", "s22")}<span>${back[1]}</span></a><span class="aq" aria-hidden="true"><b>${shrunk.question}</b><small>${shrunk.place}</small></span>`}
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
  const planError = planErrorOf(ask);
  const lockedNote = locked === "offline" ? html`<p class="note" id="answer-locked">${icon("unplug", "s15")}<span>Reconnect to answer.</span></p>`
    : locked ? html`<p class="note" id="answer-locked">${icon("layers", "s15")}<span>${planWait(ask, planError)}</span></p>` : cutNote(ask);
  const shown = writeShown(state.writing.has(ask.id), locked, Boolean(pending));
  const write = shown === "form"
    ? html`<form class="writeform" data-action="send-text"><label class="sr" for="draft">Your answer</label><textarea id="draft" name="text" rows="3">${state.drafts.get(ask.id) || ""}</textarea><button class="btn primary" ${pending ? raw('aria-disabled="true"') : ""}>Send</button></form>`
    : html`<button class="ghost write" data-action="write" aria-disabled="${shown === "disabled" ? "true" : "false"}">${icon("pencil", "s15")}Write an answer</button>`;
  const busy = pending ? "true" : "false";
  const snoozeButtons = !ask.task || runnerOf(ask) || locked === "offline"
    ? ""
    : ask.snoozed_until
      ? html`<button class="ghost" data-action="unsnooze" aria-disabled="${busy}">Unsnooze</button>`
      : html`<button class="ghost" data-action="snooze" data-for="1h" aria-disabled="${busy}">Snooze 1 h</button><button class="ghost" data-action="snooze" data-tomorrow="9" aria-disabled="${busy}">Snooze until tomorrow 09:00</button>`;
  const editable = Boolean(batch) && ask.state === "open" && batch.state === "asked" && !locked;
  const editPlanLink = editable ? html`<a class="ghost" href="#batchplan">${icon("pencil", "s15")}Edit the plan</a>` : "";
  const more = html`<div class="more">${editPlanLink}${write}${locked || !dismissable(ask) ? "" : html`<button class="ghost" data-action="dismiss" aria-disabled="${busy}">Dismiss</button>`}${snoozeButtons}</div>`;
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
  // A long untitled question heads the detail with a short title and reads whole at the top of Why.
  const question = autolinked(askHeading(ask), ask.repo);
  const body = askBody(ask);
  const top = html`<div class="ask">
    <a class="ctx" href="${ask.task && !owner ? `/tasks/${ask.task}` : "/asks"}" title="${ask.task && !owner ? `task ${ask.task}` : ""}">${wide ? "" : tile("ask", "sm")}<span class="mono">${taskLabel(ask.key)}</span><span>${ask.step}</span><span>${ask.lane || ""}</span>${ask.command ? html`<span class="tag">Command</span>` : ""}${runnerTag(ask)}${wide ? "" : html`<span>${timeTag(ask.since)}</span><span>revision ${ask.revision}</span>`}</a>
    ${ask.risk_estimate ? html`<p class="note">${icon("lock", "s15")}<span>${ask.risk_estimate.line}</span></p>` : ""}
    ${layout() === "compact" ? html`<h1 class="question">${question}</h1>` : html`<h2 class="question">${question}</h2>`}${decided}</div>`;
  // A go ask reads its batch's plan first; the planning task's own plan result says the same.
  const planned = batch ? [
    ["batchplan", "Plan", html`<section class="sect" id="batchplan"><div class="sh">${icon("layers", "s15")}Plan, ${plural(batch.entries.length, "task")} in start order<span class="meta">rev ${batch.plan_revision}</span></div>${laneKey()}${laneGraph(batch, editable)}${planEditOf(batch).note ? html`<p class="note" role="status">${planEditOf(batch).note}</p>` : ""}</section>`],
    ...(batch.closes?.length ? [["closes", "Closes", closesSection(batch)]] : []),
    ...(splitsOf(batch).length ? [["splits", "Splits", splitsSection(batch)]] : []),
  ] : [];
  const task = taskOf(ask);
  const sectionsRead = taskSections(task).filter(([key]) => key !== "plan" || (!batch && !planUnread(ask, task)));
  const lead = batch || !plansFirst(ask) ? [] : sectionsRead.filter(([key]) => key === "plan");
  const read = sectionsRead.filter((section) => !lead.includes(section));
  const sections = [...planned.map(([key, label]) => [key, label]), ...lead.map(([key, label]) => [key, label]), ["why", "Why"], ...read.map(([key, label]) => [key, label]), ["history", "History"], ...(ask.command ? [["command", "Command"]] : [])];
  const jump = html`<nav class="jump" aria-label="Sections">${sections.map(([key, label], i) => html`<a href="#${key}" aria-current="${i === 0 ? "true" : "false"}">${label}</a>`)}</nav>`;
  const why = html`<section class="sect" id="why"><div class="sh">${icon("message-text", "s15")}What the agent asks<span class="meta">${ask.run ? html`run <span class="mono">${ask.run}</span>` : ""}</span></div>
    ${body ? html`<p class="asked">${autolinked(body, ask.repo)}</p>` : ""}
    ${ask.text || !body ? md(ask.text || ask.question, ask.repo) : ""}
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
    ? html`<div class="sending" role="status"><span>Sending <b>"${pending.text}"</b></span></div>`
    : "";
  const barArmed = armed !== null && armed === recommended;
  const bar = !wide && !pending && ask.state === "open" && options.length
    ? html`<div class="abar" hidden>${barArmed && runsCommand(ask, recommended) ? commandBlock(ask, bound) : ""}<button class="btn secondary" data-action="options">${icon("chevron-up", "s15")}${plural(options.length, "option")}</button>
      <button id="bar-answer" class="btn primary ${barArmed ? "armed" : ""}" data-answer="${options.indexOf(recommended)}" aria-disabled="${locked ? "true" : "false"}">${barArmed
        ? armedLabel(recommended, runsCommand(ask, recommended))
        : html`<span>${recommended}</span><small>${[recommended === ask.recommendation ? "Recommended" : "First option", runsCommand(ask, recommended) && "Runs a command"].filter(Boolean).join(". ")}</small>`}</button>
      ${locked === "offline" ? html`<p class="note">${icon("unplug", "s15")}<span>Reconnect to answer.</span></p>` : locked ? html`<p class="note">${icon("layers", "s15")}<span>${planWait(ask, planError)}</span></p>` : cutNote(ask)}</div>`
    : "";
  const reading = read.map(([, , section]) => section);
  const plan = [...planned, ...lead].map(([, , section]) => section);
  if (wide) return html`<div class="pinned">${header}${top}${sending}</div><div class="scroll" data-sections>${jump}${plan}${why}${reading}${history}${command}</div>`;
  return html`${header}<div class="scroll" data-sections>${top}${jump}${plan}${why}${reading}${history}${command}</div>${bar}${sending}`;
}

const planErrorOf = (ask) => (ask.approves_plan ? taskRead(state.task, ask.task).task : state.batch)?.error;

const cutNote = (ask) => {
  const cut = planCut(ask, state.task?.id === ask.task ? state.task.body : null);
  return cut ? html`<p class="note warn">${icon("file-text", "s15")}<span>${cut}</span></p>` : "";
};

// Why the options of an ask are disabled: offline, or an ask whose plan is not read yet for this
// revision, so no answer approves a plan the page did not show.
export function lockOf(ask) {
  if (state.failed) return "offline";
  if (runnerOf(ask)) return null;
  if (planUnread(ask, state.task?.id === ask.task ? state.task.body : null)) return "plan";
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
export const askOnScreen = () => state.data.asks.find((ask) => String(ask.id) === selectedAsk());

// An answer leaves at once: a tapped option was already confirmed by its second tap, and Send is a
// deliberate press of its own. "Sending" shows only while the request is in flight.
export function answer(ask, text, written, run = false) {
  disarm();
  if (state.pending) return;
  state.notices.delete(ask.id);
  state.pending = { id: ask.id, text };
  send(ask, state.pending, written, run);
}

// The first tap on an option arms it; the second, within ARM_MS, sends it.
function arm(armed) {
  disarm();
  state.armed = { ...armed, timer: setTimeout(() => { state.armed = null; render(); }, ARM_MS) };
  render();
}

export function disarm() {
  if (!state.armed) return;
  clearTimeout(state.armed.timer);
  state.armed = null;
}

async function send(ask, pending, written, run) {
  render();
  const body = { [written ? "text" : "option"]: pending.text, revision: ask.revision };
  if (run) body.run = true;
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
export async function next(ask) {
  const before = listedAsks().map((candidate) => candidate.id);
  await refresh();
  if (selectedAsk() !== String(ask.id)) return;
  const left = listedAsks().filter((candidate) => candidate.id !== ask.id);
  const following = left.find((candidate) => before.indexOf(candidate.id) > before.indexOf(ask.id)) || left[0];
  go(following ? `/asks/${encodeURIComponent(following.id)}` : state.listUrl);
}

export async function snooze(ask, button) {
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

export function act(target) {
  const ask = askOnScreen();
  if (!ask || target.getAttribute("aria-disabled") === "true") return;
  if (target.dataset.answer !== undefined) {
    if (state.pending) return;
    const pressed = press(state.armed, ask, ask.options[Number(target.dataset.answer)], Date.now());
    if (pressed.send) return answer(ask, pressed.send.text, false, pressed.send.run);
    if (pressed.arm) arm(pressed.arm);
    return;
  }
  const action = target.dataset.action;
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

// The answer sheet (DESIGN.md, Answer while reading): every option of the ask on screen, opened by the bar's "N options".
export const answerSheet = () => document.getElementById("answer");

function openSheet() {
  if (!askOnScreen()) return;
  renderSheet();
  answerSheet().showModal();
}

// Closes itself once the ask is gone, answered, or its answer is sending.
export function renderSheet() {
  const sheet = answerSheet();
  const ask = state.data && askOnScreen();
  if (!ask || ask.state !== "open" || state.pending) return sheet.close();
  const focused = document.activeElement?.closest?.("#answer") ? focusKey(document.activeElement) : null;
  sheet.innerHTML = html`<div class="shead"><h2 id="answer-title">Answer ask #${ask.id}<small>revision ${ask.revision}</small></h2><button class="iconbtn" data-action="sheet-close" aria-label="Close">${icon("x", "s20")}</button></div>
    <div class="opts" role="group" aria-label="Options">${optionList(ask, false)}</div>${cutNote(ask)}`.text;
  if (focused) sheet.querySelector(focused)?.focus();
}

export function asksPhone() {
  const { views, rows } = asksBody();
  return html`<div class="top"><h1>Asks</h1><span class="sp"></span>${connection()}</div>
    <div class="panel" role="region" aria-label="Asks"><div class="scroll">${views}${rows}</div></div>`;
}

export function drawer(wide, panels) {
  const id = selectedAsk();
  const body = id ? askDetail(id, wide) : html`<p class="empty">${emptyDetail(panels)}</p>`;
  return html`<aside class="drawer" aria-label="Ask">${body}</aside>`;
}

export function asksClick(action, target) {
  if (action === "sheet-close") answerSheet().close();
  else if (target.dataset.answer !== undefined || ["options", "write", "dismiss", "snooze", "unsnooze"].includes(action)) act(target);
  else return false;
  return true;
}

export function asksInput(target) {
  const ask = target.id === "draft" && askOnScreen();
  if (ask) state.drafts.set(ask.id, target.value);
}

export function asksSubmit(action) {
  if (action !== "send-text") return false;
  const ask = askOnScreen();
  const text = (state.drafts.get(ask?.id) || "").trim();
  if (ask && text && writeShown(state.writing.has(ask.id), lockOf(ask), Boolean(state.pending)) === "form") answer(ask, text, true);
  return true;
}

// S snoozes the selected ask for 1 h, 1 to 9 press its options. Never inside an input.
export function asksKey(event) {
  if (event.key === "s" || event.key === "S") {
    const body = askOnScreen() && !state.failed && !state.pending && snoozeShortcut(askOnScreen(), Date.now());
    if (body) snooze(askOnScreen(), { dataset: { for: body.for } });
  } else if (selectedAsk() && /^[1-9]$/.test(event.key)) {
    const index = optionKey(event);
    if (index !== null) document.querySelector(`.opt[data-answer="${index}"]`)?.click();
  } else return false;
  return true;
}

answerSheet().addEventListener("click", (event) => { if (event.target === answerSheet()) answerSheet().close(); });
