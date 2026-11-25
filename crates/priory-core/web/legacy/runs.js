// The runs page's legacy drawing and actions.
import { RUN_FACETS, RUN_VIEWS, duration, runState, runViews, tokenText } from "../runs.js";
import { filtered, olderPart, taskLabel } from "../tasks.js";
import { facet, facetPicks, facetQuery, html, icon, listView, olderRow, savedViews, state, stateWord, tile, timeTag } from "./shared.js";


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

export function runsBody(wide) {
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
