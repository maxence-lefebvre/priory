// The Flows panel's pure parts: node shapes, edge styles and the top to bottom timeline of one
// workflow, from `GET /api/flows`. No drawing, no IO.

export const KINDS = ["read", "reversible", "irreversible", "ask", "answer", "verdict"];

const WRITES = { reversible: "reversible write", irreversible: "irreversible write" };
const NODE = { width: 150, height: 52 };
const GAP = { column: 24, row: 60, edge: 12 };
const LANE = { pad: 8, step: 14 };
const PER_ROW = 2;

/**
 * The outline of a node of `kind` in a `width` by `height` box, as an SVG path; each kind has its own silhouette.
 * @param {string} kind @param {number} width @param {number} height
 * @returns {{ d: string }}
 */
export function nodeShape(kind, width, height) {
  const [w, h] = [width, height];
  const cut = 14;
  const polygon = (/** @type {number[][]} */ points) => `M${points.map(([x, y]) => `${x} ${y}`).join("L")}Z`;
  const shapes = {
    read: `M${h / 2} 0H${w - h / 2}A${h / 2} ${h / 2} 0 0 1 ${w - h / 2} ${h}H${h / 2}A${h / 2} ${h / 2} 0 0 1 ${h / 2} 0Z`,
    reversible: polygon([[0, 0], [w, 0], [w, h], [0, h]]),
    irreversible: polygon([[cut, 0], [w - cut, 0], [w, cut], [w, h - cut], [w - cut, h], [cut, h], [0, h - cut], [0, cut]]),
    ask: polygon([[cut, 0], [w - cut, 0], [w, h / 2], [w - cut, h], [cut, h], [0, h / 2]]),
    answer: polygon([[cut, 0], [w, 0], [w - cut, h], [0, h]]),
    verdict: polygon([[w / 2, 0], [w, h / 2], [w / 2, h], [0, h / 2]]),
  };
  const known = /** @type {keyof typeof shapes} */ (kind in shapes ? kind : "reversible");
  return { d: shapes[known] };
}

/**
 * The dash and weight of an edge by who decides it; its color comes from its class.
 * @param {string} by
 * @returns {{ dash: string, width: number }}
 */
export function edgeStyle(by) {
  const styles = {
    code: { dash: "6 2", width: 1.5 },
    model: { dash: "9 5", width: 2 },
    human: { dash: "", width: 3.5 },
    asleep: { dash: "1 5", width: 2.5 },
  };
  return styles[/** @type {keyof typeof styles} */ (by)] ?? styles.code;
}

/**
 * The step pairs of `workflow` that lie on a path to an irreversible step with no human decision.
 * @param {string} workflow @param {{ workflow: string, path: string[] }[]} paths `paths` of the flows
 * @returns {Set<string>} `from>to`
 */
export function ungatedPairs(workflow, paths) {
  const pairs = new Set();
  for (const entry of paths) {
    if (entry.workflow !== workflow) continue;
    entry.path.slice(1).forEach((to, at) => pairs.add(`${entry.path[at]}>${to}`));
  }
  return pairs;
}

/**
 * One workflow laid out top to bottom: each step one row below the latest step that leads to it,
 * `done` and `stop` in the last row, and an edge for each outcome.
 * @param {any} workflow one entry of the flows' `workflows`
 * @param {Set<string>} ungated `from>to` pairs, from `ungatedPairs`
 */
export function timeline(workflow, ungated) {
  /** @type {Map<string, any>} */
  const steps = new Map(workflow.steps.map((/** @type {any} */ step) => [step.id, step]));
  /** @type {Map<string, number>} */
  const row = new Map();
  /** @type {Set<string>} */
  const back = new Set();
  const place = (/** @type {string} */ id, /** @type {number} */ depth, /** @type {string[]} */ trail) => {
    if (!steps.has(id) || row.get(id) >= depth) return;
    row.set(id, depth);
    for (const next of steps.get(id).edges) {
      if (trail.includes(next.to) || next.to === id) back.add(`${id}>${next.to}>${next.outcome}`);
      else place(next.to, depth + 1, [...trail, id]);
    }
  };
  place(workflow.start, 0, []);
  const last = Math.max(0, ...row.values()) + 1;
  const ends = ["done", "stop"].filter((end) => workflow.steps.some((/** @type {any} */ step) => step.edges.some((/** @type {any} */ next) => next.to === end)));
  for (const end of ends) row.set(end, last);
  const unplaced = workflow.steps.filter((/** @type {any} */ step) => !row.has(step.id));
  unplaced.forEach((/** @type {any} */ step) => row.set(step.id, last - 1));

  /** @type {string[][]} */
  const depths = [];
  for (const id of [...steps.keys(), ...ends]) (depths[row.get(id) ?? 0] ??= []).push(id);
  const rows = depths.flatMap((ids = []) => Array.from({ length: Math.ceil(ids.length / PER_ROW) }, (_, at) => ids.slice(at * PER_ROW, (at + 1) * PER_ROW)));
  const columns = Math.max(...rows.map((ids) => ids.length));
  const block = columns * NODE.width + (columns - 1) * GAP.column + 2 * GAP.edge;
  const gateOf = (/** @type {any} */ step) => {
    if (step.kind !== "irreversible") return null;
    if (step.autonomous) return `autonomous: ${step.autonomous}`;
    return step.approved_by?.length ? `approved by ${step.approved_by.join(", ")}` : "no approval";
  };

  const nodes = rows.flatMap((ids, at) => {
    const inner = ids.length * NODE.width + (ids.length - 1) * GAP.column;
    return ids.map((id, index) => {
      const step = steps.get(id);
      const x = (block - inner) / 2 + index * (NODE.width + GAP.column);
      const base = { id, x, y: GAP.edge + at * (NODE.height + GAP.row), width: NODE.width, height: NODE.height };
      if (!step) return { ...base, kind: "end", terminal: true, label: id, gate: null, decides: null };
      return { ...base, kind: step.kind, terminal: false, label: step.id, gate: gateOf(step), decides: step.decides };
    });
  });
  const at = (/** @type {string} */ id) => nodes.find((node) => node.id === id);
  let lanes = 0;
  const edges = workflow.steps.flatMap((/** @type {any} */ step) => step.edges.map((/** @type {any} */ next, /** @type {number} */ index) => {
    const [from, to] = [at(step.id), at(next.to)];
    const parallel = step.edges.findIndex((/** @type {any} */ other) => other.to === next.to) < index;
    const looped = back.has(`${step.id}>${next.to}>${next.outcome}`);
    const start = { x: from.x + from.width / 2, y: from.y + from.height };
    const end = { x: to.x + to.width / 2, y: to.y };
    // A lane keeps an edge that skips a row, loops back or repeats another's ends off the nodes.
    const skips = looped || parallel || to.y - from.y > NODE.height + GAP.row + 1;
    const lane = skips ? block + lanes++ * LANE.step : 0;
    const d = skips
      ? `M${start.x} ${start.y}C${start.x} ${start.y + 24} ${lane} ${start.y + 24} ${lane} ${start.y + 44}V${end.y - 44}C${lane} ${end.y - 24} ${end.x} ${end.y - 24} ${end.x} ${end.y}`
      : `M${start.x} ${start.y}C${start.x} ${start.y + 30} ${end.x} ${end.y - 30} ${end.x} ${end.y}`;
    const mid = skips ? { x: lane - 3, y: (start.y + end.y) / 2 } : { x: (start.x + end.x) / 2, y: (start.y + end.y) / 2 };
    return {
      from: step.id, to: next.to, outcome: next.outcome, by: next.by, d, mid, back: looped, vertical: skips,
      ungated: next.by !== "human" && ungated.has(`${step.id}>${next.to}`),
    };
  }));
  const width = block + lanes * LANE.step + LANE.pad;
  const height = (rows.length) * (NODE.height + GAP.row) - GAP.row + 2 * GAP.edge;
  const text = [
    `Workflow ${workflow.name} starts at ${workflow.start}.`,
    ...workflow.steps.map((/** @type {any} */ step) => `${step.id}: ${WRITES[step.kind] ?? step.kind}, ${step.edges.map((/** @type {any} */ next) => `${next.outcome} to ${next.to} decided by ${next.by}`).join("; ")}.`),
  ].join(" ");
  return { nodes, edges, width, height, text };
}
