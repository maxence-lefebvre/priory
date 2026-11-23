import { taskLabel } from "./tasks.js";

export const ASK_VIEWS = ["open", "snoozed", "answered", "all"];

/** The groups of `askGroups` a saved view lists, in order. @param {string} view */
export const shownGroups = (view) => (view === "all" ? ["open", "snoozed", "answered", "withdrawn"] : [view]);

/** The `POST /api/snooze` body of the S key: 1 h on an open ask of a task here, else null. @param {any} ask @param {number} now */
export const snoozeShortcut = (ask, now) =>
  ask?.state === "open" && ask.task && !runnerOf(ask) && !snoozed(ask, now) ? { kind: "ask", id: ask.id, for: "1h" } : null;

export const snoozed = (ask, now) => ask.state === "open" && Boolean(ask.snoozed_until) && Date.parse(ask.snoozed_until) > now;

// The asks list's groups. An ask snoozed past `now` leaves Open for Snoozed (DESIGN.md, Snooze).
export function askGroups(asks, now) {
  // `since` may carry any offset, so compare instants, not strings.
  const open = asks.filter((ask) => ask.state === "open").sort((a, b) => (Date.parse(a.since) || 0) - (Date.parse(b.since) || 0));
  return {
    open: open.filter((ask) => !snoozed(ask, now)),
    snoozed: open.filter((ask) => snoozed(ask, now)),
    answered: asks.filter((ask) => ask.state === "answered").sort((a, b) => (b.answered_at || "").localeCompare(a.answered_at || "")),
    withdrawn: asks.filter((ask) => ask.state === "withdrawn").reverse(),
  };
}

/**
 * The open asks of this runner's task, snoozed ones too, oldest first: its own, and those its task
 * lists, such as another task's ask that its held result waits on. @param {any[]} asks @param {{ id: number, asks?: any[] }} task
 */
export const openAsksOf = (asks, task) =>
  asks
    .filter((ask) => (ask.task === task.id || task.asks?.some((own) => own.id === ask.id)) && !runnerOf(ask) && ask.state === "open")
    .sort((a, b) => Date.parse(a.since) - Date.parse(b.since));

/** "with <lead>" for an open ask or desk item routed to a lead, else null. @param {any} ask */
export const routedMark = (ask) => (ask?.routed_to && (ask.state ?? "open") === "open" ? `with ${ask.routed_to}` : null);

/** What an open ask's detail says of its lead, else null. @param {any} ask */
export const routedNote = (ask) => (routedMark(ask) ? `${ask.routed_to} is answering. You can still answer first: the first answer stands.` : null);

/** The runner of another runner's ask, which alone lists `submissions`, else null. @param {any} ask */
export const runnerOf = (ask) => (Array.isArray(ask?.submissions) ? ask.runner : null);

// How long a runner may be quiet before its asks say when it was last seen: alive messages come once a minute.
const AWAY_MS = 3 * 60 * 1000;

/**
 * "last seen <age> ago" for another runner's ask whose runner is away, else null.
 * @param {any} ask @param {number} now @param {(iso: string) => string} ago
 */
export function seenAgo(ask, now, ago) {
  if (!runnerOf(ask)) return null;
  if (!ask.last_seen) return "never seen";
  return now - Date.parse(ask.last_seen) > AWAY_MS ? `last seen ${ago(ask.last_seen)} ago` : null;
}

/**
 * What became of the latest answer or dismiss sent to another runner's ask, while it waits or once
 * refused, else null: an applied one shows in the ask itself. @param {any} ask
 */
export function sentNote(ask) {
  const runner = runnerOf(ask);
  const last = runner ? ask.submissions.at(-1) : null;
  if (!last) return null;
  const what = last.verb === "dismiss" ? "the dismiss" : last.text;
  if (last.outcome === "refused") return { text: `${runner} refused ${what}: ${last.why}`, refused: true };
  if (last.outcome) return null;
  return { text: last.verb === "dismiss" ? `Dismiss sent to ${runner}` : `Sent to ${runner}: ${last.text}`, refused: false };
}

const SUBMITTED = { answered: "answered", dismissed: "withdrawn", refused: "refused" };

/** Each answer and dismiss sent to another runner's ask as an answer history row, oldest first. @param {any} ask */
export function submitted(ask) {
  const runner = runnerOf(ask);
  if (!runner) return [];
  return ask.submissions.map((/** @type {any} */ sent) => {
    const outcome = sent.outcome === "refused" ? `${runner} refused it: ${sent.why}` : sent.outcome ? `${runner} applied it` : "Not applied yet";
    const what = sent.verb === "dismiss" ? "the dismiss" : sent.text;
    return { tile: SUBMITTED[sent.outcome] ?? "queued", text: `Sent ${what} to ${runner} for revision ${sent.revision}. ${outcome}`, at: sent.sent };
  });
}

/** Who answered, as the views say it: a person, `agent:<lead>`, auto, or `session <name>` for a person's own session. Null when unrecorded. @param {string | null | undefined} by */
export const answeredBy = (by) => (!by ? null : by.startsWith("agent:") ? by : by === "auto" ? "auto" : by.startsWith("session:") ? `session ${by.slice(8)}` : "a person");

/** Whether an ask takes free text and a dismissal: a step's or a hold's ask takes only its options. */
export const takesText = (ask) => ask?.free_text !== false;

// How long a first tap on an option stays armed for the second.
export const ARM_MS = 4000;

/** Whether `armed` still arms `option` of this revision of the ask, as the Herdr panel's `press` does. */
export const stillArmed = (armed, ask, option, now) =>
  armed != null && armed.id === ask.id && armed.revision === ask.revision && armed.text === option && now - armed.at < ARM_MS;

/**
 * What a tap on `option` does: the first tap arms it, and a second tap on it within ARM_MS, at the
 * same revision, sends it at once, with `run` set for an option that runs a command (R6.8).
 * @param {any} armed @param {any} ask @param {string} option @param {number} now
 */
export function press(armed, ask, option, now) {
  if (stillArmed(armed, ask, option, now)) {
    const run = Array.isArray(ask.command?.runs) && ask.command.runs.includes(option);
    return { send: { text: option, run } };
  }
  return { arm: { id: ask.id, revision: ask.revision, text: option, at: now } };
}

/**
 * Why an ask write did not apply, for the note on the ask. @param {number} status @param {any} body
 * @param {(iso: string) => string} ago a time as the page says its age @param {any} [ask] the ask written to
 */
export function writeNote(status, body, ago, ask) {
  const fresh = body?.ask;
  if (status === 409 && fresh) {
    if (fresh.state === "open") return `The question changed. Read revision ${fresh.revision} and answer again.`;
    return fresh.state === "answered" ? `Already answered ${ago(fresh.answered_at)} ago: ${fresh.answer}.` : `Withdrawn: ${fresh.withdrawn_why}.`;
  }
  if (status === 503 && runnerOf(ask)) return `Could not send: ${body?.error}.`;
  if (status === 503) return "Could not send: no priory daemon runs beside this server. Start priory daemon, then send again.";
  if (status === 504 || status === 0) return "The daemon may have applied this but did not confirm it. Check the ask's state before you send again.";
  return `Could not send: ${body?.error || `the server answered ${status}`}.`;
}

/** The ask detail's header once its options scroll away: the heading, over the task key, step and revision. */
export const shrunkHead = (ask) => ({
  question: askHeading(ask),
  place: [taskLabel(ask.key), ask.step, `revision ${ask.revision}`].filter(Boolean).join(", "),
});

/** Who answered, as the answered row's icon: auto, session, agent or person; null when unrecorded. @param {string | null | undefined} by */
export const answererOf = (by) => (!by ? null : by === "auto" ? "auto" : by.startsWith("session:") ? "session" : by.startsWith("agent:") ? "agent" : "person");

/** A confidence's bars, of three. */
export const CONFIDENCE_BARS = { high: 3, medium: 2, low: 1 };

/**
 * The answered asks of this runner's task `task`, newest answer first, as its page shows them: who
 * answered, the confidence and rationale the agent stated, and the recommendation when another option won.
 * @param {any[]} asks @param {number} task
 */
export const answeredAsks = (asks, task) => asks
  .filter((ask) => ask.task === task && !runnerOf(ask) && ask.state === "answered")
  .sort((a, b) => (Date.parse(b.answered_at) || 0) - (Date.parse(a.answered_at) || 0))
  .map((ask) => ({
    id: ask.id,
    title: askHeading(ask),
    question: ask.question ?? null,
    answer: ask.answer,
    at: ask.answered_at ?? null,
    answerer: answererOf(ask.answered_by),
    by: answeredBy(ask.answered_by),
    confidence: ask.confidence ?? null,
    bars: CONFIDENCE_BARS[ask.confidence] ?? 0,
    rationale: ask.rationale ?? null,
    tradeoff: ask.tradeoff ?? null,
    recommended: ask.recommendation != null && ask.recommendation !== ask.answer ? ask.recommendation : null,
  }));

/** What follows an auto answer's confidence word: whose it is, and the level its task answers at. @param {string | null} level */
export const confidenceNote = (level) => `confidence, the agent's own.${level ? ` The task answers at ${level} and above.` : ""}`;

/** The longest heading of an untitled ask, in characters, before its ellipsis. */
export const HEADING_MAX = 80;

/**
 * An ask's heading, where its text is a heading or one line: its title; else its question when that
 * is one line of at most 80 characters; else one derived from its task's key and step, or from the
 * question's first sentence when it has no task, cut at a word to about 80 characters. The detail
 * shows the full question in its body. @param {any} ask
 */
export function askHeading(ask) {
  if (ask?.title) return ask.title;
  const question = String(ask?.question ?? "").trim();
  if (question.length <= HEADING_MAX && !question.includes("\n")) return question;
  const key = String(ask?.key ?? "").replace(/^(linear|github|slack|cli):/, "");
  if (key && ask?.step) return cutAt(`${key} asks at ${ask.step}`, HEADING_MAX);
  const end = question.search(/[.?!](\s|$)|\n/);
  const first = end < 0 ? question : question.slice(0, question[end] === "\n" ? end : end + 1).trim();
  const step = ask?.step && !first.startsWith(ask.step) ? `${ask.step}: ` : "";
  return cutAt(`${step}${first}`, HEADING_MAX);
}

/** `text` cut at the last word that ends within `max` characters, with an ellipsis. @param {string} text @param {number} max */
export function cutAt(text, max) {
  if (text.length <= max) return text;
  const space = text.slice(0, max + 1).lastIndexOf(" ");
  const kept = space > max / 2 ? text.slice(0, space) : text.slice(0, max);
  return `${kept.replace(/[\s.,;:!?-]+$/, "")}…`;
}

/** The full question, for the detail's body when its heading does not show all of it, else null. @param {any} ask */
export const askBody = (ask) => (ask?.question && askHeading(ask) !== ask.question ? ask.question : null);
