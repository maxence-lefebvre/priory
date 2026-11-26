// The batch page's legacy drawing and actions.
import { render } from "../app.js";
import { editBody, issueName, laneLayout, momentWords, stages, successors } from "../batches.js";
import { plural } from "../desk.js";
import { backTo } from "../pages.js";
import { taskLabel } from "../tasks.js";
import { api, refresh } from "./data.js";
import { age, ago, autolinked, focusId, html, icon, layout, place, post, role, routeKey, tile, timeTag, track, writeRefusal } from "./shared.js";
import { state } from "./state.js";

// The batch the page or the go ask shows, and the go ask's plan edits: the entry whose menu is open,
// the drop armed by its first tap, whether an edit is in flight, and what the last one said.
Object.assign(state, { batch: null, planEdit: { at: null, batch: null, open: null, armed: null, busy: false, note: null } });

// A batch as /api/batches/{id} serves it: by id on its page, by its planning task's key in a go ask.
// `version` is the go ask's revision: a plan read for another revision is never drawn beside it, so a
// Go never answers a plan the page did not show. A refresh marks it stale; a newer read supersedes.
let batchCalls = 0;
export function batchOf(which, version = null) {
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

// An open menu or an armed Drop holds for one batch at one plan revision; the last edit's note, for the batch.
export const planEditAt = (batch) => `${batch.id}:${batch.plan_revision}`;
export function planEditOf(batch) {
  const edit = state.planEdit;
  if (edit.at === planEditAt(batch)) return edit;
  return { open: null, armed: null, busy: false, note: edit.batch === batch.id ? edit.note : null };
}

// The plan's splits as the planner sent them, each kept only when it names an issue.
export const splitsOf = (batch) => (batch.splits || []).filter((split) => typeof split?.issue === "string");

export const laneKey = () => html`<p class="key-line"><span class="moment"><i class="mk stack" aria-hidden="true"></i>stacks: starts once the one above waits on review</span><span class="moment"><i class="mk" aria-hidden="true"></i>starts once the one above merges</span></p>`;

// The lane graph (DESIGN.md, Lane graph): one row per entry in start order. Tiles and numbers sit in
// each row; drawLanes adds the lines once the rows are laid out.
export function laneGraph(batch, editable) {
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
export function drawLanes(app) {
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

export function closesSection(batch) {
  const closes = batch.closes || [];
  if (!closes.length) return "";
  return html`<section class="sect" id="closes"><div class="sh">${icon("check", "s15")}Closes ${closes.length} ${closes.length > 1 ? "issues" : "issue"}</div><ol class="ilist">${closes.map((close) => html`<li>${icon(close.closed ? "check" : "circle-dashed", "s15")}<span><span class="mono">${close.issue}</span> ${autolinked(close.why, batch.repo)}${close.evidence ? html` ${autolinked(close.evidence, batch.repo)}` : ""}${close.closed ? html`. Closed ${timeTag(close.closed, ago(close.closed))}.` : ""}</span></li>`)}</ol></section>`;
}

export function splitsSection(batch) {
  const splits = splitsOf(batch);
  if (!splits.length) return "";
  const parts = (issue) => batch.entries.filter((entry) => entry.issue === issue).map(issueName).join(", ");
  return html`<section class="sect" id="splits"><div class="sh">${icon("layers", "s15")}Splits ${splits.length} ${splits.length > 1 ? "issues" : "issue"}</div><ol class="ilist">${splits.map((split) => html`<li>${icon("layers", "s15")}<span><span class="mono">${split.issue}</span> into ${parts(split.issue)}. ${autolinked(split.why || "", batch.repo)}</span></li>`)}</ol></section>`;
}

// The batch page (DESIGN.md, Batch page): /batches/<id>.
export function batchPage(id) {
  const [href, label] = backTo(place.entry.from ?? null);
  const back = html`<div class="top"><a class="back" href="${href}">${icon("chevron-left", "s22")}${label}</a></div>`;
  const batch = batchOf(id);
  if (!batch) {
    const failed = state.batch?.error;
    return html`${back}<h1 class="sr">Batch ${id}</h1><p class="empty">${failed ? html`Could not load batch ${id}: ${failed}. <code>priory batch show ${id}</code> shows it.` : `Loading batch ${id}…`}</p>`;
  }
  return html`${back}<div class="scroll"><div class="ask bhead">
    <p class="ctx">${tile("batch", "sm")}<a class="mono" href="/tasks/${batch.task}" title="task ${batch.task}">${taskLabel(batch.key)}</a><span>${batch.state}</span><span>${batch.lane || ""}</span>${batch.went ? html`<span>went ${timeTag(batch.went, ago(batch.went))} by ${batch.went_by}</span>` : ""}<span>plan rev ${batch.plan_revision}</span></p>
    <h1 class="question">${batch.title || taskLabel(batch.key)}</h1>${stageBar(batch)}</div>
    <section class="sect" id="entries"><div class="sh">${icon("layers", "s15")}${plural(batch.entries.length, "entry", "entries")} in start order</div>${laneKey()}${laneGraph(batch, false)}</section>
    ${closesSection(batch)}${splitsSection(batch)}</div>`;
}

// One tap of an entry's menu. A drop that takes successors with it arms first and names them.
export async function editPlan(target) {
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

// Leaving the page closes an entry's menu and disarms its Drop.
export const closePlanEdit = () => { Object.assign(state.planEdit, { open: null, armed: null }); };

export function batchBefore() {
  if (state.renderedKey && state.renderedKey !== routeKey()) closePlanEdit();
}

export function batchClick(action, target) {
  if (action === "entry-menu" && state.batch?.body) {
    const edit = planEditOf(state.batch.body);
    const open = edit.open === target.dataset.entry ? null : target.dataset.entry;
    state.planEdit = { ...edit, at: planEditAt(state.batch.body), batch: state.batch.body.id, open, armed: null };
    render();
  } else if (action === "batch-edit") editPlan(target);
  else return false;
  return true;
}
