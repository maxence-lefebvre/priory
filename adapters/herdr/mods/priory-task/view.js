// The Task panel's view model: `priory task ID --json` in, what the panel draws out. No drawing, no IO.

import { age, ago, askHeading, autolink, eventWords, plural, prStack } from "./priory.js";

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

// A confidence's bars, of three, as the web's task page draws them.
const BARS = /** @type {Record<string, number>} */ ({ high: 3, medium: 2, low: 1 });

/** Who answered, as the web's answeredBy words it; a test keeps the two the same. @param {string | null | undefined} by */
const answeredBy = (by) => (!by ? null : by.startsWith("agent:") ? by : by === "auto" || by === "autopilot" ? by : by.startsWith("session:") ? `session ${by.slice(8)}` : "a person");

/** Who answered, as the row's tile: auto, session, agent or person, else null. @param {string | null | undefined} by */
const answererOf = (by) => (!by ? null : by === "auto" || by === "autopilot" ? "auto" : by.startsWith("session:") ? "session" : by.startsWith("agent:") ? "agent" : "person");

/** @param {any} ask */
function askNote(ask) {
  if (ask.state === "withdrawn") return "Withdrawn";
  const by = answeredBy(ask.answered_by);
  const note = `Answered ${ask.answer}${by ? ` by ${by}` : ""}`;
  return ask.confidence && answererOf(ask.answered_by) !== "auto" ? `${note}, ${ask.confidence}` : note;
}

/** @param {{ conclusion: string | null }[]} checks */
function checksLine(checks) {
  if (!checks.length) return null;
  const failing = checks.filter((check) => check.conclusion && !["success", "skipped", "neutral"].includes(check.conclusion)).length;
  const running = checks.filter((check) => !check.conclusion).length;
  return failing ? `${failing} of ${plural(checks.length, "check")} failing`
    : running ? `${running} of ${plural(checks.length, "check")} running`
    : plural(checks.length, "check passes", "checks pass");
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
    // Open asks in the order priory lists them, then answered ones newest first, then withdrawn ones.
    asks: [
      ...asks.filter((ask) => ask.state === "open"),
      ...asks.filter((ask) => ask.state === "answered").sort((a, b) => (Date.parse(b.answered_at) || 0) - (Date.parse(a.answered_at) || 0)),
      ...asks.filter((ask) => ask.state !== "open" && ask.state !== "answered"),
    ].map((ask) => {
      const auto = ask.state === "answered" && answererOf(ask.answered_by) === "auto";
      return {
        id: ask.id,
        tile: ask.state === "open" ? "ask" : ask.state === "answered" ? answererOf(ask.answered_by) ?? "answered" : ask.state,
        text: askHeading(ask),
        note: ask.state === "open" ? `Asked ${ago(ask.since, now)}` : askNote(ask),
        age: ask.state === "answered" && ask.answered_at ? age(ask.answered_at, now) : null,
        // Only an auto answer opens to its confidence and rationale: nobody chose it.
        confidence: auto ? ask.confidence ?? null : null,
        bars: auto ? BARS[ask.confidence] ?? 0 : 0,
        rationale: auto ? ask.rationale ?? null : null,
        auto,
      };
    }),
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
