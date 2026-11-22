// The batch page and the go ask's plan (DESIGN.md, Batch page and Lane graph), as pure functions.
import { taskLabel } from "./tasks.js";

/**
 * One row per entry in start order, each in a lane, and one edge per `after`. An entry continues the
 * lane of the first predecessor it is the last to wait on; otherwise it takes the lowest lane whose
 * entry has no successor left below. A lane stays busy down to its entry's last successor, so an
 * edge runs down its predecessor's lane and turns into the entry's lane just above it.
 * @param {{ entry: string, after: { entry: string, moment: string }[] }[]} entries
 * @param {number} max the most lanes the width fits; entries past it share the last lane
 */
export function laneLayout(entries, max = Infinity) {
  const at = new Map(entries.map((entry, i) => [entry.entry, i]));
  const edges = entries.flatMap((entry, to) =>
    entry.after.filter((edge) => at.has(edge.entry)).map((edge) => ({ from: at.get(edge.entry), to, moment: edge.moment })));
  const last = entries.map((_, i) => Math.max(i, ...edges.filter((edge) => edge.from === i).map((edge) => edge.to)));
  const busyUntil = [];
  const lanes = [];
  entries.forEach((_, i) => {
    const kept = edges.find((edge) => edge.to === i && last[edge.from] === i);
    const free = busyUntil.findIndex((until) => until < i);
    const lane = kept ? lanes[kept.from] : free < 0 ? Math.min(busyUntil.length, max - 1) : free;
    busyUntil[lane] = Math.max(busyUntil[lane] ?? -1, last[i]);
    lanes.push(lane);
  });
  return { rows: entries.map((entry, i) => ({ entry, lane: lanes[i] })), lanes: busyUntil.length, edges };
}

/** The order the stage bar and its words show a batch's stages, as the desk line counts them. */
const STAGES = ["merged", "started", "waiting", "queued", "blocked", "planned", "failed", "dropped"];

/** Each stage with entries, and how many. @param {Record<string, number>} counts */
export const stages = (counts) => STAGES.filter((stage) => counts[stage] > 0).map((stage) => [stage, counts[stage]]);

/** An entry's issue as a row names it, without its source: `ENG-41.1` for a part of a split issue. */
export const issueName = (entry) => (entry.part == null ? taskLabel(entry.issue) : `${taskLabel(entry.issue)}.${entry.part}`);

/**
 * When an entry starts, in words: `starts at the go`, `stacks on #1 ENG-40`, or `starts once #1
 * ENG-40 merges`, with the predecessor's place in the plan.
 */
export function momentWords(entry, entries) {
  const named = (moment) => entry.after.filter((edge) => edge.moment === moment).map((edge) => {
    const at = entries.findIndex((other) => other.entry === edge.entry);
    return at < 0 ? edge.entry : `#${at + 1} ${issueName(entries[at])}`;
  });
  const [stacks, merges] = [named("waiting"), named("merged")];
  const words = [
    stacks.length ? `stacks on ${stacks.join(" and ")}` : "",
    merges.length ? `starts once ${merges.join(" and ")} ${merges.length > 1 ? "merge" : "merges"}` : "",
  ].filter(Boolean);
  return words.length ? words.join(", then ") : "starts at the go";
}

/** Every entry that starts after entry `id`, directly or not, in plan order. */
export function successors(entries, id) {
  const after = new Set([id]);
  for (const entry of entries) if (entry.after.some((edge) => after.has(edge.entry))) after.add(entry.entry);
  after.delete(id);
  return [...after];
}

/**
 * The `POST /api/batches/{id}/edit` body for one tap of an entry's menu: `up`, `down` or `drop`,
 * bound to the plan revision the page drew. Null when the entry has no neighbour that way. A drop
 * takes the entry's successors with it, since they would start without the work they need.
 */
export function editBody(revision, entries, id, action) {
  const at = entries.findIndex((entry) => entry.entry === id);
  if (action === "drop") return { revision, drop: id, ...(successors(entries, id).length ? { with_successors: true } : {}) };
  const other = entries[action === "up" ? at - 1 : at + 1];
  if (at < 0 || !other) return null;
  return { revision, move: id, [action === "up" ? "before" : "after"]: other.entry };
}
