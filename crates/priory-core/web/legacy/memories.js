// The memories page's legacy drawing and actions.
import { render } from "../app.js";
import { ARM_MS } from "../asks.js";
import { memoryNote, memoryTag, plural } from "../desk.js";
import { refresh, signOut } from "./data.js";
import { html, icon, post, raw, state, tile, timeTag } from "./shared.js";


// The review list (DESIGN.md, Memory candidates): each candidate's full text, then Accept and Drop.
export function memoriesPage() {
  const back = html`<div class="top"><a class="back" href="/">${icon("chevron-left", "s22")}Desk</a></div>`;
  const candidates = state.data.desk.items.find((item) => item.kind === "memories")?.candidates || [];
  const n = candidates.length;
  const intro = html`<div class="ask mhead"><h1 class="question">${plural(n, "memory candidate")}</h1>
    <p class="note">${icon("brain", "s15")}<span>An agent wrote each one. Accept gives it to every task's runs, on every runner. Drop discards it, and the task that wrote it stops getting it.</span></p></div>`;
  const list = n ? candidates.map(memoryCandidate) : html`<p class="empty">No memory candidate waits. Agents write them with the remember and forget tools.</p>`;
  return html`${back}<div class="scroll">${intro}${list}</div>`;
}

function memoryCandidate(candidate) {
  const tag = memoryTag(candidate);
  const body = candidate.forget
    ? html`<p class="text">Forgets ${candidate.name}: runs stop getting it.</p>`
    : html`<p class="desc">${candidate.description}</p><p class="text">${candidate.text}</p>`;
  const current = candidate.current_text == null ? ""
    : html`<details class="current" ${state.memoryCurrent.has(candidate.id) ? raw("open") : ""}><summary data-action="memory-current" data-id="${candidate.id}">Current text</summary><p class="text">${candidate.current_text}</p></details>`;
  return html`<section class="sect mem" aria-label="${candidate.name}"><div class="sh">${tile("memories", "sm")}<span class="mono">${candidate.name}</span><span class="tag ${tag.warn ? "t-warn" : ""}">${tag.label}</span><span class="meta">${timeTag(candidate.written)}</span></div>
    ${body}${current}
    <p class="by">Written by <span class="mono">${candidate.task}</span>${candidate.runner ? ` on ${candidate.runner}` : ""}, candidate <span class="mono">${candidate.id}</span></p>
    ${memoryActs(candidate.id)}</section>`;
}

// Accept arms on the first tap and sends on the second, as Cancel does; Drop sends at once.
function memoryActs(id) {
  const busy = state.memoryBusy !== null || state.memorySent.has(id) || Boolean(state.failed);
  const armed = state.memoryArmed?.id === id;
  const accept = armed ? html`<span><small>Tap again to accept</small>Accept</span><span class="drain" aria-hidden="true"></span>` : "Accept";
  const button = (verb, kind, label) => html`<button id="memory-${verb}-${id}" class="btn ${kind} act ${verb === "accept" && armed ? "armed" : ""}" data-action="memory-act" data-id="${id}" data-verb="${verb}" aria-disabled="${busy ? "true" : "false"}">${label}</button>`;
  const notice = state.memoryNotices.get(id);
  return html`<span class="task-acts">${button("accept", "primary", accept)}${button("drop", "secondary", "Drop")}</span>${notice ? html`<p class="note acts-note" role="status">${icon("corner-down-right", "s15")}<span>${notice}</span></p>` : ""}`;
}

export function disarmMemory() {
  if (!state.memoryArmed) return;
  clearTimeout(state.memoryArmed.timer);
  state.memoryArmed = null;
}

export async function memoryAct(target) {
  if (target.getAttribute("aria-disabled") === "true") return;
  const { id, verb } = target.dataset;
  const armed = state.memoryArmed?.id === id;
  disarmMemory();
  if (verb === "accept" && !armed) {
    state.memoryArmed = { id, at: Date.now(), timer: setTimeout(() => { state.memoryArmed = null; render(); }, ARM_MS) };
    return render();
  }
  state.memoryBusy = id;
  render();
  let reply;
  try {
    reply = await post(`/api/memories/${encodeURIComponent(id)}/${verb}`, {});
  } catch {
    reply = { status: 0, body: { error: "the connection dropped" } };
  }
  state.memoryBusy = null;
  const { status, body } = reply;
  if (status === 401) return signOut("The daemon refused this token. Paste the current one.");
  if (status === 200) state.memorySent.add(id);
  state.memoryNotices.set(id, memoryNote(verb, status, body));
  render();
  refresh();
}
