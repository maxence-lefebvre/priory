// The terminal page's legacy drawing and actions.
import { plural } from "../desk.js";
import { taskLabel } from "../tasks.js";
import { EARLIER_CAP, KEYS, keyboardSession, paintRows, sayLine, screenRows, screenWatch } from "../terminal.js";
import { api, signOut } from "./data.js";
import { STATES, clock, html, icon, post, raw, state, tile } from "./shared.js";


// The run's terminal: its screen polled while shown (terminal.js), never kept on this device.
export const watch = screenWatch({
  read: (run, have, history) => api(`/api/runs/${run}/screen?${new URLSearchParams({ history, ...(have ? { have } : {}) })}`).catch((error) => {
    if (error.status === 401) signOut("The daemon refused this token. Paste the current one.");
    throw error;
  }),
  changed: () => paintTerminal(document.getElementById("app")),
});
// `left` is the pane's sideways scroll for run `run`, kept while a refresh redraws the task page.
// `typing` opens the Say line, `line` holds its text, `said` its last note and whether it was said.
// `menu` opens the header's More, and `keys` holds the text the keys bar pastes.
export const terminal = { fit: false, copied: null, run: null, left: 0, onScreen: false, typing: false, line: "", said: null, saying: false, menu: false, keys: "" };
// This page's hold on the pane's keyboard (terminal.js). Its writes keep going while the page unloads.
export const keyboard = keyboardSession({
  post: (path, body) => fetch(path, {
    method: "POST",
    keepalive: true,
    headers: { Authorization: `Bearer ${state.token}`, "Content-Type": "application/json" },
    body: JSON.stringify(body),
  }).then(async (response) => ({ status: response.status, body: await response.json().catch(() => ({})) })),
  signOut: () => signOut("The daemon refused this token. Paste the current one."),
  changed: () => paintTerminal(document.getElementById("app"), true),
  format: (iso) => clock(iso),
});
export const screenText = (screen) => screenRows(screen.lines).map((parts) => parts.map((part) => part.text).join("")).join("\n").trimEnd();

// The live run, else the run whose last screen this page still holds.
export function terminalRun(task) {
  const runs = task.runs || [];
  const kept = watch.state();
  return runs.findLast((run) => !run.ended) || runs.find((run) => run.id === kept.run && kept.screen) || null;
}

function terminalBody({ term, task, tmux, end }) {
  const run = Number(term);
  const kept = watch.state();
  const { screen, at, stale, gone } = kept.run === run ? kept : {};
  const notes = [];
  if (screen?.redacted) notes.push(["warn", "shield", `${plural(screen.redacted, "secret")} hidden on this screen`]);
  if (stale && !end && !gone) notes.push(["warn", "unplug", screen ? `Reconnecting. Screen from ${clock(new Date(at).toISOString())}, ${Math.round((Date.now() - at) / 1000)} s ago.` : "Reconnecting. The screen could not be read yet."]);
  if (end) notes.push(["", (STATES[end] || ["muted", "circle-help"])[1], screen ? `Run ${run} ended as ${end}. This is the last screen this page saw.` : `Run ${run} ended as ${end}.`]);
  else if (gone) notes.push(["warn", "unplug", `This server reads no screen of run ${run}. Run priory screen ${task} on the runner to see why.`]);
  else if (!screen && !stale) notes.push(["", "clock", "Loading the screen…"]);
  const keys = keyboard.state();
  const held = end ? null : keys.held;
  const offer = !end && !held && Boolean(screen?.keyboard);
  if (keys.ended && !held) notes.push(["", "clock", keys.ended, offer]);
  const earlier = screen?.earlier || 0;
  const top = end || !screen ? "" : kept.more ? html`<button class="term-more" data-action="term-earlier">${icon("chevron-up", "s15")}Load earlier${earlier ? html` <span class="dim">(${earlier} shown)</span>` : ""}</button>`
    : earlier >= EARLIER_CAP ? html`<p class="term-edge">The last ${EARLIER_CAP.toLocaleString("en")} lines are as far back as this pane reads.</p>` : "";
  const typing = terminal.typing && !end;
  const copied = (what, text) => (terminal.copied === what ? "Copied" : text);
  const tag = held ? html`<span class="ro keys">keys</span>` : html`<span class="ro${typing ? " on" : ""}">${typing ? "say" : "read-only"}</span>`;
  const menu = terminal.menu ? html`<div class="term-menu" id="term-more">
      ${offer ? html`<button data-action="term-take-ask">${icon("keyboard", "s15")}<span>Take the keyboard<small>Send keys straight into the pane</small></span></button>` : ""}
      ${held ? html`<button data-action="term-give-ask">${icon("keyboard", "s15")}<span>Give the keyboard back</span></button>` : ""}
      <button data-action="term-attach" data-task="${task}">${icon("eye", "s15")}<span>${copied("attach", "Copy attach command")}</span></button></div>` : "";
  return html`${notes.map(([kind, name, text, again]) => html`<p class="note ${kind}" role="status">${icon(name, "s15")}<span>${text}${again ? html` <button class="linkish" data-action="term-take-ask">Take it again</button>` : ""}</span></p>`)}
    <div class="term${terminal.fit ? " fit" : ""}${end ? " ended" : ""}${held ? " held" : ""}" role="region" aria-label="Terminal of run ${run}">
      <div class="term-head">${icon("terminal", "s15")}<span class="mono">${tmux}</span><span class="rid">run ${run}</span>${end ? tile(end, "sm") : ""}${tag}<span class="sp"></span>
        ${end ? "" : html`<button data-action="term-type" aria-label="Type" aria-pressed="${String(typing)}">${icon("message-text", "s15")}<span>Type</span></button>`}
        <button data-action="term-copy" aria-label="Copy screen" ${screen ? "" : raw('aria-disabled="true"')}>${icon("copy", "s15")}<span>${copied("screen", "Copy screen")}</span></button>
        <button data-action="term-fit" aria-label="Fit" aria-pressed="${String(terminal.fit)}">${icon("fit", "s15")}<span>Fit</span></button>
        <button data-action="term-menu" aria-label="More" aria-controls="term-more" aria-expanded="${String(terminal.menu)}">${icon("more", "s15")}</button></div>
      ${menu}${top}<pre id="term-screen" tabindex="0" aria-label="Screen${screen ? `, ${screen.width} by ${screen.height}` : ""}"></pre>
      ${typing ? html`<div class="term-type"><form data-action="term-say" data-task="${task}"><span class="pr" aria-hidden="true">›</span><label class="sr" for="term-line">Say to the agent</label>
        <input id="term-line" autocomplete="off" placeholder="Say to the agent"><button class="send" ${terminal.line.trim() && !terminal.saying ? "" : raw('aria-disabled="true"')}>Say</button></form>
        ${terminal.said ? html`<p class="tn ${terminal.said.said ? "ok" : "no"}" role="status">${icon(terminal.said.said ? "check" : "circle-x", "s15")}<span>${terminal.said.note}</span></p>` : ""}</div>` : ""}
      ${held ? keysBar(held, keys) : ""}</div>`;
}

// The markup of each row of the screen. The cursor shows only on a live run's screen with no earlier rows above it.
function terminalRows(screen, end) {
  if (!screen) return [];
  // Each part's classes come from screenRows' fixed set; its text stays text.
  const inner = (part) => (part.hidden ? html`<span class="hid">${part.text}</span>` : part.cls ? html`<span class="${part.cls}">${part.text}</span>` : part.text);
  const part = (part) => (part.cursor ? html`<span class="cur">${inner(part)}</span>` : inner(part));
  const earlier = screen.earlier || 0;
  return screenRows(screen.lines, earlier || end ? null : screen.cursor).map((parts, at) => html`${at < earlier ? html`<span class="earlier">${parts.map(part)}</span>` : parts.map(part)}\n${at === earlier - 1 ? raw('<span class="fold" aria-hidden="true"></span>') : ""}`.text);
}

// The bar of a held keyboard: whose and until when, a text pasted without Enter, and one button a key.
function keysBar(held, { note, returning, pasting }) {
  const row = KEYS.map((key) => (key ? html`<button data-action="term-key" data-key="${key[0]}" aria-label="${key[0]}"${key[2] ? raw(' class="hot"') : ""}>${key[1]}</button>` : raw('<span class="gap" aria-hidden="true"></span>')));
  return html`<div class="keys-bar"><p class="who">${icon("keyboard", "s15")}<span><b>Your keys</b> until ${clock(held.ends)}</span><span class="sp"></span><button class="give" data-action="term-give-ask" ${returning ? raw('aria-disabled="true"') : ""}>${returning ? "Giving back…" : "Give back"}</button></p>
    <form data-action="term-paste"><span class="pr" aria-hidden="true">›</span><label class="sr" for="term-keys">Text to paste, without Enter</label>
      <input id="term-keys" autocomplete="off" autocapitalize="off" spellcheck="false" placeholder="Text, sent without Enter"><button class="send" ${pasting || returning ? raw('aria-disabled="true"') : ""}>Send</button></form>
    <div class="keys-row" role="group" aria-label="Keys">${row}</div>
    ${note ? html`<p class="tn" role="status">${icon("check", "s15")}<span>${note}</span></p>` : ""}</div>`;
}

export const keyboardSheet = () => document.getElementById("keyboard");

// The confirm sheet before a person takes the keyboard, or gives it back (DESIGN.md, Terminal pane).
export function openKeyboardSheet(kind) {
  const sheet = keyboardSheet();
  const run = keyboard.state().run;
  // The sheet acts on the run it names, never on one the pane switched to under it.
  sheet.dataset.run = run;
  const tail = (watch.state().screen?.lines || []).map((line) => line.replace(/\x1b\[[0-9;:]*m/g, "")).filter((line) => line.trim()).slice(-2).join("\n");
  sheet.innerHTML = (kind === "take"
    ? html`<div class="shead"><h2 id="keyboard-title">Take the keyboard of run ${run}?</h2><button class="iconbtn" data-action="keyboard-close" aria-label="Close">${icon("x", "s20")}</button></div>
      <p class="warnline">${icon("shield", "s15")}<span>Your keys reach the agent and its shell directly. Priory's guard does not check them.</span></p>
      <ul class="facts"><li>${icon("clock", "s15")}<span>For 15 minutes, or until you give it back.</span></li>
        <li>${icon("pause", "s15")}<span>The agent's queued prompts and your Says wait until then.</span></li>
        <li>${icon("pencil", "s15")}<span>Priory records each key and the length of each text, never the text.</span></li></ul>
      <div class="acts"><button class="btn secondary" data-action="keyboard-close">Keep watching</button><button class="btn danger" data-action="term-take">${icon("keyboard", "s15")}Take the keyboard</button></div>`
    : html`<div class="shead"><h2 id="keyboard-title">Give the keyboard back?</h2><button class="iconbtn" data-action="keyboard-close" aria-label="Close">${icon("x", "s20")}</button></div>
      <p class="line">Anything you typed and did not submit stays in the agent's input box, and its next prompt joins it. Clear it with ⌃C or submit it with ⏎ first.</p>
      ${tail ? html`<pre class="peek">${tail}</pre>` : ""}
      <div class="acts"><button class="btn secondary" data-action="keyboard-close">Keep typing</button><button class="btn primary" data-action="term-give">Give back</button></div>`).text;
  if (!sheet.open) sheet.showModal();
  sheet.querySelector(".acts .btn:last-child")?.focus();
}

export async function keyboardAct(action) {
  const sheet = keyboardSheet();
  const run = keyboard.state().run;
  sheet.close();
  if (String(run) !== sheet.dataset.run) return;
  if (action === "term-take") {
    terminal.typing = false;
    await keyboard.take(run);
    return document.getElementById("term-keys")?.focus();
  }
  await keyboard.giveBack();
  watch.revision();
}

// Sends one input; a text leaves the field once it was sent, unless the person typed on.
export async function sendInput(input) {
  const sent = await keyboard.send(input);
  if (sent && input.text !== undefined && terminal.keys === input.text) terminal.keys = "";
  if (sent) watch.revision();
  paintTerminal(document.getElementById("app"), true);
}

// Sends the Say line; a refusal keeps its text, so a tap sends it again.
async function sayFromPane(task) {
  const [text, run] = [terminal.line, terminal.run];
  if (terminal.saying || !text.trim()) return;
  terminal.saying = true;
  paintTerminal(document.getElementById("app"), true);
  const said = await sayLine(post, task, text);
  if (said.signOut) return signOut("The daemon refused this token. Paste the current one.");
  // The pane moved to another run meanwhile: its line and note are not this one's.
  if (terminal.run !== run) return;
  terminal.saying = false;
  // Text typed while the line was in flight stays.
  if (said.said && terminal.line === text) terminal.line = "";
  terminal.said = said;
  paintTerminal(document.getElementById("app"), true);
}

// Repaints the pane when what it shows changed, or always after a render, keeping its sideways
// scroll and focus. Fit sizes the text to the pane, between 9 px and the pane's own size.
export function paintTerminal(app, rendered = false) {
  const box = app?.querySelector("[data-term]");
  // A page without the pane gives back a keyboard it held.
  if (!box) return app && keyboard.leave();
  const kept = watch.state();
  // The watch may still hold the run shown before, until it polls the one in this box.
  const screen = kept.run === Number(box.dataset.term) ? kept.screen : null;
  // A grant this page holds on a run it no longer shows goes back, so that run's outbox resumes.
  keyboard.at(box.dataset.end ? null : Number(box.dataset.term), `${box.dataset.task}/${box.dataset.tmux}`);
  if (terminal.run !== box.dataset.term) Object.assign(terminal, { run: box.dataset.term, left: 0, typing: false, line: "", said: null, saying: false, menu: false, keys: "" });
  const frame = terminalBody(box.dataset).text;
  const key = JSON.stringify([frame, innerWidth]);
  // A new screen in the same frame rewrites only its changed rows, so a selection elsewhere stays.
  if (box.dataset.painted === key && !rendered) return paintRows(box.querySelector("pre"), terminalRows(screen, box.dataset.end));
  const active = document.activeElement;
  const focused = !box.contains(active) ? null : active.dataset.key ? `[data-key="${CSS.escape(active.dataset.key)}"]` : active.id ? `#${active.id}` : active.dataset.action ? `[data-action="${active.dataset.action}"]` : "pre";
  const caret = ["term-line", "term-keys"].includes(active?.id) ? [active.id, active.selectionStart, active.selectionEnd] : null;
  box.innerHTML = frame;
  box.dataset.painted = key;
  const pre = box.querySelector("pre");
  paintRows(pre, terminalRows(screen, box.dataset.end));
  const own = parseFloat(getComputedStyle(pre).fontSize);
  const fit = terminal.fit && screen;
  pre.style.fontSize = fit ? `${Math.min(own, Math.max(9, (pre.clientWidth - 24) / (screen.width * 0.6)))}px` : "";
  pre.style.lineHeight = fit ? "1.33" : "";
  pre.scrollLeft = terminal.left;
  pre.addEventListener("scroll", () => { terminal.left = pre.scrollLeft; });
  if (focused) (focused === "pre" ? pre : box.querySelector(focused))?.focus({ preventScroll: true });
  // Typed text stays out of the frame, so typing alone repaints nothing.
  for (const [id, text] of [["term-line", terminal.line], ["term-keys", terminal.keys]]) {
    const input = box.querySelector(`#${id}`);
    if (input) input.value = text;
  }
  if (caret) box.querySelector(`#${caret[0]}`)?.setSelectionRange(caret[1], caret[2]);
}

// Polls while a live run's pane is on screen and the page is shown.
export function syncTerminal(app) {
  const box = app.querySelector("[data-term]");
  if (!box || box.dataset.end || document.hidden || !state.token || !terminal.onScreen) return watch.hide();
  watch.show(Number(box.dataset.term));
}

export function copyTerminal(text, what) {
  navigator.clipboard?.writeText(text).then(() => {
    terminal.copied = what;
    paintTerminal(document.getElementById("app"));
    setTimeout(() => {
      terminal.copied = null;
      paintTerminal(document.getElementById("app"));
    }, 2000);
  }, () => {});
}

// The Terminal panel: each running task, whose page shows its terminal.
export function terminalsPanel() {
  const running = (state.data?.tasks || []).filter((task) => task.state === "running");
  if (!running.length) return html`<p class="empty">No agent runs now. A running task's terminal shows here.</p>`;
  return html`<ol class="hist" aria-label="Running tasks">${running.map((task) => html`<li>${tile("running", "sm")}<a href="/tasks/${task.id}#terminal" title="task ${task.id}"><span class="mono">${taskLabel(task.key)}</span> ${task.title || ""}</a></li>`)}</ol>`;
}

// Each event function of a page returns true once it acted.
export function terminalClick(action, target) {
  if (action === "term-fit") {
    terminal.fit = !terminal.fit;
    paintTerminal(document.getElementById("app"));
  } else if (action === "term-copy") {
    if (watch.state().screen) copyTerminal(screenText(watch.state().screen), "screen");
  } else if (action === "term-earlier") watch.earlier();
  else if (action === "term-type") {
    Object.assign(terminal, { typing: !terminal.typing, said: null });
    paintTerminal(document.getElementById("app"), true);
    if (terminal.typing) document.getElementById("term-line")?.focus();
  } else if (action === "term-attach") copyTerminal(`priory attach ${target.dataset.task} --read-only`, "attach");
  else if (action === "term-menu") {
    terminal.menu = !terminal.menu;
    paintTerminal(document.getElementById("app"), true);
  } else if (action === "term-take-ask" || action === "term-give-ask") {
    terminal.menu = false;
    paintTerminal(document.getElementById("app"), true);
    openKeyboardSheet(action === "term-take-ask" ? "take" : "give");
  } else if (action === "term-take" || action === "term-give") keyboardAct(action);
  else if (action === "keyboard-close") keyboardSheet().close();
  else if (action === "term-key") sendInput({ key: target.dataset.key });
  else return false;
  return true;
}

export function terminalInput(target) {
  if (target.id === "term-keys") terminal.keys = target.value;
  if (target.id === "term-line") {
    terminal.line = target.value;
    // Only the button changes; a repaint per letter would cost the caret.
    target.form.querySelector(".send").setAttribute("aria-disabled", String(!terminal.line.trim() || terminal.saying));
  }
}

export function terminalSubmit(action, form) {
  if (action === "term-paste") {
    if (terminal.keys) sendInput({ text: terminal.keys });
  } else if (action === "term-say") sayFromPane(Number(form.dataset.task));
  else return false;
  return true;
}

export function terminalKey(event) {
  if (event.key !== "Escape") return false;
  if (terminal.menu && event.target.closest?.(".term")) {
    terminal.menu = false;
    paintTerminal(document.getElementById("app"), true);
    document.querySelector('[data-action="term-menu"]')?.focus();
    return true;
  }
  if (event.target.id === "term-line") {
    Object.assign(terminal, { typing: false, said: null });
    paintTerminal(document.getElementById("app"), true);
    document.querySelector('[data-action="term-type"]')?.focus();
    return true;
  }
  return false;
}

// A page that goes away gives its keyboard back, so the run's outbox does not wait out the 15 minutes.
addEventListener("pagehide", () => keyboard.leave());
keyboardSheet().addEventListener("click", (event) => { if (event.target === keyboardSheet()) keyboardSheet().close(); });
keyboardSheet().addEventListener("close", () => document.querySelector("#app [data-action='term-menu']")?.focus());
document.addEventListener("visibilitychange", () => syncTerminal(document.getElementById("app")));
addEventListener("resize", () => { if (terminal.fit) paintTerminal(document.getElementById("app")); });
