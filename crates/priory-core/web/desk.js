// The web Desk's pure parts: where the next thing opens, Since you looked, a task's progress and
// the section chip in view. No drawing, no IO.

/** The page that answers the desk's `next`: the ask, the task page, the intake banner, or none. @param {any} item */
export function nextLink(item) {
  if (!item) return null;
  if (item.kind === "ask") return `/asks/${encodeURIComponent(item.id)}`;
  if (item.task != null) return `/tasks/${item.task}`;
  if (item.kind === "report" && String(item.what).startsWith("intake: ")) return "#intake";
  return null;
}

/** Whether an Up next run is a queued task waiting for its place, which starts on room, not for its own reason. @param {any} run */
export const onRoom = (run) => run.kind === "queued" && /^(first in the queue|a band above it|queue position \d+)$/.test(run.why);

/** `since_you_looked`, newest first, as one group per task, the task with the newest event first. @param {any[]} events */
export function sinceGroups(events) {
  /** @type {Map<unknown, { task: number | null, key: string | null, events: any[] }>} */
  const groups = new Map();
  for (const event of events) {
    if (!groups.has(event.task)) groups.set(event.task, { task: event.task, key: event.key, events: [] });
    groups.get(event.task)?.events.push(event);
  }
  return [...groups.values()];
}

/**
 * The events the Desk shows: those it opened on, then each newer one. Its own `POST /api/seen`
 * moves the visit, so a later desk lists only what came after. The Herdr Desk's `mergeSince`.
 * @param {any[]} kept @param {any[]} fresh `since_you_looked`, newest first
 */
export function mergeSince(kept, fresh) {
  const id = (/** @type {any} */ event) => `${event.time}|${event.task}|${event.what}`;
  const known = new Set(kept.map(id));
  return [...fresh.filter((event) => !known.has(id(event))), ...kept];
}

/** A task's published `progress` for its page, or null until it has a field. @param {any} progress */
export function progressView(progress) {
  const criteria = (progress?.acceptanceCriteria ?? []).map((/** @type {any} */ criterion) => ({
    id: criterion.id,
    description: criterion.description,
    state: criterion.required ? "Required" : "Optional",
  }));
  const view = { goal: progress?.goal ?? null, criteria, latestChange: progress?.latestChange ?? null, nextAction: progress?.nextAction ?? null };
  return view.goal || criteria.length || view.latestChange || view.nextAction ? view : null;
}

/**
 * The section whose chip lights: the last one whose top passed under the sticky chip bar, or the
 * last one once the scroller reaches its end, where a short last section never reaches the top.
 * @param {{ tops: number[], scrollTop: number, height: number, scrollHeight: number, bar: number }} scroller
 */
export function sectionInView({ tops, scrollTop, height, scrollHeight, bar }) {
  if (scrollTop + height >= scrollHeight - 1) return tops.length - 1;
  return Math.max(tops.length ? 0 : -1, tops.findLastIndex((top) => top <= scrollTop + bar));
}

const DUE = /, due (\d{4}-\d\d-\d\dT[\d:]+Z)$/;
const localDay = (/** @type {number} */ at) => Date.UTC(new Date(at).getFullYear(), new Date(at).getMonth(), new Date(at).getDate()) / 86_400_000;

/**
 * A to-do's text split at its `, due <RFC 3339 UTC>` ending: the text before it, the UTC time, and
 * the time in local time with a relative hint (a clock time today, tomorrow or yesterday, else a
 * date and a count of days). Null when the text has no valid due ending.
 * @param {string} what @param {number} now
 */
export function dueParts(what, now) {
  const found = DUE.exec(what);
  const at = found ? Date.parse(found[1]) : NaN;
  if (!found || Number.isNaN(at)) return null;
  const clock = new Date(at).toLocaleTimeString("en", { hour: "2-digit", minute: "2-digit", hourCycle: "h23" });
  const days = localDay(at) - localDay(now);
  const near = { "-1": "yesterday", 0: "today", 1: "tomorrow" }[/** @type {"0"} */ (String(days))];
  const day = new Date(at).toLocaleString("en", { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit", hourCycle: "h23" });
  const hint = days > 0 ? `in ${days} days` : `${-days} days ago`;
  return { text: what.slice(0, found.index), iso: found[1], label: near ? `${near} ${clock}` : `${day} (${hint})` };
}

/** A to-do's text with its due time in local time, or as it is without one. @param {string} what @param {number} now */
export function dueText(what, now) {
  const parts = dueParts(what, now);
  return parts ? `${parts.text}, due ${parts.label}` : what;
}

/**
 * The write a desk row offers: Read for a report, Release for a held task, with the revision the
 * page drew as `priory release --expected` takes it. Null for a row with no action here.
 * @param {any} item @param {number} revision
 */
export function rowAction(item, revision) {
  if (item.kind === "report") return { label: "Read", path: `/api/inbox/${item.id}/read`, body: {} };
  if (item.kind === "held") return { label: "Release", path: `/api/tasks/${item.task}/release`, body: { revision } };
  return null;
}

/** Why a row action did not apply, for the note under the row. @param {string} label @param {number} status @param {any} body */
export function actionNote(label, status, body) {
  if (status === 409) return "The state changed since this page loaded. Read the task again, then tap again.";
  if (status === 503) return "Could not send: no priory daemon runs beside this server. Start priory daemon, then send again.";
  if (status === 504 || status === 0) return "The daemon may have applied this but did not confirm it. Check the desk before you send again.";
  return `Could not ${label.toLowerCase()} it: ${body?.error || `the server answered ${status}`}.`;
}

/** A `batch` row's line split into its counts and its `Next: ...` sentence, or null. @param {string} what */
export function batchParts(what) {
  // The daemon's sentence comes last, after a title that may hold the same words.
  const at = what.lastIndexOf(" Next: ");
  return at < 0 ? { line: what, next: null } : { line: what.slice(0, at), next: what.slice(at + 1) };
}

/** A memory candidate's tag: whether it replaces or forgets a memory runs get now. @param {any} candidate */
export function memoryTag(candidate) {
  if (candidate.forget) return candidate.replaces ? { label: "forgets a memory runs get", warn: true } : { label: "forget", warn: false };
  if (candidate.replaces == null) return { label: "may replace a memory runs get", warn: true };
  return candidate.replaces ? { label: "replaces a memory runs get", warn: true } : { label: "new", warn: false };
}

/** What an accept or a drop did, or why it was not sent, for the note under the candidate. @param {string} verb @param {number} status @param {any} body */
export function memoryNote(verb, status, body) {
  if (status === 200) return `${body.ok.charAt(0).toUpperCase()}${body.ok.slice(1)}, then it leaves this list.`;
  if (status === 404 || status === 409) return `Not sent: ${body.error}. The list reads it again.`;
  if (status === 503) return `Could not send: ${body.error}.`;
  if (status === 504 || status === 0) return "The server may have sent this but did not confirm it. Check the list before you send again.";
  return `Could not ${verb} it: ${body?.error || `the server answered ${status}`}.`;
}

/** A count and its noun: "1 task", "2 tasks"; `many` covers irregular plurals. @param {number} count @param {string} one @param {string} [many] */
/** The repository of task `id` in `tasks`, so a Desk row's `#n` links where its task lives; null without one. */
export const taskRepo = (tasks, id) => (id == null ? null : (tasks || []).find((task) => task.id === id)?.repo ?? null);

export const plural = (count, one, many = `${one}s`) => `${count} ${count === 1 ? one : many}`;

/**
 * The line under the Desk's ask count: how long the oldest ask waits, then every other item the
 * Needs me view counts, so the two counts add up. @param {{ oldest: string | null, stopped: number, todos: number, reports: number, other: number }} counts
 */
export function countWords({ oldest, stopped, todos, reports, other }) {
  const rest = [
    stopped && `${stopped} stopped`,
    todos && plural(todos, "to-do"),
    reports && plural(reports, "report"),
    other && `${other} waiting`,
  ].filter(Boolean);
  const also = rest.length > 1 ? `Also ${rest.slice(0, -1).join(", ")} and ${rest.at(-1)}.` : rest.length ? `Also ${rest[0]}.` : "";
  return [oldest && `Oldest waits ${oldest}.`, also].filter(Boolean).join(" ");
}

/** The Desk's count: the number of open asks and its words, or no number when none is open, since stopped work can still wait. @param {number} n */
export const countHead = (n) => (n ? { n, words: n === 1 ? "ask needs you" : "asks need you" } : { n: null, words: "No asks need you" });

/** The Answer capsule's words for `n` open asks: offline it leaves the primary look and only opens the asks to read. */
export function capsuleWords(n, offline) {
  const asks = plural(n, "ask");
  return offline ? { text: `Read ${asks}, offline`, icon: "unplug", off: true } : { text: `Answer ${asks}`, icon: "ask", off: false };
}

/**
 * The ask a wide Desk opens in its drawer as the person arrives, else null. Arrival comes from
 * another page; a redraw, another saved view, or a closed Desk ask leaves the drawer empty.
 * @param {{ from: string | null, list: string, view: string, asks: { id: any }[] }} at
 */
export function arrivalAsk({ from, list, view, asks }) {
  if (view !== "needs" || !asks.length) return null;
  const [first, id] = (from || "").split(/[?#]/)[0].split("/").filter(Boolean);
  if (from !== null && (!first || first === "index.html")) return null;
  if (first === "asks" && id && list === "desk") return null;
  return asks[0].id;
}

/**
 * The Desk's banners, each the daemon's sentence: the pause, the room, the intake, the build output
 * reclaimed in the last 24 hours, then one per day or week spend cap reached, as `priory desk` prints them. The pause and the stall say when
 * they began in the page's words: "stalled 12 min ago", not "stalled since <time>".
 * @param {any} desk `/api/desk` @param {(iso: string) => string} ago
 */
export function deskBanners(desk, ago) {
  const began = (/** @type {string | null | undefined} */ text) => text?.replace(/^(paused|stalled) since (\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ)/, (_, what, iso) => `${what} ${ago(iso)}`);
  return [
    { text: began(desk.paused), icon: "pause", intake: false },
    { text: began(desk.banner), icon: "hard-drive", intake: false },
    { text: desk.intake, icon: "unplug", intake: true },
    { text: desk.reclaim, icon: "hard-drive", intake: false },
    ...(desk.caps ?? []).map((/** @type {string} */ text) => ({ text, icon: "hourglass", intake: false })),
  ].filter((banner) => banner.text);
}

const STOPPED = ["blocked", "failed", "prompt", "command"];

/**
 * What the Desk draws from its four listings: the open asks oldest first, as the asks listing holds
 * them, the ask the capsule opens, the stopped work, to-dos, reports and every other item, the
 * banners, and the queue's parts.
 * @param {{ desk: any, asks: any[], queue: any, tasks: any[] }} data @param {(iso: string) => string} ago
 */
export function deskModel({ desk, asks, queue, tasks }, ago) {
  const byId = new Map(asks.map((ask) => [ask.id, ask]));
  const items = desk.items.filter((/** @type {any} */ item) => item.kind !== "queue");
  const open = items.filter((/** @type {any} */ item) => item.kind === "ask").map((/** @type {any} */ item) => byId.get(item.id) || { ...item, question: item.what, options: [] });
  const of = (/** @type {string[]} */ kinds) => items.filter((/** @type {any} */ item) => kinds.includes(item.kind));
  // The capsule opens the ask that ranks first, not only the oldest one.
  const first = desk.ranked?.find((/** @type {any} */ item) => item.kind === "ask");
  return {
    next: desk.next, open, first: first ? byId.get(first.id) || open[0] : open[0],
    stopped: of(STOPPED), todos: of(["todo"]), reports: of(["report"]),
    other: items.filter((/** @type {any} */ item) => ![...STOPPED, "ask", "report", "todo"].includes(item.kind)),
    asleep: desk.counts.asleep, asleepTasks: queue.asleep, banners: deskBanners(desk, ago), intake: desk.intake,
    running: queue.running, queue: queue.waiting, tasks, upNext: queue.up_next, needs: items.length,
  };
}
