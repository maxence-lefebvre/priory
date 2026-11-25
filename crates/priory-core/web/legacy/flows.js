// The flows page's legacy drawing and actions.
import { render } from "../app.js";
import { plural } from "../desk.js";
import { KINDS, edgeStyle, historyRows, lockBadge, nodeShape, timeline, ungatedPairs } from "../flows.js";
import { api, keep, signOut } from "./data.js";
import { connection } from "./frame.js";
import { ICONS, html, icon, raw, state } from "./shared.js";


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
    <text x="${edge.label.x}" y="${edge.label.y}" text-anchor="${edge.label.anchor}" class="felabel"${edge.label.turned ? raw(` transform="rotate(-90 ${edge.label.x} ${edge.label.y})"`) : ""}>${clip(edge.outcome, 14)}</text></g>`;
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
  return html`<section class="sect flow" aria-labelledby="${id}"><h2 class="sh" id="${id}">${workflow.name}<span class="meta num">${plural(workflow.steps.length, "step")}</span></h2>
    <svg class="diagram" viewBox="0 0 ${drawing.width} ${drawing.height}" role="img" aria-label="${drawing.text}">
      <defs><marker id="arrow" viewBox="0 0 10 10" refX="9" refY="5" markerUnits="userSpaceOnUse" markerWidth="10" markerHeight="10" orient="auto-start-reverse"><path d="M0 1L10 5L0 9z" fill="context-stroke"/></marker></defs>
      ${drawing.edges.map(flowEdge)}${drawing.nodes.map(flowNode)}</svg>
    ${flowHistory(historyRows(irreversible ?? [], workflow.name))}
    ${open.length ? html`<div class="unattended" role="note"><p class="t-danger">${icon("lock", "s15")}Reaches an irreversible step with no person's decision:</p><ul>${open.map((entry) => html`<li class="mono">${entry.path.join(" > ")}</li>`)}</ul></div>` : ""}
    <details class="astext"><summary>Steps as text</summary><ol>${workflow.steps.map((step) => html`<li><b class="mono">${step.id}</b>, ${KIND_WORD[step.kind] ?? step.kind}, decided by ${step.decides}. ${step.edges.map((edge) => html`${edge.outcome} leads to ${edge.to} (${BY_WORD[edge.by] ?? edge.by}). `)}</li>`)}</ol></details></section>`;
}

export function flowsPage(compact) {
  const head = compact ? html`<div class="top"><h1>Flows</h1><span class="sp"></span>${connection()}</div>` : html`<div class="lhead"><h1>Flows</h1></div>`;
  const flows = state.data.flows;
  if (!flows && state.flowsError) return html`${head}<p class="empty" role="alert">The flows did not load: ${state.flowsError}. Run <code>priory check --explain</code> meanwhile.</p><p class="empty"><button class="btn secondary" data-action="flows-retry">Retry</button></p>`;
  if (!flows) { setTimeout(loadFlows); return html`${head}<p class="empty" role="status">Loading the flows…</p>`; }
  const body = flows.workflows.length ? html`${flowLegend()}${flows.workflows.map((workflow) => flowSection(workflow, flows.paths, flows.irreversible))}` : html`<p class="empty">This factory has no workflows. Add one under <code>workflows/</code>.</p>`;
  return html`${head}<div class="panel" role="region" aria-label="Flows"><div class="scroll">${body}</div></div>`;
}

export function flowsClick(action) {
  if (action !== "flows-retry") return false;
  state.flowsError = null;
  render();
  return true;
}
