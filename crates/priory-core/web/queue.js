// The web Queue panel's pure parts: its bands, why each task waits and the Move menu. No drawing, no IO.

export const QUEUE_VIEWS = ["waiting", "held", "ended"];

/** `/api/queue`'s queued and ready tasks by band, in the factory's band order, each in admission order. @param {any} queue */
export function queueBands(queue) {
  const waiting = queue.waiting.filter((/** @type {any} */ task) => ["queued", "ready"].includes(task.state));
  const order = [...queue.bands, ...new Set(waiting.map((/** @type {any} */ task) => task.band).filter((/** @type {any} */ band) => !queue.bands.includes(band)))];
  return order.map((band) => ({ band, tasks: waiting.filter((/** @type {any} */ task) => task.band === band) })).filter((group) => group.tasks.length);
}

/**
 * A capped lane of `/api/queue`'s `lanes`: its live runs against the cap in force now, then the
 * cap and its window, as `priory queue` prints it. @param {any} lane
 */
export function laneCap(lane) {
  const now = lane.cap === null ? `${lane.running} running, no cap now` : `${lane.running} of ${lane.cap} running`;
  const outside = lane.max_running_outside === null ? "no cap outside" : `${lane.max_running_outside} outside`;
  const cap = lane.window === null ? `max_running ${lane.max_running}` : `max_running ${lane.max_running} ${lane.window}, ${outside}`;
  return { lane: lane.lane, now, cap, full: lane.cap !== null && lane.running >= lane.cap };
}

/**
 * Why a waiting task has not started: the daemon's reason, room for a ready task, a band ahead
 * with tasks, or the task ahead in its band. @param {any} task @param {{ band: string, tasks: any[] }[]} bands
 */
export function waitReason(task, bands) {
  if (task.reason) return task.reason;
  if (task.state === "ready") return "Ready: starts when a runner has room.";
  const at = bands.findIndex((group) => group.band === task.band);
  const tasks = bands[at]?.tasks ?? [];
  const ahead = tasks[tasks.indexOf(task) - 1];
  if (ahead) return `Waits behind ${ahead.key} in band ${task.band}.`;
  if (at > 0) return `Waits behind band ${bands[at - 1].band}.`;
  return `Waits for its turn: first in band ${task.band}.`;
}

/**
 * The Move menu of a waiting task: To the top, Up one and Down one within its band, then To band
 * for each other band of the factory, each with its `POST /api/tasks/{id}/move` body.
 * @param {any} task @param {{ band: string, tasks: any[] }[]} bands @param {string[]} factoryBands @param {number} revision
 */
export function moveOptions(task, bands, factoryBands, revision) {
  const tasks = bands.find((group) => group.band === task.band)?.tasks ?? [];
  const at = tasks.indexOf(task);
  return [
    ...(at > 0 ? [{ label: "To the top", body: { revision, top: true } }, { label: "Up one", body: { revision, before: tasks[at - 1].id } }] : []),
    ...(at >= 0 && at < tasks.length - 1 ? [{ label: "Down one", body: { revision, after: tasks[at + 1].id } }] : []),
    ...factoryBands.filter((band) => band !== task.band).map((band) => ({ label: `To band ${band}`, body: { revision, band } })),
  ];
}
