// The report view model of the Bonjour panel and of the web view's Reports panel: `priory report
// show --json` in, sections, rows, picks and threads out. No drawing, no IO. It knows no section by
// name. crates/priory-core/web/reports.js is a copy that imports the web view's links.js; a test
// keeps the two the same.

import { autolink } from "./priory.js";

const sameDay = (/** @type {number} */ at, /** @type {number} */ now) => new Date(at).toDateString() === new Date(now).toDateString();
const clock = (/** @type {number} */ at) => new Date(at).toLocaleTimeString("en", { hour: "2-digit", minute: "2-digit", hourCycle: "h23" });
const date = (/** @type {number} */ at) => new Date(at).toLocaleString("en", { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit", hourCycle: "h23" });

/** A time of day today, else a date; a time that is not RFC 3339 shows as written. @param {string | null} text @param {number} now */
function when(text, now) {
  const at = Date.parse(text ?? "");
  if (Number.isNaN(at)) return text ?? null;
  return sameDay(at, now) ? clock(at) : date(at);
}

/**
 * @typedef {{ label: string, key: string, recommended: boolean }} Option
 * @typedef {{ id: string, state: "open" | "agreed" | "applied", options: Option[], chosen: string | null, note: string | null }} Pick
 * @typedef {ReturnType<typeof row>} Row
 * @typedef {{ id: string, title: string, count: number, rows: Row[] }} Section
 */

/**
 * An open pick's options; a chosen one folds to its choice and what the workflow did with it, and
 * keeps its options until the workflow applies it, since Change reopens them.
 * @param {any} pick @returns {Pick}
 */
function choice(pick) {
  const options = pick.options.map((/** @type {string} */ label, /** @type {number} */ index) => ({
    label,
    key: index < 9 ? String(index + 1) : "",
    recommended: label === pick.recommended,
  }));
  if (pick.chosen != null) {
    return {
      id: pick.id,
      state: pick.applied == null ? "agreed" : "applied",
      options: pick.applied == null ? options : [],
      chosen: pick.chosen,
      note: pick.applied ?? "Waiting for the workflow to apply it.",
    };
  }
  return { id: pick.id, state: "open", options, note: pick.recommended == null ? null : `Recommends ${pick.recommended}`, chosen: null };
}

/**
 * @param {any} section @param {any} row @param {number} index @param {number} now
 * @param {{ pattern: string, url: string }[]} links
 */
function row(section, row, index, now, links) {
  return {
    // A row without an id goes by its place: a number, which no row id (a string) equals.
    id: JSON.stringify([section.id, row.id ?? index]),
    tile: row.state ?? null,
    key: row.key ?? null,
    text: row.text,
    // A report names no repository, so only the factory's own links apply.
    textSpans: autolink(row.text ?? "", links, null),
    meta: row.meta ?? null,
    owner: row.owner ?? null,
    time: when(row.time, now),
    pick: row.pick ? choice(row.pick) : null,
    // The row's own id, which a comment names; a row without one takes no comments.
    target: row.id ?? null,
    threads: (row.comments ?? []).map((/** @type {any} */ thread) => ({
      ...comment(thread, now),
      resolved: thread.resolved,
      replies: thread.replies.map((/** @type {any} */ reply) => comment(reply, now)),
    })),
  };
}

/** @param {any} entry @param {number} now */
const comment = (entry, now) => ({ id: entry.id, author: entry.author, when: when(entry.created, now), text: entry.text });

/**
 * @param {{ revision: number, report: any }} read
 * @param {{ now: number, view?: string, links?: { pattern: string, url: string }[], query?: string }} options `view` is "all",
 *   the default, or "picks"; `links` as `priory links` lists them; `query` keeps the rows that hold it
 */
export function dossier(read, { now, view = "all", links = [], query = "" }) {
  const { report } = read;
  const needle = query.trim().toLowerCase();
  const picks = report.sections.flatMap((/** @type {any} */ section) => section.rows).map((/** @type {any} */ row) => row.pick).filter(Boolean);
  const agreed = picks.filter((/** @type {any} */ pick) => pick.chosen != null).length;
  const open = picks.length - agreed;
  const since = Date.parse(report.since);
  const from = sameDay(since, now) ? clock(since) : `${new Date(since).toLocaleDateString("en", { weekday: "short" })} ${clock(since)}`;
  const left = !picks.length ? "" : open ? ` ${open} ${open === 1 ? "pick" : "picks"} to agree.` : " Every pick is agreed.";
  /** @type {Section[]} */
  const sections = report.sections.map((/** @type {any} */ section) => ({
    id: section.id,
    title: section.title,
    count: section.count ?? section.rows.length,
    rows: section.rows.map((/** @type {any} */ candidate, /** @type {number} */ index) => row(section, candidate, index, now, links))
      .filter((/** @type {Row} */ candidate) => view !== "picks" || candidate.pick?.state === "open")
      .filter((/** @type {Row} */ candidate) => !needle || holds(candidate, needle)),
  }));
  const shown = sections.filter((section) => (view !== "picks" && !needle) || section.rows.length);
  return {
    task: report.task,
    revision: report.revision,
    waited: read.revision,
    date: new Date(`${report.date}T00:00:00Z`).toLocaleDateString("en", { month: "short", day: "numeric", timeZone: "UTC" }),
    greeting: report.greeting,
    since: `Since ${from}.${left}`,
    progress: { agreed, total: picks.length, label: picks.length ? `${agreed} of ${picks.length} agreed` : "No picks" },
    views: [{ id: "all", label: "All sections", count: null }, { id: "picks", label: "Picks", count: open }],
    sections: shown,
    // How many rows a search found; null without one.
    found: needle ? shown.reduce((sum, section) => sum + section.rows.length, 0) : null,
  };
}

/** Whether a row's key, text, meta, owner or a comment in its threads holds `needle`. @param {Row} row @param {string} needle */
const holds = (row, needle) => [row.key, row.text, row.meta, row.owner, ...row.threads.flatMap((thread) => [thread, ...thread.replies].map((entry) => entry.text))]
  .some((text) => text?.toLowerCase().includes(needle));

/**
 * The next row with a pick to agree after row `after`, wrapping to the first; null when none is left.
 * @param {ReturnType<typeof dossier>} view @param {string | null} after
 */
export function nextPick(view, after) {
  const rows = view.sections.flatMap((section) => section.rows);
  const at = rows.findIndex((candidate) => candidate.id === after);
  const open = (/** @type {Row} */ candidate) => candidate.pick?.state === "open";
  return (rows.slice(at + 1).find(open) ?? rows.find(open))?.id ?? null;
}

/**
 * Why a pick did not go, given what was sent, the report as Priory shows it now, and the CLI's error.
 * `from` is the choice a Change replaces.
 * @param {{ pick: string, option: string, revision: number, from?: string | null }} sent
 * @param {{ revision: number, report: any } | null} fresh
 * @param {string} error
 */
export function refusal(sent, fresh, error) {
  if (!fresh) return `Not sent: ${error}.`;
  const pick = fresh.report.sections.flatMap((/** @type {any} */ section) => section.rows)
    .map((/** @type {any} */ row) => row.pick).find((/** @type {any} */ candidate) => candidate?.id === sent.pick);
  if (!pick) return "The report no longer holds this pick.";
  if (pick.chosen != null && (pick.chosen !== sent.from || pick.applied != null)) return `Already picked ${pick.chosen}.`;
  if (fresh.report.revision !== sent.revision) return `The report changed to revision ${fresh.report.revision}. Read it and pick again.`;
  return `Not sent: ${error}.`;
}

/**
 * The tasks of the reports written just before and just after task `task`'s, from `priory report
 * list --json`'s reports, newest first; null at either end or when the list lacks the task.
 * @param {{ task: number }[]} reports @param {number} task
 */
export function around(reports, task) {
  const at = reports.findIndex((report) => report.task === task);
  if (at < 0) return { older: null, newer: null };
  return { older: reports[at + 1]?.task ?? null, newer: reports[at - 1]?.task ?? null };
}
