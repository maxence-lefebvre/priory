// The Task panel's view model: `priory task ID --json` in, what the panel draws out. No drawing, no IO.

import { age, ago, autolink, eventWords, prStack } from "./priory.js";

// The runs History shows, newest first.
const HISTORY = 10;
// The events the Inbox shows, newest first; `priory task ID` prints every one.
const INBOX = 20;

// What a prompt's state reads as, as the web view's inbox.js words it; a test keeps the two the same.
const PROMPTS = /** @type {Record<string, string>} */ ({
  confirmed: "delivered as prompt", queued: "still queued", attempted: "being typed", uncertain: "typed, unconfirmed", withdrawn: "prompt withdrawn",
});
const PROMPT_TILES = /** @type {Record<string, string>} */ ({ confirmed: "done", queued: "queued", attempted: "running", uncertain: "blocked", withdrawn: "dropped" });

/** What one inbox event did, as `priory task ID` prints it. @param {any} event @param {number} task */
function handled(event, task) {
  const own = event.task === task;
  const prompt = PROMPTS[event.prompt] || (event.outcome === "kept" && own ? "for its next run" : "");
  let text = prompt ? `${event.outcome}, ${prompt}` : event.outcome;
  if (event.wait != null) text += `, fired wait ${event.wait}`;
  if (event.task != null && !own) text += ` (routed to task ${event.task})`;
  return text;
}

/** @param {{ conclusion: string | null }[]} checks */
function checksLine(checks) {
  if (!checks.length) return null;
  const failing = checks.filter((check) => check.conclusion && !["success", "skipped", "neutral"].includes(check.conclusion)).length;
  const running = checks.filter((check) => !check.conclusion).length;
  return failing ? `${failing} of ${checks.length} checks failing`
    : running ? `${running} of ${checks.length} checks running`
    : `${checks.length} checks pass`;
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
      note: ask.state === "open" ? `Asked ${ago(ask.since, now)}` : ask.state === "answered" ? `Answered ${ask.answer}` : "Withdrawn",
    })),
    pr: pr ? {
      title: `#${pr.number} ${pr.title ?? ""}`.trim(),
      url: pr.url,
      line: `${pr.branch} into ${pr.base}, ${pr.state}`,
      checks: checksLine(pr.checks ?? []),
    } : null,
    stack: prStack(task.stack, pr).entries,
    inbox: (task.inbox ?? []).slice(0, INBOX).map((/** @type {any} */ event) => ({
      id: event.seq,
      tile: PROMPT_TILES[event.prompt] || (event.outcome === "kept" && event.task === task.id ? "asleep" : "ready"),
      text: `${eventWords(event.type)} ${event.subject ?? ""}`.trim(),
      note: `${handled(event, task.id)} · ${age(event.time, now)}`,
    })),
    inboxTotal: (task.inbox ?? []).length,
    history: [...(task.runs ?? [])].reverse().slice(0, HISTORY).map((run) => ({
      id: run.id,
      tile: run.ended ? run.end || "moved" : "running",
      text: `${run.step}, run ${run.id}${run.outcome ? `, ${run.outcome}` : ""}`,
      when: age(run.started, now),
      scores: (task.scores ?? []).filter((/** @type {any} */ score) => score.run === String(run.id))
        .map((/** @type {any} */ score) => ({ scorer: score.scorer, verdict: score.verdict, reason: score.reason })),
    })),
  };
}
