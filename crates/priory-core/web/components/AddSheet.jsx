// The Add sheet (DESIGN.md, Add): one field and three chips; Enter queues the task through POST /api/request.
// The draft lives in localStorage until the daemon takes it, so a reload or a dropped connection keeps it.
import { memo, useEffect } from "react";
import { createPortal, flushSync } from "react-dom";
import { addBody, addOutcome, swapLine, withDefaults } from "../add.js";
import { cache, refresh } from "../data.js";
import { useStore } from "../hooks/useData.js";
import { keyOf, newest } from "../queries.js";
import { request } from "../reads/request.js";
import { api, post, session, signOut } from "../session.js";
import { store } from "../store.js";
import { taskLabel, workStep } from "../tasks.js";
import { Icon, Tile } from "./Icon.jsx";

const DRAFT_KEY = "priory.add";
// The sheet's draft and chips, the server's defaults, and where the last send got to.
export const adding = store({ phase: "empty", error: null, defaults: null, task: null, draft: { text: "" }, swapped: null, ...JSON.parse(localStorage.getItem(DRAFT_KEY) || "{}") });
export const changeAdd = (part) => adding.set({ ...adding.get(), ...part });

const sheet = () => document.getElementById("add");
export const addOpen = () => sheet().open;

// Only an unsent draft keeps its phase across a reload, so it still asks before it sends.
export function keepDraft() {
  const { draft, phase } = adding.get();
  localStorage.setItem(DRAFT_KEY, JSON.stringify({ draft, phase: ["offline", "unsure"].includes(phase) ? phase : "empty" }));
}

// Lanes have no list of their own: the chip offers each lane a listed task carries.
const knownLanes = () => [...new Set([...(newest(cache, "tasks")?.tasks ?? []), ...(newest(cache, "queue")?.waiting ?? [])].map((task) => task.lane).filter(Boolean))].sort();

// The factory may reload while the page is open; a read that fails keeps the last one.
export async function readAddDefaults() {
  const defaults = await cache.fetchQuery({ ...request(), staleTime: 0 }).catch(() => adding.get().defaults);
  changeAdd({ defaults });
}

const focusText = () => document.getElementById("add-text")?.focus();

export async function openAdd() {
  if (!newest(cache, "desk") || sheet().open) return;
  if (adding.get().phase === "added") changeAdd({ phase: "empty", task: null });
  const before = adding.get().defaults;
  // The Lane chip offers the lanes these two list, which the page may not show.
  const read = Promise.all([readAddDefaults(), ...["tasks", "queue"].map((name) => cache.ensureQueryData({ queryKey: keyOf(name) }).catch(() => {}))]);
  if (!before) await read;
  if (sheet().open) return;
  // The field the sheet focuses is drawn before it opens.
  flushSync(() => adding.set({ ...adding.get() }));
  sheet().showModal();
  focusText();
}

async function submitAdd(event) {
  event.preventDefault();
  const { draft, phase, defaults } = adding.get();
  const body = addBody(withDefaults(draft, defaults));
  if (!body || phase === "sending") return;
  // Offline, Enter only keeps the draft; it goes once the person confirms it after reconnecting.
  if (session.get().failed) {
    changeAdd({ phase: "offline" });
    return keepDraft();
  }
  // Stored as unsure first: a reload while it sends must not offer it as unsent.
  localStorage.setItem(DRAFT_KEY, JSON.stringify({ draft, phase: "unsure" }));
  changeAdd({ phase: "sending" });
  const reply = await post("/api/request", body).catch(() => ({ status: 0, body: {} }));
  const outcome = addOutcome(reply.status, reply.body);
  if (outcome.state === "signed-out") {
    changeAdd({ phase: "empty" });
    keepDraft();
    sheet().close();
    return signOut("The daemon refused this token. Paste the current one.");
  }
  if (outcome.state === "added") {
    changeAdd({ draft: { text: "" }, swapped: null, task: { key: outcome.key, id: outcome.id, body: null } });
    localStorage.removeItem(DRAFT_KEY);
    if (outcome.id !== null) api(`/api/tasks/${outcome.id}`).then((task) => { if (adding.get().task?.id === outcome.id) changeAdd({ task: { ...adding.get().task, body: task } }); }, () => {});
    refresh();
  }
  flushSync(() => changeAdd({ phase: outcome.state, error: outcome.error || null }));
  if (outcome.state !== "added") keepDraft();
  else sheet().querySelector(".added .btn.primary")?.focus();
}

function discard() {
  flushSync(() => changeAdd({ phase: "empty", error: null, swapped: null, draft: { text: "" } }));
  localStorage.removeItem(DRAFT_KEY);
  focusText();
}

function another() {
  flushSync(() => changeAdd({ phase: "empty", task: null }));
  focusText();
}

function typed(name, value) {
  changeAdd({ draft: { ...adding.get().draft, [name]: value }, ...(name === "workflow" ? { swapped: null } : {}) });
  keepDraft();
}

const Chip = ({ name, label, value, values, none }) => {
  const options = [...new Set([...values, value].filter(Boolean))];
  return (
    <label className="chip"><span>{label}</span><select name={name} data-add-chip={name} value={value || ""} onChange={(event) => typed(name, event.target.value)}>
      <option value="">{none}</option>
      {options.map((option) => <option key={option}>{option}</option>)}</select><Icon name="chevron-down" size="s13" /></label>
  );
};

const Added = ({ task: { key, id, body } }) => {
  const task = body || { key, state: "queued" };
  return (
    <div className="added" role="status">
      <p className="line">Added. The task waits in the queue for its turn.</p>
      <div className="row"><Tile state={task.state} />
        <span className="l1"><span className="mono">{taskLabel(task.key)}</span><span>{workStep(task)}</span><span className="age">{task.state}</span></span>
        <span className="l2">{task.brief?.request || task.brief?.title || (id === null ? "No task took this key yet. It shows on the Desk once one does." : "")}</span>
        <span className="l3">{[task.band, task.lane].filter(Boolean).join(", ")}</span></div>
      <div className="acts"><button type="button" className="btn secondary" data-action="add-another" onClick={another}>Add another</button><button type="button" className="btn primary" data-action="add-done" onClick={() => sheet().close()}>Close</button></div></div>
  );
};

const LINES = {
  offline: "Offline. The draft stays on this device, and nothing is sent until you confirm it after you reconnect.",
  unsure: "The connection dropped while sending, so Priory may have queued it. The draft stays until you send it again or discard it.",
};

function Status({ phase, error, back }) {
  if (phase === "refused") return <p className="field-error" id="add-status"><Icon name="circle-x" size="s15" />{`Could not add: ${error}.`}</p>;
  const line = back
    ? (phase === "unsure" ? "Reconnected. Priory may have queued this before the drop: check the Desk's All view, then send it if it is missing." : "Reconnected. This draft has not been sent. Send it now?")
    : LINES[phase];
  return line ? <p className="warnline" id="add-status"><Icon name="unplug" size="s15" /><span>{line}</span></p> : null;
}

export const AddSheet = memo(function AddSheet() {
  const { phase, error, defaults, swapped, task, draft: kept } = useStore(adding);
  const { failed } = useStore(session);
  // A tap on the scrim, outside the sheet's box, closes it; closing hands focus back to an Add button.
  useEffect(() => {
    const dialog = sheet();
    const outside = (event) => { if (event.target === dialog) dialog.close(); };
    const closed = () => document.querySelector("#app [data-action='add-open']")?.focus();
    dialog.addEventListener("click", outside);
    dialog.addEventListener("close", closed);
    return () => {
      dialog.removeEventListener("click", outside);
      dialog.removeEventListener("close", closed);
      if (dialog.open) dialog.close();
    };
  }, []);
  const draft = withDefaults(kept, defaults);
  const sending = phase === "sending";
  const back = !failed && ["offline", "unsure"].includes(phase);
  const status = phase === "refused" || back || LINES[phase];
  const label = sending ? "Adding…" : back ? "Send" : failed ? "Keep draft" : "Add";
  const body = phase === "added" && task ? <Added task={task} /> : (
    <form data-action="add" aria-busy={sending ? "true" : undefined} onSubmit={submitAdd}>
      <label className="sr" htmlFor="add-text">What should the factory do?</label>
      <input id="add-text" name="text" autoComplete="off" enterKeyHint="send" placeholder="What should the factory do?" value={draft.text} readOnly={sending} aria-describedby={status || swapped ? "add-status add-swapped" : undefined} onChange={(event) => typed("text", event.target.value)} />
      <Status phase={phase} error={error} back={back} />{swapped ? <p className="warnline" id="add-swapped"><Icon name="circle-alert" size="s15" /><span>{swapLine(swapped, defaults?.workflow)}</span></p> : null}
      <div className="chips"><Chip name="workflow" label="Workflow" value={draft.workflow} values={defaults?.workflows || []} none={defaults?.workflow ? `default (${defaults.workflow})` : "pick one"} />
        <Chip name="band" label="Band" value={draft.band} values={defaults?.bands || newest(cache, "asks")?.bands || []} none="default" />
        <Chip name="lane" label="Lane" value={draft.lane} values={knownLanes()} none="routed" /></div>
      <div className="acts">{phase === "empty" || sending ? null : <button type="button" className="ghost" data-action="add-discard" onClick={discard}>Discard draft</button>}<span className="sp" />
        <button type="submit" className="btn primary" aria-disabled={!draft.text.trim() || sending ? "true" : "false"}>{label}</button></div></form>
  );
  return createPortal(<><div className="shead"><h2 id="add-title">Add a task</h2><button type="button" className="iconbtn" data-action="add-close" aria-label="Close" onClick={() => sheet().close()}><Icon name="x" size="s20" /></button></div>{body}</>, sheet());
});
