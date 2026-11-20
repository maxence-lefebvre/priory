// The Queue panel's view model: `priory queue DIR --json` in, rows and states out. No drawing, no IO.

const title = (/** @type {string} */ name) => name.charAt(0).toUpperCase() + name.slice(1);
const clock = (/** @type {string} */ iso) => new Date(iso).toLocaleTimeString("en", { hour: "2-digit", minute: "2-digit", hourCycle: "h23" });

const sameDay = (/** @type {string} */ iso, /** @type {number} */ now) => new Date(iso).toDateString() === new Date(now).toDateString();
/** A time of day today, else its weekday and time. @param {string} iso @param {number} now */
const when = (iso, now) => (sameDay(iso, now) ? clock(iso) : `${new Date(iso).toLocaleDateString("en", { weekday: "short" })} ${clock(iso)}`);

/** Relative under 24 h, then a date (DESIGN.md, Copy). @param {string} iso @param {number} now */
function age(iso, now) {
  const minutes = Math.max(0, Math.round((now - Date.parse(iso)) / 60_000));
  if (minutes < 60) return `${minutes} min`;
  if (minutes < 1440) return `${Math.floor(minutes / 60)} h`;
  return new Date(iso).toLocaleString("en", { month: "short", day: "numeric" });
}

/** @typedef {{ paused?: string | null, banner?: string | null }} Room `paused` and `banner` from `priory desk --json` */

/** "paused since TIME[: REASON]" as the person reads it. @param {string} paused */
function pausedSince(paused) {
  const [, since, reason] = /^paused since (\S+?)(?:: (.*))?$/.exec(paused) ?? [];
  return { since: since ? clock(since) : null, reason: reason ?? null };
}

/** Why the waiting `task` has not started. @param {any} task @param {any} before the task before it in start order @param {Room} room */
function why(task, before, room) {
  if (task.position === 1 && room.paused) {
    const { reason } = pausedSince(room.paused);
    return { text: `Starts are paused${reason ? `: ${reason}` : "."}`, tone: "warn" };
  }
  // "stalled since TIME: short of RESOURCE by AMOUNT; task N waits for room"
  const stall = /^stalled since (\S+): (short of .+); task (\d+) waits for room$/.exec(room.banner ?? "");
  if (stall && Number(stall[3]) === task.id) return { text: `Waits for room: ${stall[2]} since ${clock(stall[1])}.`, tone: "warn" };
  if (!before) return { text: "Starts when the runner has room.", tone: "muted" };
  if (task.band_position === 1) return { text: `Starts after ${before.key}, the band above.`, tone: "muted" };
  return { text: `Starts after ${before.key}.`, tone: "muted" };
}

/** One condition of a hold, as a person reads it. @param {any} condition @param {Map<number, string>} keys @param {number} now */
function condition(condition, keys, now) {
  if (condition.met) return { text: condition.met_as ?? "Met", met: true };
  const text = {
    person: () => "Until a person releases it",
    time: () => `Until ${when(condition.until, now)}`,
    task: () => `Until ${keys.get(condition.task) ?? `task ${condition.task}`} ends`,
    issue: () => `Until ${condition.resource} is done`,
    merge: () => `Until ${condition.resource} merges`,
    blockers: () => `Until the blockers of ${condition.resource} are done`,
  }[/** @type {string} */ (condition.kind)];
  return { text: text ? text() : condition.kind, met: false };
}

/** Each value of a field, in first-seen order, with how many tasks carry it. @param {any[]} tasks @param {string} field */
function counted(tasks, field) {
  const counts = new Map();
  for (const task of tasks) if (task[field] != null) counts.set(task[field], (counts.get(task[field]) ?? 0) + 1);
  return [...counts];
}

/**
 * The saved views with their counts, and the chosen view's sections, each left out when empty.
 * @param {any} queue
 * @param {{ view: string, now: number, room?: Room, facets?: { lane?: string, band?: string } }} options
 */
export function queueList(queue, { view, now, room = {}, facets = {} }) {
  const all = queue.waiting.filter((/** @type {any} */ task) => task.state !== "held");
  // Running runs and ended tasks carry no lane or band, so a facet leaves them out.
  const faceted = Boolean(facets.lane || facets.band);
  const shown = (/** @type {any} */ task) => (!facets.lane || task.lane === facets.lane) && (!facets.band || task.band === facets.band);
  const waiting = all.filter(shown);
  const held = queue.waiting.filter((/** @type {any} */ task) => task.state === "held" && shown(task));
  const runs = faceted ? [] : queue.running;
  const done = faceted ? [] : queue.ended;
  /** @param {string} id @param {string} name @param {any[]} list @param {(item: any) => any} row */
  const section = (id, name, list, row) => ({ id, title: name, note: String(list.length), rows: list.map(row) });
  /** @param {any} task */
  const waitingRow = (task) => ({
    id: task.id,
    tile: task.state,
    key: task.key,
    title: task.title,
    step: task.step,
    position: task.band_position,
    why: why(task, all[all.indexOf(task) - 1], room),
  });
  const bands = [...new Set(waiting.map((/** @type {any} */ task) => task.band))].map((band) =>
    section(`band-${band}`, title(band), waiting.filter((/** @type {any} */ task) => task.band === band), waitingRow));
  const running = section("running", "Running", runs, (run) => ({ id: run.task, tile: "running", key: run.key, title: run.title, step: run.step, run: run.run, age: age(run.started, now) }));
  /** @type {Map<number, string>} */
  const keys = new Map([...queue.waiting, ...queue.ended].map((task) => [task.id, task.key]));
  for (const run of queue.running) keys.set(run.task, run.key);
  const holds = section("held", "Held", held, (task) => ({
    id: task.id,
    tile: "held",
    key: task.key,
    title: task.title,
    step: task.step,
    band: title(task.band),
    why: { text: title(task.reason ?? "held"), tone: "held" },
    conditions: (task.conditions ?? []).map((/** @type {any} */ item) => condition(item, keys, now)),
  }));
  const ended = section("ended", "Last day", done, (task) => ({
    id: task.id, tile: task.state, key: task.key, title: task.title, step: task.step, age: age(task.ended, now), why: { text: task.reason ?? task.state, tone: "muted" },
  }));
  const sections = { next: [running, ...bands, holds, ended], held: [holds], ended: [ended], all: [running, ...bands, holds, ended] }[view] ?? [];
  const paused = room.paused ? pausedSince(room.paused) : null;
  return {
    waiting: all.length,
    paused: paused && `Starts are paused${paused.since ? ` since ${paused.since}` : ""}${paused.reason ? `: ${paused.reason}` : "."}`,
    views: [
      { id: "next", label: "Next", count: waiting.length },
      { id: "held", label: "Held", count: held.length },
      { id: "ended", label: "Last day", count: done.length },
      { id: "all", label: "All", count: runs.length + waiting.length + held.length + done.length },
    ],
    facets: { lane: counted(queue.waiting, "lane"), band: counted(queue.waiting, "band") },
    sections: sections.filter((candidate) => candidate.rows.length),
  };
}

/** @param {number} n */
const ordinal = (n) => `${n}${n % 10 === 1 && n % 100 !== 11 ? "st" : n % 10 === 2 && n % 100 !== 12 ? "nd" : n % 10 === 3 && n % 100 !== 13 ? "rd" : "th"}`;

/** The day under a time: today, a weekday within the week, else a date. @param {string} iso @param {number} now */
function day(iso, now) {
  if (sameDay(iso, now)) return "today";
  const date = new Date(iso);
  if (Date.parse(iso) - now < 6 * 86_400_000) return date.toLocaleDateString("en", { weekday: "short" });
  return date.toLocaleDateString("en", { month: "short", day: "numeric" });
}

/** Why a queued task is next, from the `why` of `priory queue --json`. @param {string} why */
function queuedWhy(why) {
  if (why === "first in the queue") return "First in the queue. It starts once there is room.";
  if (why === "a band above it") return "First in its band, after the band above.";
  const position = /^queue position (\d+)$/.exec(why);
  return position ? `${ordinal(Number(position[1]))} in the queue.` : why;
}

/**
 * The next three runs, queued, held or scheduled, each with when it starts and why.
 * @param {any} queue @param {number} now
 * @returns {{ task: number | null, when: string, sub: string, label: string, why: string }[]}
 */
export function upNext(queue, now) {
  return queue.up_next.map((/** @type {any} */ next) => {
    if (next.kind === "queued") return { task: next.task, when: "now", sub: "on room", label: next.key, title: next.title, why: queuedWhy(next.why) };
    const label = next.kind === "scheduled" && next.task == null ? `${next.trigger}, schedule` : next.key;
    const why = next.kind === "held" ? "Its hold ends." : next.task == null ? "The schedule starts a task." : "The schedule wakes it.";
    return { task: next.task, when: clock(next.at), sub: day(next.at, now), label, title: next.title, why };
  });
}

/** The tasks in `task`'s band that wait in start order, so a reorder stays inside it. @param {any} queue @param {any} task */
const bandmates = (queue, task) => queue.waiting.filter((/** @type {any} */ other) => other.state !== "held" && other.band === task.band);

/**
 * The `priory move` place that moves task `id` one step (Shift J and K) or onto another row (the grip),
 * or null when that leaves its band or changes nothing.
 * @param {any} queue @param {number} id @param {number | { onto: number }} to
 * @returns {{ before: number } | { after: number } | null}
 */
export function reorder(queue, id, to) {
  const task = queue.waiting.find((/** @type {any} */ candidate) => candidate.id === id);
  if (!task || task.state === "held") return null;
  const band = bandmates(queue, task).map((/** @type {any} */ other) => other.id);
  const from = band.indexOf(id);
  const at = typeof to === "number" ? from + to : band.indexOf(to.onto);
  if (at < 0 || at >= band.length || at === from) return null;
  return at < from ? { before: band[at] } : { after: band[at] };
}

/** What Move… offers: the top of the task's band, then each other band of the factory. @param {any} queue @param {number} id */
export function moveTargets(queue, id) {
  const task = queue.waiting.find((/** @type {any} */ candidate) => candidate.id === id);
  if (!task) return [];
  const top = task.state !== "held" && task.band_position !== 1 ? [{ label: `To the top of ${title(task.band)}`, move: { top: true } }] : [];
  const bands = queue.bands.filter((/** @type {string} */ band) => band !== task.band);
  return [...top, ...bands.map((band) => ({ label: `To ${title(band)}`, move: { band } }))];
}
