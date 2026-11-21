// A task's inbox and the unrouted events (docs/daemon/routing.md), worded as `priory task ID` prints them.

const PROMPTS = {
  confirmed: "delivered as prompt",
  queued: "still queued",
  attempted: "being typed",
  uncertain: "typed, unconfirmed",
  withdrawn: "prompt withdrawn",
};

/**
 * What one event of task `task`'s inbox did: its outcome, its prompt, the wait it fired and the
 * task routing gave it to when that is another task.
 * @param {{ outcome: string, prompt: string | null, wait: number | null, task: number | null }} event
 * @param {number} task
 */
export function handled(event, task) {
  const own = event.task === task;
  const prompt = PROMPTS[event.prompt] || (event.outcome === "kept" && own ? "for its next run" : "");
  let text = prompt ? `${event.outcome}, ${prompt}` : event.outcome;
  if (event.wait != null) text += `, fired wait ${event.wait}`;
  if (event.task != null && !own) text += ` (routed to task ${event.task})`;
  return text;
}

/** The desk's calm line for `count` unrouted events of the last 24 hours, or "" with none. */
export function unroutedNote(count) {
  if (!count) return "";
  return `${count} ${count === 1 ? "event" : "events"} reached no task in the last 24 hours`;
}

/**
 * What routes an owned group's next event, by its owner's state: revive a failed or dropped task,
 * request a new one after a done task, else subscribe the active one.
 * @param {{ state: string }} task
 */
export function routeAction(task) {
  if (["failed", "dropped"].includes(task.state)) return "revive";
  return task.state === "done" ? "request" : "subscribe";
}

/** The line for the events no task ever owned, one count per source, or "" with none. */
export function neverOwned(other) {
  if (!other.length) return "";
  const counts = other.map(({ source, events }) => `${events} ${source} ${events === 1 ? "event" : "events"}`);
  return `No task ever owned: ${counts.join(", ")}.`;
}
