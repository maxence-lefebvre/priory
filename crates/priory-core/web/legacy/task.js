// The task page's legacy drawing and actions.
import { answeredAsks, askHeading, confidenceNote, courseNote, openAsksOf } from "../asks.js";
import { backTo } from "../pages.js";
import { taskLabel, workStep } from "../tasks.js";
import { taskOf, taskSections } from "./sections.js";
import { ICONS, autolinked, html, icon, raw, role, state, tile, timeTag } from "./shared.js";
import { ENDED, taskActs } from "./taskacts.js";
import { terminalSection } from "./terminal.js";


// A task's page: its state, reason, actions, a box to say something to its agent, then its brief,
// plan, pull request and trace.
export function taskPage(id) {
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
    <p class="ctx">${tile(task.state, "sm")}<span class="mono">${taskLabel(task.key)}</span><span>${task.state}</span><span>${workStep(task)}</span><span>${task.lane || ""}</span><span>${task.band || ""}</span>${task.batch == null ? "" : html`<a href="/batches/${task.batch}">batch ${task.batch}</a>`}${task.auto_answer ? html`<span>Answers asks itself at <b>${task.auto_answer}</b></span>` : ""}</p>
    <h1 class="question">${autolinked(task.tldr || task.brief?.title || task.brief?.request?.split("\n")[0] || taskLabel(task.key), task.repo)}</h1>
    ${task.reason ? html`<p class="text t-${role(task.state)}">${autolinked(task.reason, task.repo)}</p>` : ""}
    ${taskAsks(task)}${taskActs(task.id, task.state)}${say}${taskAnswered(task)}</div>${sections}${terminalSection(task)}</div>`;
}

// The task's open asks, each with a way to answer it (DESIGN.md, Task detail).
function taskAsks(task) {
  const asks = openAsksOf(state.data.asks, task);
  if (!asks.length) return "";
  return html`<ul class="taskasks" aria-label="Open asks">${asks.map((ask) => html`<li>${tile("ask", "sm")}<span>${askHeading(ask)}<span class="age">${timeTag(ask.since)}</span></span><a class="btn primary" href="/asks/${encodeURIComponent(ask.id)}">Answer…</a></li>`)}</ul>`;
}

// Who answered, as the answered row's icon and what a screen reader reads of it.
const ANSWERERS = { auto: ["cog", "Answered by auto"], autopilot: ["cog", "Answered by autopilot"], agent: ["cog", "Answered by an agent"], session: ["terminal", "Answered from a session"], person: ["user", "Answered by a person"] };

const confidenceBars = (row) => html`<span class="bars" role="img" aria-label="Confidence ${row.confidence}">${[1, 2, 3].map((n) => raw(n <= row.bars ? '<i class="on"></i>' : "<i></i>"))}</span>`;

// The task's answered asks, newest first, each with who answered. An auto answer opens to its
// confidence, rationale and trade-off, and says how to change course (DESIGN.md, Task detail).
function taskAnswered(task) {
  const rows = answeredAsks(task.asks || [], task.id);
  if (!rows.length) return "";
  const live = !ENDED.includes(task.state);
  const row = (ask) => {
    const [name, label] = ANSWERERS[ask.answerer] || ["check", "Answered"];
    const by = !ask.by ? "" : ask.answerer === "session" ? html` by session <span class="mono">${ask.by.slice(8)}</span>`
      : ask.answerer === "person" ? ` by ${ask.by}` : html` by <b>${ask.by}</b>`;
    const answered = html`<p class="said">Answered <b>${ask.answer}</b>${by}${ask.recommended == null ? "" : `. Recommended ${ask.recommended}.`}</p>`;
    const head = raw(`<svg class="ico who" viewBox="0 0 24 24" role="img" aria-label="${label}">${ICONS[name]}</svg>`);
    if (ask.answerer !== "auto" && ask.answerer !== "autopilot") {
      const sure = ask.confidence ? html`<p class="sure dim">${confidenceBars(ask)}${ask.confidence}${ask.rationale ? `: ${ask.rationale}` : ""}</p>` : "";
      return html`<li>${head}<div><p class="aq">${ask.title}</p>${answered}${sure}</div>${timeTag(ask.at)}</li>`;
    }
    const sure = ask.confidence ? html`<p class="sure">${confidenceBars(ask)}<span><b>${ask.confidence}</b> ${confidenceNote(ask.answerer === "auto" ? task.auto_answer : null)}</span></p>` : "";
    const why = [["Why", ask.rationale], ["Autopilot", ask.why], ["Trade-off", ask.tradeoff], ["The ask", ask.question !== ask.title ? ask.question : null]].filter(([, text]) => text);
    const reasons = why.length ? html`<dl class="why">${why.map(([term, text]) => html`<dt>${term}</dt><dd>${text}</dd>`)}</dl>` : "";
    return html`<li class="auto">${head}<div><p class="aq">${ask.title}</p>${answered}${sure}${reasons}
      <p class="course"><span>${courseNote(ask.answerer, ask.confidence)}</span>${live ? html`<a href="#say">Say to the agent</a>` : ""}<a href="/asks/${encodeURIComponent(ask.id)}">Open ask ${ask.id}</a></p></div>${timeTag(ask.at)}</li>`;
  };
  return html`<section class="answered" aria-labelledby="answered-head"><div class="ah"><h2 id="answered-head">Answered asks</h2><span class="meta">${rows.length}${rows.length > 1 ? ", newest first" : ""}</span></div>
    <ol class="arows">${rows.map(row)}</ol></section>`;
}
