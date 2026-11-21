// The web Tasks panel's pure parts: its saved views and facets. No drawing, no IO.

const ENDED = ["done", "failed", "dropped"];

export const TASK_VIEWS = ["active", "blocked", "ended"];

/** `/api/tasks` in start order as each saved view lists it: ended tasks newest first. @param {any[]} tasks */
export function taskViews(tasks) {
  return {
    active: tasks.filter((task) => !ENDED.includes(task.state)),
    blocked: tasks.filter((task) => task.state === "blocked"),
    ended: tasks.filter((task) => ENDED.includes(task.state)).sort((a, b) => b.since.localeCompare(a.since)),
  };
}

/**
 * Where another runner's task runs, from its claim: "On runner a, last seen 12 min ago", or null for
 * a task of this runner, which has an id. `ago` turns a time into "12 min ago".
 * @param {any} task @param {(iso: string) => string} ago
 */
export function elsewhere(task, ago) {
  if (task.id != null || !task.runner) return null;
  return `On runner ${task.runner}, ${task.last_seen ? `last seen ${ago(task.last_seen)}` : "never seen alive"}`;
}

/**
 * Each facet's key, label and a task's values, as the Asks list's facets. A task with no lane
 * shows under `none`. @type {[string, string, (task: any) => string[]][]}
 */
export const TASK_FACETS = [
  ["state", "State", (task) => [task.state ?? "elsewhere"]],
  ["lane", "Lane", (task) => [task.lane ?? "none"]],
  ["workflow", "Workflow", (task) => [task.workflow]],
];

/**
 * The task facets, with a Runner facet in a factory of several runners, where every task names one.
 * Like the Asks list's, it has no order and names this runner "<name> (here)". @param {any[]} tasks
 * @returns {any[]}
 */
export function taskFacets(tasks) {
  if (!tasks.some((task) => task.runner)) return TASK_FACETS;
  const named = (name) => (tasks.some((task) => task.runner === name && task.id != null) ? `${name} (here)` : name);
  return [...TASK_FACETS, ["runner", "Runner", (task) => [task.runner ?? "none"], null, named]];
}

/**
 * The items every picked facet value matches. @template T @param {T[]} items
 * @param {Map<string, string>} picks @param {[string, string, (item: T) => string[]][]} facets
 */
export const filtered = (items, picks, facets = /** @type {any} */ (TASK_FACETS)) =>
  items.filter((item) => facets.every(([key, , values]) => !picks.has(key) || values(item).includes(/** @type {string} */ (picks.get(key)))));

/**
 * A list's rows the server left out: `counted` in its view's count, only without a filter since
 * the server does not count them per facet, and `unloaded`, which loads them in the view that
 * shows history. @param {{ view: string, history: string, older: number | undefined, picked: boolean }} list
 */
export const olderPart = ({ view, history, older = 0, picked }) => ({ counted: picked ? 0 : older, unloaded: view === history ? older : 0 });

const TRIAGE_WORDS = { "nothing-to-do": "nothing to do", "persons-call": "your call" };

/** A feedback item's class as the task page words it. */
export const triageWords = (triage) => TRIAGE_WORDS[triage] || triage;
