// The Desk panel's view model: Priory's listings in, the count, sections and rows out. No drawing, no IO.

import { autolink, prStack } from "./priory.js";

const STOPPED = ["blocked", "failed", "prompt", "delivery"];
const SNOOZABLE = ["ask", "blocked", "failed", "prompt"];
const RECOMMENDS = /** @type {Record<string, string>} */ ({ blocked: "Resume", prompt: "Resume", failed: "Revive" });

/** @param {any} item */
const isSnoozed = (item) => item.snoozed_until != null;
const sameDay = (/** @type {string} */ iso, /** @type {number} */ now) => new Date(iso).toDateString() === new Date(now).toDateString();
const clock = (/** @type {string} */ iso) => new Date(iso).toLocaleTimeString("en", { hour: "2-digit", minute: "2-digit", hourCycle: "h23" });
const date = (/** @type {string} */ iso) => new Date(iso).toLocaleString("en", { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit", hourCycle: "h23" });
/** A time of day today, else a date. @param {string} iso @param {number} now */
export const when = (iso, now) => (sameDay(iso, now) ? clock(iso) : date(iso));

/** Relative under 24 h, then a date (DESIGN.md, Copy). @param {string} iso @param {number} now */
export function age(iso, now) {
  const minutes = Math.max(0, Math.round((now - Date.parse(iso)) / 60_000));
  if (minutes < 60) return `${minutes} min`;
  if (minutes < 1440) return `${Math.floor(minutes / 60)} h`;
  return date(iso);
}

/** @param {any} progress a task's published progress, or null */
function progressLines(progress) {
  if (!progress) return null;
  const criteria = progress.acceptanceCriteria ?? [];
  return [
    progress.goal ? `Goal: ${progress.goal}` : null,
    criteria.length ? `${criteria.length} acceptance ${criteria.length === 1 ? "criterion" : "criteria"}` : null,
    progress.latestChange ? `Latest: ${progress.latestChange}` : null,
    progress.nextAction ? `Next: ${progress.nextAction}` : null,
  ].filter(Boolean);
}

/**
 * The Desk at `now`: the count, the next thing, the room banner, the saved views and each view's
 * sections. Agent text passes through as text, and as `autolink` spans where the web view links it.
 * @param {{ desk: any, asks: any, tasks: any, queue: any }} data
 * @param {{ now: number, links?: { pattern: string, url: string }[] }} options `links` as `priory links` lists them
 */
export function deskModel({ desk, asks: asksListing, tasks: tasksListing, queue }, { now, links = [] }) {
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
    const text = ask ? ask.title || ask.question : ["blocked", "failed"].includes(item.kind) && task?.reason ? task.reason : item.what;
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
      snooze: SNOOZABLE.includes(item.kind) ? { kind: item.kind, id: item.id } : null,
      snoozed: isSnoozed(item),
      task: item.task,
      prs: prStack(task?.stack).label,
      revision: ask?.revision ?? null,
      progress: progressLines(task?.progress),
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
  const later = shown.filter((item) => item.kind !== "ask" && item.kind !== "queue" && !STOPPED.includes(item.kind));
  const snoozed = items.filter(isSnoozed);
  const asleep = desk.counts.asleep;
  const inState = (/** @type {string} */ state) => tasks.filter((task) => task.state === state);

  const line = [
    asks.length ? `Oldest waits ${age(asks[0].since, now)}.` : null,
    stopped.length ? `${stopped.length} stopped.` : null,
    desk.counts.snoozed ? `${desk.counts.snoozed} snoozed.` : null,
  ].filter(Boolean).join(" ");

  /** @type {Record<string, () => { title: string, note: string | null, rows: any[] }[]>} */
  const views = {
    needs: () => [
      { title: "Asks", note: `${asks.length}, oldest first`, rows: asks.map(itemRow) },
      { title: "Stopped", note: String(stopped.length), rows: stopped.map(itemRow) },
      {
        title: "Later",
        note: "no action needed",
        rows: [
          ...(asleep ? [{ ...taskRow({ id: "asleep", state: "asleep", tldr: `${asleep} ${asleep === 1 ? "task" : "tasks"} asleep on waits` }), id: "asleep", task: null, count: asleep }] : []),
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
    next: asks.length ? nextThing(asks[0], askById.get(asks[0].id), taskById.get(asks[0].task), now, links) : null,
    empty: asks.length || stopped.length ? null : "No open asks. Agents ask here when they need you.",
    views: [
      { id: "needs", label: "Needs me", count: asks.length + stopped.length },
      { id: "running", label: "Running", count: inState("running").length },
      { id: "asleep", label: "Asleep", count: asleep },
      { id: "snoozed", label: "Snoozed", count: snoozed.length },
      { id: "all", label: "All tasks", count: tasks.length },
    ],
    sections: (/** @type {string} */ view) => (views[view] ?? views.needs)().filter((section) => section.rows.length),
    banners: [desk.paused, desk.banner, desk.intake].filter(Boolean),
    since: {
      from: desk.last_visit ? when(desk.last_visit, now) : null,
      rows: desk.since_you_looked.map((/** @type {any} */ event) => ({ key: event.key, task: event.task, what: event.what, time: when(event.time, now) })),
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
 * @param {any} item a desk ask item @param {any} ask the same ask in `priory asks` @param {any} task @param {number} now
 * @param {{ pattern: string, url: string }[]} links
 */
function nextThing(item, ask, task, now, links) {
  const progress = task?.progress;
  const question = ask?.title || ask?.question || item.what;
  return {
    ask: item.id,
    revision: ask?.revision ?? null,
    task: item.task,
    key: item.key ?? ask?.key ?? null,
    step: item.step,
    age: age(item.since, now),
    question,
    questionSpans: autolink(question, links, ask?.repo ?? null),
    recommendation: ask?.recommendation ?? null,
    tradeoff: ask?.tradeoff ?? null,
    progress: progress ? { latestChange: progress.latestChange ?? null, nextAction: progress.nextAction ?? null } : null,
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
