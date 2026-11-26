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
 * @typedef {{ id: string, state: "open" | "agreed" | "applied", options: Option[], chosen: string | null, note: string | null, recommended: string | null }} Pick
 * @typedef {ReturnType<typeof row>} Row
 * @typedef {{ state?: "all" | "open" | "picked", recommends?: (string | null)[] }} Filter a section's chips:
 *   the picks' state, and the options they recommend, null for none
 * @typedef {{ state: { id: "all" | "open" | "picked", count: number }[], recommends: { option: string | null, count: number }[] }} Chips
 * @typedef {{ id: string, title: string, count: number, rows: Row[], chips: Chips | null, filter: Required<Filter>,
 *   recommended: { row: string, pick: string, option: string }[] }} Section
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
      recommended: pick.recommended ?? null,
    };
  }
  // The recommended option carries its own mark, so an open pick needs no note.
  return { id: pick.id, state: "open", options, note: null, chosen: null, recommended: pick.recommended ?? null };
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
 * @param {{ now: number, view?: string, links?: { pattern: string, url: string }[], query?: string, filters?: Record<string, Filter> }} options
 *   `view` is "all", the default, or "picks"; `links` as `priory links` lists them; `query` keeps the rows that hold it;
 *   `filters` holds each section's chips by its id
 */
export function dossier(read, { now, view = "all", links = [], query = "", filters = {} }) {
  const { report } = read;
  const needle = query.trim().toLowerCase();
  const picks = report.sections.flatMap((/** @type {any} */ section) => section.rows).map((/** @type {any} */ row) => row.pick).filter(Boolean);
  const agreed = picks.filter((/** @type {any} */ pick) => pick.chosen != null).length;
  const open = picks.length - agreed;
  const since = Date.parse(report.since);
  const from = sameDay(since, now) ? clock(since) : `${new Date(since).toLocaleDateString("en", { weekday: "short" })} ${clock(since)}`;
  const left = !picks.length ? "" : open ? ` ${open} ${open === 1 ? "pick" : "picks"} to agree.` : " Every pick is agreed.";
  /** @type {Section[]} */
  const sections = report.sections.map((/** @type {any} */ section) => {
    const all = section.rows.map((/** @type {any} */ candidate, /** @type {number} */ index) => row(section, candidate, index, now, links))
      .filter((/** @type {Row} */ candidate) => view !== "picks" || candidate.pick?.state === "open");
    const filter = { state: filters[section.id]?.state ?? "all", recommends: filters[section.id]?.recommends ?? [] };
    const rows = all.filter((candidate) => keeps(candidate, filter) && (!needle || holds(candidate, needle)));
    return {
      id: section.id,
      title: section.title,
      count: section.count ?? section.rows.length,
      rows,
      chips: chips(all, filter.state),
      filter,
      recommended: rows.flatMap((candidate) => (candidate.pick?.state === "open" && candidate.pick.recommended != null
        ? [{ row: candidate.id, pick: candidate.pick.id, option: candidate.pick.recommended }] : [])),
    };
  });
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

const picked = (/** @type {Row} */ candidate) => candidate.pick != null && candidate.pick.state !== "open";

/** Whether a section's chips keep `candidate`: a state chip other than all, or a recommends chip, keeps only picks. @param {Row} candidate @param {Required<Filter>} filter */
function keeps(candidate, { state, recommends }) {
  if (state !== "all" && (!candidate.pick || picked(candidate) !== (state === "picked"))) return false;
  return !recommends.length || (candidate.pick != null && recommends.includes(candidate.pick.recommended));
}

/**
 * A section's chips with their counts, none for a section without a pick. The recommends chips count
 * the rows the state chip keeps, in the order their options first show, with None last.
 * @param {Row[]} rows @param {Required<Filter>["state"]} state @returns {Chips | null}
 */
function chips(rows, state) {
  const picks = rows.filter((candidate) => candidate.pick);
  if (!picks.length) return null;
  const open = picks.filter((candidate) => !picked(candidate)).length;
  /** @type {Map<string | null, number>} */
  const counts = new Map();
  for (const candidate of picks.filter((each) => keeps(each, { state, recommends: [] }))) {
    const option = candidate.pick?.recommended ?? null;
    counts.set(option, (counts.get(option) ?? 0) + 1);
  }
  const none = counts.get(null);
  counts.delete(null);
  if (none) counts.set(null, none);
  return {
    state: [{ id: "all", count: rows.length }, { id: "open", count: open }, { id: "picked", count: picks.length - open }],
    recommends: [...counts].map(([option, count]) => ({ option, count })),
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
