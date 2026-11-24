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
 * task routing gave it to when that is another task, by `name`.
 * @param {{ outcome: string, prompt: string | null, wait: number | null, task: number | null }} event
 * @param {number} task @param {(id: number) => string} [name]
 */
export function handled(event, task, name = (id) => `task ${id}`) {
  const own = event.task === task;
  const prompt = PROMPTS[event.prompt] || (event.outcome === "kept" && own ? "for its next run" : "");
  let text = prompt ? `${event.outcome}, ${prompt}` : event.outcome;
  if (event.wait != null) text += `, fired wait ${event.wait}`;
  if (event.task != null && !own) text += ` (routed to ${name(event.task)})`;
  return text;
}

/** The desk's calm line for `count` unrouted events of the last 24 hours, or "" with none. */
export function unroutedNote(count) {
  if (!count) return "";
  return `${count} ${count === 1 ? "event" : "events"} reached no task in the last 24 hours`;
}

/** The desk's calm line for `count` events a task kept that no agent read, or "" with none. */
export function unheardNote(count) {
  if (!count) return "";
  return `${count} ${count === 1 ? "event" : "events"} kept by a sleeping task reached no agent`;
}

/** The desk's calm line for `count` requests a pre script dropped in the last 24 hours, or "" with none. */
export function droppedNote(count) {
  if (!count) return "";
  return `${count} ${count === 1 ? "request" : "requests"} a pre script dropped in the last 24 hours`;
}

/**
 * Why a pre script dropped a request, its reason or how it failed, and what judges its resource again.
 * @param {{ reason: string | null, failed: string | null }} drop
 */
export function droppedWhy(drop) {
  const why = drop.reason ? `Dropped: ${drop.reason}.` : drop.failed ? `Dropped: the script failed: ${drop.failed}.` : "Dropped with no reason.";
  return `${why} A new event of it that a trigger wants runs the script again.`;
}

/**
 * What gets an unheard group's events to an agent, by its task's state: say to an asleep task;
 * say to a blocked one, which reads them when it resumes; route an ended one as an owned group
 * would; else nothing, since its next run reads them.
 * @param {{ state: string }} task
 */
export function unheardActs(task) {
  if (task.state === "asleep") return ["say"];
  if (task.state === "blocked") return ["say", "resume"];
  return ["done", "failed", "dropped"].includes(task.state) ? [routeAction(task)] : [];
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

/**
 * The workflow "Request a task" prefills: the group's task's when the factory lets a person request
 * it, else none, so the factory's `[request]` default applies. With no list read, the task's.
 * @param {string} workflow @param {string[] | null | undefined} requestable
 */
export function requestedWorkflow(workflow, requestable) {
  return !requestable || requestable.includes(workflow) ? workflow : undefined;
}

/** The line for the events no task ever owned, one count per source, or "" with none. */
export function neverOwned(other) {
  if (!other.length) return "";
  const counts = other.map(({ source, events }) => `${events} ${source} ${events === 1 ? "event" : "events"}`);
  return `No task ever owned: ${counts.join(", ")}.`;
}

const SOURCES = { github: "GitHub", linear: "Linear", slack: "Slack", factory: "" };

/** An event's type in plain words: `linear.issue.updated` reads "Linear issue updated". */
export function eventWords(type) {
  const [source, ...rest] = type.split(".");
  return [SOURCES[source] ?? source, ...rest].filter(Boolean).join(" ").replaceAll("_", " ");
}

/** A Since you looked line, `<type> <subject>: <outcome>`, with its type in plain words; a report's line as it is. @param {string} what */
export function sinceWords(what) {
  const [type] = what.split(" ", 1);
  return /^[a-z_]+(\.[a-z_]+)+$/.test(type) ? eventWords(type) + what.slice(type.length) : what;
}

/** The detail pane's line while nothing is selected, by the panels on screen: events open no detail. */
export const emptyDetail = (panels) =>
  panels.includes("inbox") && !panels.some((panel) => ["desk", "asks"].includes(panel))
    ? "Open an event's task to read what the event did."
    : "Select an ask to read it here.";
