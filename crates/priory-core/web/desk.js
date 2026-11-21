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
