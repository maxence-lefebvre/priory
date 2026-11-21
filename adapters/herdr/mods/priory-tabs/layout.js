// Turns a Priory view into a Herdr layout, and finds a view's blocks in the layout Herdr shows.
// It uses no Node or Herdr API, so the `layout` script, the tests and the mods share it.

/** The mod that shows each panel. A panel without one gets a `priory-elsewhere` block. */
export const MODS = {
  desk: "priory-desk",
  asks: "priory-asks",
  queue: "priory-queue",
  reports: "priory-bonjour",
  costs: "priory-costs",
  workflow: "priory-flows",
};

/** @param {any} node @param {string} id @returns {any[] | null} the nodes from `node` to the one with `id` */
function pathTo(node, id) {
  if (node?.id === id) return [node];
  for (const child of node?.children ?? []) {
    const path = pathTo(child, id);
    if (path) return [node, ...path];
  }
  return null;
}

/** @param {any} node @returns {string[]} */
const panels = (node) => (node.panel ? [node.panel] : Object.values(node).flat().flatMap(panels));

/**
 * The frame with its `work-body` laid out as the view: the view's tree beside the frame's `main`
 * block, or around it when the view has a `terminal` panel. Each node's id is its path in the tree.
 * @param {string} name @param {{ layout: any }} view @param {any} frame
 */
export function layout(name, view, frame) {
  if (panels(view.layout).filter((panel) => panel === "terminal").length > 1) {
    throw new Error(`view ${name} has two terminal panels, and Herdr has one main block`);
  }
  const root = structuredClone(frame);
  const body = pathTo(root, "work-body")?.at(-1);
  const main = pathTo(root, "main")?.at(-1);
  if (!body || !main) throw new Error("the frame needs a work-body block holding a main block");
  /** @param {any} node @param {string} id @returns {any} */
  const convert = (node, id) => {
    if (node.panel === "terminal") return main;
    if (node.panel) return { id, type: "block", content: { mod: MODS[node.panel] ?? "priory-elsewhere", props: { panel: node.panel } } };
    const [type, children] = Object.entries(node)[0];
    const converted = children.map((/** @type {any} */ child, /** @type {number} */ at) => convert(child, `${id}-${at}`));
    if (type === "tabs") return { id, type: "stack", active: converted[0].id, children: converted };
    return { id, type, children: converted.map((/** @type {any} */ child) => ({ ...child, size: { weight: 1 } })) };
  };
  const tree = convert(view.layout, "view");
  if (panels(view.layout).includes("terminal")) {
    body.children = [tree];
    return root;
  }
  // Without a terminal panel, the view sits beside `main` as the hand-written side panel did.
  body.children = [main, { ...tree, size: { px: 420 }, min: 280, max: 900 }];
  return root;
}

/** @param {any} node @param {string} panel @returns {any} the first block that shows `panel` */
export function panelBlock(node, panel) {
  if (node?.content?.props?.panel === panel) return node;
  for (const child of node?.children ?? []) {
    const found = panelBlock(child, panel);
    if (found) return found;
  }
  return null;
}

/**
 * The Herdr commands that bring the node `id` into view: show each hidden node on its path, and
 * select its branch in each stack, from the root down.
 * @param {any} root the layout Herdr shows @param {string} id @returns {[string, object][]}
 */
export function reveal(root, id) {
  const path = pathTo(root, id) ?? [];
  /** @type {[string, object][]} */
  const commands = [];
  path.forEach((node, at) => {
    if (node.hidden) commands.push(["layout.show", { id: node.id }]);
    const next = path[at + 1];
    if (node.type === "stack" && next && node.active !== next.id) commands.push(["layout.select", { stack: node.id, child: next.id }]);
  });
  return commands;
}
