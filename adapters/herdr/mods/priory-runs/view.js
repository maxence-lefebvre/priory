// The web Runs panel's pure parts: a run's state, its saved views, facets, duration and tokens.
// No drawing, no IO.

/** `running` while the run is live, else how it ended (DESIGN.md, State tile). @param {any} run */
export const runState = (run) => (run.ended ? run.end || "ended" : "running");

export const RUN_VIEWS = ["recent", "live"];

/** `/api/runs`, newest first, as each saved view lists it. @param {any[]} runs */
export const runViews = (runs) => ({ recent: runs, live: runs.filter((run) => !run.ended) });

/**
 * Each facet's key, label and a run's values. A run's outcome is how it ended and its result's
 * outcome, so "moved" and "passed" both find it. A run with no agent shows under `none`.
 * @type {[string, string, (run: any) => string[]][]}
 */
export const RUN_FACETS = [
  ["outcome", "Outcome", (run) => [...new Set([runState(run), run.outcome].filter(Boolean))]],
  ["agent", "Agent", (run) => [run.agent ?? "none"]],
  ["step", "Step", (run) => [run.step]],
];

/** How long the run took, or has taken so far: "42 min", "2 h 5 min". @param {any} run @param {number} now */
export function duration(run, now) {
  const minutes = Math.max(0, Math.floor(((run.ended ? Date.parse(run.ended) : now) - Date.parse(run.started)) / 60000));
  return minutes < 60 ? `${minutes} min` : `${Math.floor(minutes / 60)} h ${minutes % 60} min`;
}

export { tokenText } from "./costs.js";
