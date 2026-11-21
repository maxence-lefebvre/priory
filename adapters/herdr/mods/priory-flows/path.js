// The drawing's geometry for gpui-kit: the absolute SVG paths that view.js draws (M, L, H, V, C, A
// and Z) as PathBuilder calls, scaled, and where each edge's outcome label sits. No drawing, no IO.

// An 11 px label is about this wide per character, and a line this tall; main.js clips a lane's label to 10 characters.
const LABEL = { char: 6.5, line: 16, lane: 10 };

/**
 * Each edge's label: beside a lane edge, else beside its curve, at its middle. gpui-kit turns no
 * text, so an upright lane label that would cover one placed before it moves down a line.
 * @param {{ outcome: string, mid: { x: number, y: number }, vertical: boolean }[]} edges
 * @param {number} scale
 * @returns {{ x: number, y: number }[]}
 */
export function labelSpots(edges, scale) {
  /** @type {{ x: number, y: number, width: number }[]} */
  const placed = [];
  return edges.map((edge) => {
    const x = edge.mid.x * scale + (edge.vertical ? 6 : 5);
    let y = edge.mid.y * scale - LABEL.line / 2;
    if (!edge.vertical) return { x, y };
    const width = Math.min(edge.outcome.length, LABEL.lane) * LABEL.char;
    while (placed.some((other) => Math.abs(other.y - y) < LABEL.line && x < other.x + other.width && other.x < x + width)) y += LABEL.line;
    placed.push({ x, y, width });
    return { x, y };
  });
}

/** @typedef {[string, ...(number | boolean)[]]} Call */

/**
 * @param {string} d an SVG path with absolute commands only, as view.js writes them
 * @param {number} scale what each coordinate and radius is multiplied by
 * @returns {Call[]}
 */
export function pathCalls(d, scale) {
  const tokens = d.match(/[A-Za-z]|-?\d*\.?\d+(?:e[-+]?\d+)?/g) ?? [];
  /** @type {Call[]} */
  const calls = [];
  let [x, y] = [0, 0];
  let at = 0;
  const next = () => Number(tokens[at++]) * scale;
  while (at < tokens.length) {
    const command = tokens[at++];
    if (command === "M" || command === "L") {
      [x, y] = [next(), next()];
      calls.push([command === "M" ? "move_to" : "line_to", x, y]);
    } else if (command === "H") {
      x = next();
      calls.push(["line_to", x, y]);
    } else if (command === "V") {
      y = next();
      calls.push(["line_to", x, y]);
    } else if (command === "C") {
      const [ax, ay, bx, by] = [next(), next(), next(), next()];
      [x, y] = [next(), next()];
      calls.push(["cubic_bezier_to", x, y, ax, ay, bx, by]);
    } else if (command === "A") {
      const [rx, ry] = [next(), next()];
      const [rotation, large, sweep] = [Number(tokens[at++]), tokens[at++] === "1", tokens[at++] === "1"];
      [x, y] = [next(), next()];
      calls.push(["arc_to", rx, ry, rotation, large, sweep, x, y]);
    } else if (command === "Z") {
      calls.push(["close"]);
    } else {
      throw new Error(`pathCalls: unsupported command ${command}`);
    }
  }
  return calls;
}
