// A task's actions (Resume, Revive, Cancel, Release, Say), as every page sends them.
import { ARM_MS } from "./asks.js";
import { refresh, revision } from "./data.js";
import { onSignOut, post, refuse } from "./session.js";
import { store } from "./store.js";

export const ACTS = { resume: "Resume", revive: "Revive", cancel: "Cancel", release: "Release" };
const DONE_AS = { resume: "Resumed", revive: "Revived", cancel: "Cancelled", release: "Released", say: "Said" };
const ARMS = ["revive", "cancel"];
export const ENDED = ["done", "failed", "dropped"];
export const actsOf = (taskState) => (taskState === "blocked" ? ["resume", "cancel"] : ["failed", "dropped"].includes(taskState) ? ["revive"] : taskState === "done" ? [] : ["cancel"]);

// `armed`: a cancel or revive armed by its first tap, until the second tap or ARM_MS: { task, act, at,
// timer }. `busy`: the task whose action is in flight. `notices`: each task's last action note.
// `drafts`: each task's unsent Say text, which outlives the page.
const EMPTY = () => ({ armed: null, busy: null, notices: new Map(), drafts: new Map() });
export const acts = store(EMPTY());
const change = (part) => acts.set({ ...acts.get(), ...part });
onSignOut(() => disarmTask());

export const noticeOf = (status, act, answered, remote) => (status === 200 ? `${DONE_AS[act]}: ${answered.ok}.`
  : remote && status === 409 ? `Not sent: ${answered.error}. The list reads it again: check the task, then send again.`
  : remote && status === 503 ? `Could not send: ${answered.error}.`
  : status === 409 ? "The state changed since this page loaded. Read the task again, then tap again."
    : status === 503 ? "Could not send: no priory daemon runs beside this server. Start priory daemon, then send again."
      : status === 504 || status === 0 ? "The daemon may have applied this but did not confirm it. Check the task's state before you send again."
        : `Could not ${act}: ${answered.error || `the server answered ${status}`}.`);

export function draft(task, text) {
  const drafts = new Map(acts.get().drafts);
  drafts.set(task, text);
  change({ drafts });
}

export function disarmTask() {
  const { armed } = acts.get();
  if (!armed) return;
  clearTimeout(armed.timer);
  change({ armed: null });
}

/** A tap on an action. Revive and Cancel arm on the first and send on the second. */
export function pressTask(task, act, { generation, body }) {
  const armed = acts.get().armed;
  disarmTask();
  if (ARMS.includes(act) && !(armed?.task === task && armed.act === act)) {
    return change({ armed: { task, act, generation, at: Date.now(), timer: setTimeout(() => change({ armed: null }), ARM_MS) } });
  }
  // A cancel carries the revision the page showed, as `priory cancel --expected` does.
  return writeTask(task, act, body ?? (act === "cancel" ? { revision: revision() } : {}));
}

export async function writeTask(task, act, body) {
  change({ busy: task });
  let reply;
  try {
    reply = await post(`/api/tasks/${task}/${act}`, body);
  } catch {
    reply = { status: 0, body: { error: "the connection dropped" } };
  }
  const { status, body: answered } = reply;
  const notices = new Map(acts.get().notices);
  const drafts = new Map(acts.get().drafts);
  if (status === 401) {
    change({ busy: null });
    return refuse();
  }
  // Text typed while the say was in flight stays as the next draft.
  if ([200, 202].includes(status) && act === "say" && (drafts.get(task) || "").trim() === body.text) drafts.delete(task);
  // A sent action shows as the task's newest submission once the list reads it again.
  if (status === 202) notices.delete(task);
  else notices.set(task, noticeOf(status, act, answered, typeof task === "string"));
  // A refused action changes no listing, so the refresh may not paint: the note shows now.
  change({ busy: null, notices, drafts });
  refresh();
}
