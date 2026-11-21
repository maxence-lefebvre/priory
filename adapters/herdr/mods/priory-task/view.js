// The Task panel's view model: `priory task ID --json` in, what the panel draws out. No drawing, no IO.

import { autolink, prStack } from "./priory.js";

// The runs History shows, newest first.
const HISTORY = 10;

const sameDay = (/** @type {string} */ iso, /** @type {number} */ now) => new Date(iso).toDateString() === new Date(now).toDateString();
const clock = (/** @type {string} */ iso) => new Date(iso).toLocaleTimeString("en", { hour: "2-digit", minute: "2-digit", hourCycle: "h23" });
const date = (/** @type {string} */ iso) => new Date(iso).toLocaleString("en", { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit", hourCycle: "h23" });
/** A time of day today, else a date. @param {string} iso @param {number} now */
const when = (iso, now) => (sameDay(iso, now) ? clock(iso) : date(iso));

/** @param {{ conclusion: string }[]} checks */
function checksLine(checks) {
  if (!checks.length) return null;
  const failing = checks.filter((check) => !["success", "skipped", "neutral"].includes(check.conclusion)).length;
  return failing ? `${failing} of ${checks.length} checks failing` : `${checks.length} checks pass`;
}

/**
 * One task as the Task panel draws it: its state, step, asks, pull request and recent runs. Agent
 * text passes through as text, and the title as `autolink` spans.
 * @param {any} task `priory task ID --json`
 * @param {{ now: number, links?: { pattern: string, url: string }[] }} options
 */
export function taskView(task, { now, links = [] }) {
  const title = task.tldr || task.brief?.title || task.brief?.request?.split("\n")[0] || task.key;
  const pr = task.pull_request;
  /** @type {any[]} */
  const asks = task.asks ?? [];
  return {
    id: task.id,
    key: task.key,
    state: task.state,
    context: [task.workflow, task.step, task.lane, task.band].filter(Boolean),
    title,
    titleSpans: autolink(title, links, task.repo ?? null),
    reason: task.reason ?? null,
    // Open asks first, each group in the order priory lists it.
    asks: [...asks.filter((ask) => ask.state === "open"), ...asks.filter((ask) => ask.state !== "open")].map((ask) => ({
      id: ask.id,
      tile: ask.state === "open" ? "ask" : ask.state,
      text: ask.title || ask.question,
      note: ask.state === "open" ? `Open since ${when(ask.since, now)}` : ask.state === "answered" ? `Answered ${ask.answer}` : "Withdrawn",
    })),
    pr: pr ? {
      title: `#${pr.number} ${pr.title ?? ""}`.trim(),
      url: pr.url,
      line: `${pr.branch} into ${pr.base}, ${pr.state}`,
      checks: checksLine(pr.checks ?? []),
    } : null,
    stack: prStack(task.stack, pr).entries,
    history: [...(task.runs ?? [])].reverse().slice(0, HISTORY).map((run) => ({
      id: run.id,
      tile: run.ended ? run.end || "moved" : "running",
      text: `${run.step}, run ${run.id}${run.outcome ? `, ${run.outcome}` : ""}`,
      when: when(run.started, now),
    })),
  };
}
