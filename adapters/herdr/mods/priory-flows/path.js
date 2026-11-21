// Turns the absolute SVG paths that view.js draws (M, L, H, V, C, A and Z) into gpui-kit
// PathBuilder calls, scaled. No drawing, no IO.

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
