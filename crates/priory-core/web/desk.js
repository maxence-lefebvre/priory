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
