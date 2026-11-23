// The Desk panel's view model: Priory's listings in, the count, sections and rows out. No drawing, no IO.

import { age, ago, askHeading, autolink, plural, prStack, routedMark, runnerOf, seenAgo, sentNote, sinceWords } from "./priory.js";

const STOPPED = ["blocked", "failed", "prompt", "delivery"];
const SNOOZABLE = ["ask", "blocked", "failed", "prompt"];
const RECOMMENDS = /** @type {Record<string, string>} */ ({ blocked: "Resume", prompt: "Resume", failed: "Revive", todo: "Done" });

/** @param {any} item */
const isSnoozed = (item) => item.snoozed_until != null;
const sameDay = (/** @type {string} */ iso, /** @type {number} */ now) => new Date(iso).toDateString() === new Date(now).toDateString();
const clock = (/** @type {string} */ iso) => new Date(iso).toLocaleTimeString("en", { hour: "2-digit", minute: "2-digit", hourCycle: "h23" });
const date = (/** @type {string} */ iso) => new Date(iso).toLocaleString("en", { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit", hourCycle: "h23" });
/** A time of day today, else a date. @param {string} iso @param {number} now */
export const when = (iso, now) => (sameDay(iso, now) ? clock(iso) : date(iso));

const DUE = /, due (\d{4}-\d\d-\d\dT[\d:]+Z)$/;
const localDay = (/** @type {number} */ at) => Date.UTC(new Date(at).getFullYear(), new Date(at).getMonth(), new Date(at).getDate()) / 86_400_000;

/**
 * A to-do's text with its `, due <RFC 3339 UTC>` ending in local time and a relative hint: a clock
 * time today, tomorrow or yesterday, else a date and a count of days. Other text is returned as is.
 * @param {string} what @param {number} now
 */
export function dueText(what, now) {
  const found = DUE.exec(what);
  const at = found ? Date.parse(found[1]) : NaN;
  if (!found || Number.isNaN(at)) return what;
  const clock = new Date(at).toLocaleTimeString("en", { hour: "2-digit", minute: "2-digit", hourCycle: "h23" });
  const days = localDay(at) - localDay(now);
  const near = { "-1": "yesterday", 0: "today", 1: "tomorrow" }[/** @type {"0"} */ (String(days))];
  const day = new Date(at).toLocaleString("en", { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit", hourCycle: "h23" });
  const hint = days > 0 ? `in ${days} days` : `${-days} days ago`;
  return `${what.slice(0, found.index)}, due ${near ? `${near} ${clock}` : `${day} (${hint})`}`;
}

/** @param {any} item @param {number} now */
const itemText = (item, now) => (item.kind === "todo" ? dueText(item.what, now) : item.what);

/** The web view's `memoryTag`: whether a candidate replaces or forgets a memory runs get now. @param {any} candidate */
function memoryTag(candidate) {
  if (candidate.forget) return candidate.replaces ? { tag: "forgets a memory runs get", warn: true } : { tag: "forget", warn: false };
  if (candidate.replaces == null) return { tag: "may replace a memory runs get", warn: true };
  return candidate.replaces ? { tag: "replaces a memory runs get", warn: true } : { tag: "new", warn: false };
}

/** A memory candidate as the memories row's detail shows it: the full text, never cut. @param {any} candidate */
const memoryView = (candidate) => ({
  id: candidate.id,
  name: candidate.name,
  ...memoryTag(candidate),
  description: candidate.description,
  text: candidate.forget ? `Forgets ${candidate.name}: runs stop getting it.` : candidate.text,
  current: candidate.current_text ?? null,
  by: `Written by ${candidate.task}${candidate.runner ? ` on ${candidate.runner}` : ""}, candidate ${candidate.id}`,
});

/** The pause or stall sentence with when it began in relative words, as the web Desk's deskBanners. @param {string | null | undefined} text @param {number} now */
const began = (text, now) => text?.replace(/^(paused|stalled) since (\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ)/, (_, what, iso) => `${what} ${ago(iso, now)}`);

/** @param {any} progress a task's published progress, or null */
function progressLines(progress) {
  if (!progress) return null;
  const criteria = progress.acceptanceCriteria ?? [];
  return [
    progress.goal ? `Goal: ${progress.goal}` : null,
    criteria.length ? plural(criteria.length, "acceptance criterion", "acceptance criteria") : null,
    progress.latestChange ? `Latest: ${progress.latestChange}` : null,
    progress.nextAction ? `Next: ${progress.nextAction}` : null,
  ].filter(Boolean);
}

/**
 * The Desk at `now`: the count, the next thing, the room banner, the saved views and each view's
 * sections. Agent text passes through as text, and as `autolink` spans where the web view links it.
 * @param {{ desk: any, asks: any, tasks: any, queue: any, memories?: any }} data `memories` as `priory memories DIR --json` lists them
 * @param {{ now: number, links?: { pattern: string, url: string }[] }} options `links` as `priory links` lists them
 */
export function deskModel({ desk, asks: asksListing, tasks: tasksListing, queue, memories }, { now, links = [] }) {
  /** @type {Map<unknown, any>} */
  const askById = new Map(asksListing.asks.map((/** @type {any} */ ask) => [ask.id, ask]));
  /** @type {any[]} */
  const tasks = tasksListing.tasks;
  const taskById = new Map(tasks.map((task) => [task.id, task]));

  /** @param {any} item a desk item */
  function itemRow(item) {
    const ask = item.kind === "ask" ? askById.get(item.id) : null;
    const task = taskById.get(item.task);
    const back = item.back_early;
    /** @type {any[] | null} */
    const candidates = item.kind === "memories" ? memories?.candidates ?? item.candidates ?? [] : null;
    if (candidates) item = { ...item, what: `${plural(candidates.length, "memory candidate")}: ${candidates.map((candidate) => candidate.name).join(", ")}` };
    const text = ask ? askHeading(ask) : ["blocked", "failed"].includes(item.kind) && task?.reason ? task.reason : itemText(item, now);
    return {
      id: `${item.kind}:${item.id}`,
      kind: item.kind,
      // The desk lists an outside ask without its key; `priory asks` carries the one it sent.
      key: item.key ?? ask?.key ?? null,
      step: item.step,
      age: item.since ? age(item.since, now) : null,
      // The desk's `what` for a failed task ends with its revive command; the task's reason does not.
      text,
      textSpans: autolink(text, links, ask?.repo ?? null),
      recommends: ask ? ask.recommendation ?? null : RECOMMENDS[item.kind] ?? null,
      note: isSnoozed(item) ? `Snoozed until ${when(item.snoozed_until, now)}`
        : back ? `Back early: ${back.reason}${back.reason === "new revision" && ask ? ` ${ask.revision}` : ""}` : null,
      ask: item.kind === "ask" ? item.id : null,
      // Snooze stays on its own runner: another runner's ask cannot be snoozed here.
      snooze: SNOOZABLE.includes(item.kind) && !runnerOf(ask) ? { kind: item.kind, id: item.id } : null,
      snoozed: isSnoozed(item),
      todo: item.kind === "todo" ? item.id : null,
      report: item.kind === "report" ? item.id : null,
      task: item.task,
      prs: prStack(task?.stack).label,
      revision: ask?.revision ?? null,
      progress: progressLines(task?.progress),
      ...(candidates && { memories: candidates.map(memoryView) }),
      ...(routedMark(item) && { routed: routedMark(item) }),
      ...(runnerOf(ask) && { runner: runnerOf(ask), away: seenAgo(ask, now, (iso) => age(iso, now)), sent: sentNote(ask) }),
    };
  }

  /** @param {any} task */
  const taskRow = (task) => {
    const text = task.tldr || task.reason || task.progress?.latestChange || task.workflow;
    return {
    id: `task:${task.id}`,
    kind: task.state,
    key: task.key,
    step: task.step,
    age: task.since ? age(task.since, now) : null,
    text,
    textSpans: autolink(text, links, task.repo ?? null),
    recommends: /** @type {string | null} */ (null),
    note: /** @type {string | null} */ (null),
    snooze: /** @type {{ kind: string, id: number | string } | null} */ (null),
    snoozed: false,
    todo: /** @type {number | null} */ (null),
    report: /** @type {number | null} */ (null),
    task: task.id,
    prs: prStack(task.stack).label,
    revision: /** @type {number | null} */ (null),
    progress: progressLines(task.progress),
    };
  };

  /** @type {any[]} */
  const items = desk.items;
  const shown = items.filter((item) => !isSnoozed(item));
  const asks = shown.filter((item) => item.kind === "ask");
  const stopped = shown.filter((item) => STOPPED.includes(item.kind));
  const todos = shown.filter((item) => item.kind === "todo");
  const later = shown.filter((item) => !["ask", "queue", "todo"].includes(item.kind) && !STOPPED.includes(item.kind));
  const snoozed = items.filter(isSnoozed);
  const asleep = desk.counts.asleep;
  const inState = (/** @type {string} */ state) => tasks.filter((task) => task.state === state);

  const line = [
    asks.length ? `Oldest waits ${age(asks[0].since, now)}.` : null,
    stopped.length ? `${stopped.length} stopped.` : null,
    desk.counts.snoozed ? `${desk.counts.snoozed} snoozed.` : null,
    desk.counts.unrouted ? `${plural(desk.counts.unrouted, "event")} reached no task in 24 h.` : null,
    desk.counts.unheard ? `${plural(desk.counts.unheard, "event")} no agent read.` : null,
    desk.counts.dropped ? `${plural(desk.counts.dropped, "request")} a script dropped in 24 h.` : null,
  ].filter(Boolean).join(" ");

  /** @type {Record<string, () => { title: string, note: string | null, rows: any[] }[]>} */
  const views = {
    needs: () => [
      { title: "Asks", note: `${asks.length}, oldest first`, rows: asks.map(itemRow) },
      { title: "Stopped", note: String(stopped.length), rows: stopped.map(itemRow) },
      { title: "To-dos", note: String(todos.length), rows: todos.map(itemRow) },
      {
        title: "Later",
        note: "no action needed",
        rows: [
          ...(asleep ? [{ ...taskRow({ id: "asleep", state: "asleep", tldr: `${plural(asleep, "task")} asleep on waits` }), id: "asleep", task: null, count: asleep }] : []),
          ...later.map(itemRow),
        ],
      },
    ],
    running: () => [{ title: "Running", note: null, rows: inState("running").map(taskRow) }],
    asleep: () => [{ title: "Asleep", note: "on waits", rows: inState("asleep").map(taskRow) }],
    snoozed: () => [{ title: "Snoozed", note: "the agents keep waiting", rows: snoozed.map(itemRow) }],
    all: () => [{ title: "All tasks", note: "in start order", rows: tasks.map(taskRow) }],
  };

  return {
    count: { number: asks.length, label: asks.length === 1 ? "ask needs you" : "asks need you", line },
    next: desk.next ? nextThing(desk.next, desk.next.kind === "ask" ? askById.get(desk.next.id) : null, taskById.get(desk.next.task), now, links) : null,
    empty: asks.length || stopped.length ? null : "No open asks. Agents ask here when they need you.",
    views: [
      { id: "needs", label: "Needs me", count: asks.length + stopped.length + todos.length },
      { id: "running", label: "Running", count: inState("running").length },
      { id: "asleep", label: "Asleep", count: asleep },
      { id: "snoozed", label: "Snoozed", count: snoozed.length },
      { id: "all", label: "All tasks", count: tasks.length },
    ],
    sections: (/** @type {string} */ view) => (views[view] ?? views.needs)().filter((section) => section.rows.length),
    banners: [began(desk.paused, now), began(desk.banner, now), desk.intake, ...(desk.caps ?? [])].filter(Boolean),
    since: {
      from: desk.last_visit ? when(desk.last_visit, now) : null,
      rows: desk.since_you_looked.map((/** @type {any} */ event) => ({ key: event.key, task: event.task, what: sinceWords(event.what), time: age(event.time, now) })),
    },
    upNext: queue.up_next.map((/** @type {any} */ entry) => ({
      when: entry.kind === "queued" ? "now" : when(entry.at, now),
      key: entry.key,
      task: entry.task,
      why: entry.why,
    })),
    upNextEmpty: "Nothing is queued or scheduled to run next.",
  };
}

/**
 * The desk's `next`: an ask, or an alarm that ranks above every ask.
 * @param {any} item the desk's `next` @param {any} ask the same ask in `priory asks`, when it is one @param {any} task @param {number} now
 * @param {{ pattern: string, url: string }[]} links
 */
function nextThing(item, ask, task, now, links) {
  const progress = task?.progress;
  const isAsk = item.kind === "ask";
  // As in `itemRow`: only a blocked or failed task's reason replaces the desk's `what`.
  const question = isAsk ? (ask ? askHeading(ask) : null) || itemText(item, now) : ["blocked", "failed"].includes(item.kind) && task?.reason ? task.reason : itemText(item, now);
  return {
    ask: isAsk ? item.id : null,
    snooze: SNOOZABLE.includes(item.kind) && !runnerOf(ask) ? { kind: item.kind, id: item.id } : null,
    revision: ask?.revision ?? null,
    todo: item.kind === "todo" ? item.id : null,
    task: item.task,
    key: item.key ?? ask?.key ?? null,
    step: item.step,
    age: age(item.since, now),
    why: item.why,
    question,
    questionSpans: autolink(question, links, ask?.repo ?? null),
    recommendation: isAsk ? ask?.recommendation ?? null : RECOMMENDS[item.kind] ?? null,
    tradeoff: ask?.tradeoff ?? null,
    progress: progress ? { latestChange: progress.latestChange ?? null, nextAction: progress.nextAction ?? null } : null,
    ...(routedMark(item) && { routed: routedMark(item) }),
    ...(runnerOf(ask) && { runner: runnerOf(ask), away: seenAgo(ask, now, (iso) => age(iso, now)), sent: sentNote(ask) }),
  };
}

/**
 * The events the panel shows under Since you looked: those it opened on, then each newer one.
 * Its own `priory seen` moves the visit, so a later desk lists only what came after.
 * @param {any[]} kept @param {any[]} fresh `since_you_looked`, newest first
 */
export function mergeSince(kept, fresh) {
  const id = (/** @type {any} */ event) => `${event.time}|${event.task}|${event.what}`;
  const known = new Set(kept.map(id));
  return [...fresh.filter((event) => !known.has(id(event))), ...kept];
}

/** `hour`:00 local time tomorrow, for `priory snooze --until`. @param {number} hour @param {number} now */
export function tomorrowAt(hour, now) {
  const at = new Date(now);
  at.setDate(at.getDate() + 1);
  at.setHours(hour, 0, 0, 0);
  return at.toISOString();
}
