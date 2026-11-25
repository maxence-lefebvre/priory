// The costs page's legacy drawing and actions.
import { render } from "../app.js";
import { SPLITS, attentionView, costTiles, dayBars, money, scoreTable, splitTable } from "../costs.js";
import { taskLabel } from "../tasks.js";
import { searchOf } from "../frame.js";
import { api, cache, refresh, signOut } from "./data.js";
import { connection } from "./frame.js";
import { date, html, icon, layout, place, raw, savedViews, state, timeTag } from "./shared.js";


// Costs (DESIGN.md, Costs panel): what the factory spent this week against last week, and what it
// cost the person, from /api/costs and /api/attention.
export async function loadCosts() {
  const view = state.costs;
  // A read scheduled before a sign-out, or before the connection dropped, does not go.
  if (view.loading || !state.token || !state.data || state.failed) return;
  const by = view.by;
  view.loading = true;
  const slow = setTimeout(() => { view.slow = true; render(); }, 300);
  try {
    const [costs, attention] = await Promise.all([api(`/api/costs?by=${by}`), api("/api/attention")]);
    // A sign-out while the read was in flight replaced the Costs state and drops this read.
    if (view !== state.costs) return;
    state.data.spend = { costs, attention, updated: Date.now() };
    view.error = null;
    cache.setQueryData(["spend"], state.data.spend);
  } catch (error) {
    if (view !== state.costs) return;
    if (error.status === 401) return signOut("The daemon refused this token. Paste the current one.");
    view.error = error instanceof TypeError ? "the connection dropped" : error.message;
  } finally {
    clearTimeout(slow);
    Object.assign(view, { loading: false, slow: false });
  }
  if (view !== state.costs) return;
  render();
  // The person picked another split while this read was in flight.
  if (!view.error && state.data?.spend?.costs.by !== view.by) loadCosts();
}

export function costsPage(compact) {
  const spend = state.data.spend;
  const wide = layout() === "wide";
  const shown = new URLSearchParams(searchOf(place.href)).get("view") === "attention" ? "attention" : "costs";
  const head = compact ? html`<div class="top"><h1>Costs</h1><span class="sp"></span>${connection()}</div>` : html`<div class="lhead"><h1>Costs</h1></div>`;
  const chips = wide ? "" : savedViews([["costs", "Costs", null, shown === "costs"], ["attention", "Attention", null, shown === "attention"]], "/costs");
  const failed = state.costs.error
    ? html`<p class="note warn" role="alert">${icon("circle-x", "s15")}<span>Could not load costs: ${state.costs.error}. Check that the daemon runs on its runner, then retry.</span><button class="ghost" data-action="costs-retry">Retry</button></p>`
    : "";
  const asOf = spend && (state.failed || state.costs.error || Date.now() - spend.updated > 60000) ? html`<p class="note asof"><span>Costs as read ${timeTag(new Date(spend.updated).toISOString(), date(new Date(spend.updated).toISOString()))}.</span></p>` : "";
  let body;
  if (spend) body = wide ? html`<div class="spend"><div>${costsPart(spend.costs)}</div><div>${attentionPart(spend.attention)}</div></div>` : shown === "costs" ? costsPart(spend.costs) : attentionPart(spend.attention);
  else body = state.costs.slow ? html`<div class="sk" aria-hidden="true"><span></span><span></span><span></span></div>` : "";
  return html`${head}<div class="panel" role="region" aria-label="Costs" data-costs>${chips}<div class="scroll">${failed}${asOf}${body}</div></div>`;
}

const TRENDS = { up: "trending-up", down: "trending-down" };
const deltaLine = (delta) => (delta ? html`<span class="delta">${TRENDS[delta.direction] ? icon(TRENDS[delta.direction], "s13") : ""}${delta.text}</span>` : html`<span class="delta">No change to show</span>`);
const statTile = (tile) => html`<div class="stat"><span class="lbl">${tile.label}</span><span class="val num">${tile.value}</span>${"delta" in tile ? deltaLine(tile.delta) : html`<span class="delta">${tile.note}</span>`}</div>`;

function costsPart(costs) {
  const table = splitTable(costs);
  const offline = Boolean(state.failed);
  const pick = html`<label class="by"><span>Split by</span><select data-action="costs-by" aria-disabled="${offline}" ${offline ? raw("disabled") : ""}>${SPLITS.map((by) => html`<option value="${by}" ${by === costs.by ? raw("selected") : ""}>${by}</option>`)}</select></label>`;
  if (!costs.days.length && !costs.split.length) {
    // Scores can land in a window with no cost, so they keep their table and its Split by.
    const scores = scoresPart(costs, pick);
    return html`<p class="empty">No cost in the last two weeks. Costs appear once a model step records its tokens. Price each model in <code>prices.toml</code> in the factory folder.</p>${scores ? html`<section class="sect" aria-labelledby="costs-scores">${scores}</section>` : ""}`;
  }
  const unpriced = table.unpriced.length || table.unknown
    ? html`<div class="note"><p><b>Not in the cost.</b> ${table.unpriced.length ? html`Tokens with no price: ${table.unpriced.map((entry, i) => html`${i ? ", " : ""}<span class="mono">${entry.model}</span> ${entry.tokens} tokens`)}. Add each model to <code>prices.toml</code> to price them.` : ""} ${table.unknown ? `${table.unknown}, so their cost is unknown.` : ""}</p></div>`
    : "";
  return html`<section class="sect" aria-labelledby="costs-week"><h2 class="sh" id="costs-week">This week<span class="meta">against last week</span></h2>
    <div class="stats">${costTiles(costs).map(statTile)}</div>${unpriced}</section>
    <section class="sect" aria-labelledby="costs-days"><h2 class="sh" id="costs-days">Each day</h2>${dayChart(costs)}</section>
    <section class="sect" aria-labelledby="costs-split"><h2 class="sh" id="costs-split">This week by ${costs.by}${pick}</h2>
      <div class="tscroll"><table class="ctbl"><thead><tr><th scope="col">${table.dimension}</th><th scope="col" class="r">Cost</th><th scope="col" class="r">Tokens</th><th scope="col">Notes</th></tr></thead>
      <tbody>${table.rows.map((row) => html`<tr><th scope="row" class="mono">${row.name}</th><td class="r">${row.cost}</td><td class="r">${row.tokens}</td><td class="n">${row.note}</td></tr>`)}</tbody></table></div>${scoresPart(costs)}</section>`;
}

function scoresPart(costs, pick = "") {
  const table = scoreTable(costs);
  if (!table.rows.length) return "";
  const head = ["Pass rate", "pass", "fail", "error", "not applicable"].map((label) => html`<th scope="col" class="r">${label}</th>`);
  return html`<h3 class="sh" id="costs-scores">Scores by ${costs.by}${pick}</h3><p class="cap">The pass rate leaves out errors and not applicable.</p>
    <div class="tscroll"><table class="ctbl stbl" aria-labelledby="costs-scores"><thead><tr><th scope="col">${table.dimension}</th><th scope="col">Scorer</th>${head}</tr></thead>
    <tbody>${table.rows.map((row) => html`<tr><th scope="row" class="mono">${row.name}</th><td class="mono">${row.scorer}</td><td class="r c rate">${row.rate}</td><td class="r c">${row.pass}</td><td class="r c">${row.fail}</td><td class="r c">${row.error}</td><td class="r c">${row.notApplicable}</td><td class="line">${row.line}</td></tr>`)}</tbody></table></div>`;
}

// Bars in percent of the box, so the chart takes any width without stretching its text.
function dayChart(costs) {
  const bars = dayBars(costs);
  const top = Math.max(0, ...costs.days.map((day) => day.cost_usd ?? 0));
  const slot = 100 / bars.length;
  const height = 96;
  const split = bars.findIndex((bar) => bar.current);
  const week = split > 0 ? html`<line class="week" x1="${split * slot}%" x2="${split * slot}%" y1="0" y2="${height + 52}"/><text class="wk" x="${split * slot}%" dx="4" y="10">this week</text>` : "";
  const columns = bars.map((bar, i) => {
    const h = Math.max(bar.fraction * height, 1);
    const x = i * slot;
    return html`<g class="${bar.current ? "cur" : "prev"}"><title>${bar.text}</title>
      <rect x="${x + slot * 0.18}%" y="${16 + height - h}" width="${slot * 0.64}%" height="${h}" rx="2"/>
      <text class="d" x="${x + slot / 2}%" y="${height + 32}" text-anchor="middle"><tspan class="wd">${bar.weekday} </tspan>${bar.date}</text>
      <text class="m" x="${x + slot / 2}%" y="${height + 48}" text-anchor="middle">${bar.merges}</text></g>`;
  });
  return html`<figure class="days"><svg width="100%" height="${height + 56}" role="img" aria-label="Cost per day, last week then this week. ${bars.map((bar) => bar.text).join(". ")}.">
    <text class="axis" x="0" y="10">${top ? money(top) : ""}</text>
    <line class="base" x1="0" x2="100%" y1="${16 + height}" y2="${16 + height}"/>${columns}${week}</svg>
    <figcaption>Bars show each day's cost${split > 0 ? ", and this week starts at the line" : ""}. The figure under each day counts its merges.</figcaption></figure>`;
}

function attentionPart(report) {
  const view = attentionView(report);
  if (!report.acts && !report.tasks.length && !report.waits.acted && !report.waits.open) {
    return html`<p class="empty">No attention counted this week. Your answers, dismissals and snoozes on asks count here, and each task's time once it runs.</p>`;
  }
  const { headline } = view;
  return html`<section class="sect" aria-labelledby="attention-head"><h2 class="sh" id="attention-head">Your attention<span class="meta">this week</span></h2>
    <div class="headline"><span class="val num">${headline.value}</span><span class="w">${headline.label}<span>${headline.note}</span></span></div>
    <div class="stats">${view.waits.map(statTile)}</div></section>
    <section class="sect" aria-labelledby="attention-tasks"><h2 class="sh" id="attention-tasks">Where each task's time went</h2>
      <ul class="legend">${PART_KEYS.map(([part, label]) => html`<li><svg width="18" height="10" aria-hidden="true"><rect class="p-${part}" x="0.5" y="0.5" width="17" height="9" rx="2"/></svg>${label}</li>`)}</ul>
      <ol class="tsplit">${view.tasks.map((task) => html`<li><p><a class="mono" href="/tasks/${task.id}" title="task ${task.id}">${taskLabel(task.key)}</a><span>${task.workflow}</span><span class="sp"></span><span class="num">${task.wall}</span></p>${taskBar(task)}</li>`)}</ol></section>
    <svg width="0" height="0" class="defs" aria-hidden="true"><defs><pattern id="stripe-wait" width="6" height="6" patternUnits="userSpaceOnUse" patternTransform="rotate(45)"><rect class="stripe" width="3" height="6"/></pattern></defs></svg>`;
}

const PART_KEYS = [["working", "Working"], ["person", "Waiting on you"], ["world", "Waiting on the world"], ["queued", "Queued"]];

function taskBar(task) {
  let at = 0;
  const parts = task.parts.filter((part) => part.fraction > 0).map((part) => {
    const x = at * 100;
    at += part.fraction;
    return html`<rect class="p-${part.part}" x="${x}%" y="0.5" width="${part.fraction * 100}%" height="11"><title>${part.label}: ${part.value}</title></rect>`;
  });
  // A 2 px line of the page ground between parts keeps them apart in every encoding.
  let edge = 0;
  const gaps = task.parts.filter((part) => part.fraction > 0).slice(0, -1).map((part) => {
    edge += part.fraction;
    return html`<line class="gap" x1="${edge * 100}%" x2="${edge * 100}%" y1="0" y2="12"/>`;
  });
  return html`<svg class="tbar" width="100%" height="12" role="img" aria-label="${task.text}">${parts}${gaps}</svg>`;
}

export function costsClick(action) {
  if (action !== "costs-retry") return false;
  // A refresh that reaches the daemon reads the costs again.
  state.costs.error = null;
  refresh();
  return true;
}

export function costsChange(target) {
  if (target.dataset.action !== "costs-by") return;
  state.costs.by = target.value;
  loadCosts();
}
