// A task's sections, as the ask drawer and the task page draw them.
import { render } from "../app.js";
import { planParts, runnerOf } from "../asks.js";
import { progressView } from "../desk.js";
import { eventWords, handled } from "../inbox.js";
import { taskRead } from "../pages.js";
import { tokenText } from "../runs.js";
import { checkState, prStack, stateWords } from "../stack.js";
import { triageWords } from "../tasks.js";
import { api } from "./data.js";
import { autolinked, html, icon, md, prs, role, state, taskName, tile, timeTag } from "./shared.js";


// The ask's task as /api/tasks/{id} serves it, read once per ask and again after each refresh.
let taskCalls = 0;
export function taskOf(ask) {
  if (!ask.task || runnerOf(ask)) return null;
  const { task, read } = taskRead(state.task, ask.task);
  if (!read) return task.body;
  const call = ++taskCalls;
  state.task = { ...task, call };
  api(`/api/tasks/${ask.task}`).then((body) => {
    if (state.task?.call !== call) return;
    state.task.body = body;
    render();
  }, (error) => {
    if (state.task?.call !== call) return;
    state.task.error = error.message;
    render();
  });
  return state.task.body;
}

// Brief, Plan, PR and Trace, each left out when the task lacks it. Agent text shows with its autolinks.
export function taskSections(task) {
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
  const file = task.plan_file;
  if (file) {
    const [first, ...others] = planParts(file).map((part) => ({
      meta: html`<span class="mono" title="${part.path || ""}">${part.from}</span>`,
      body: html`${md(part.text, task.repo)}${part.truncated ? html`<p class="note">${icon("file-text", "s15")}<span>Showing the first 64 KB. Read the rest at ${part.path || part.from}.</span></p>` : ""}`,
    }));
    sections.push(["plan", "Plan", sect("plan", "check", "Plan", first.meta, html`${first.body}${others.map(({ meta, body }) => html`<div class="sh">${icon("check", "s15")}Plan<span class="meta">${meta}</span></div>${body}`)}`)]);
  } else if (task.plan) sections.push(["plan", "Plan", sect("plan", "check", "Plan", `${task.plan.step}, run ${task.plan.run}`, md(task.plan.summary, task.repo))]);
  const pr = task.pull_request;
  if (pr) {
    sections.push(["pr", "PR", sect("pr", "workflow", `#${pr.number} ${pr.title || ""}`, stateWords(pr.review),
      html`<p class="meta">${pr.branch} into ${pr.base}, head <span class="mono">${String(pr.head_sha).slice(0, 7)}</span>, ${pr.state}</p>
      <ol class="hist pair">${pr.checks.map((check) => html`<li>${tile(checkState(check.conclusion), "sm")}<span>${check.name}</span><span>${check.conclusion ? stateWords(check.conclusion) : "running"}</span></li>`)}</ol>${prStackList(task.stack, pr)}${triageList(task.triage)}`)]);
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

const INBOX_SHOWN = 20;
// A prompt's state as a tile: delivered, waiting to be typed, being typed, or withdrawn.
const INBOX_TILES = { confirmed: "done", queued: "queued", attempted: "running", uncertain: "prompt", withdrawn: "dropped" };

const TRIAGE_TILES = { trivial: "ready", "nothing-to-do": "dropped", "persons-call": "ask" };

// How the agent sorted each review feedback item: trivial, nothing-to-do or persons-call.
function triageList(triage) {
  if (!triage?.length) return "";
  const label = (item) => item.replace(/^https:\/\/github\.com\//, "");
  const named = (item) => (item.startsWith("https://") ? html`<a class="autolink" href="${item}" rel="noopener noreferrer" target="_blank">${label(item)}</a>` : item);
  return html`<p class="meta">Feedback, as the agent sorted it</p><ol class="hist pair" aria-label="Review feedback items">${triage.map((entry) => html`<li>${tile(TRIAGE_TILES[entry.class] || "queued", "sm")}<span>${named(entry.item)}</span><span>${triageWords(entry.class)}</span></li>`)}</ol>`;
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
