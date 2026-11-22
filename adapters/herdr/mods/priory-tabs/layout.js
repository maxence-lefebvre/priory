// Turns a Priory view into a Herdr layout, and finds a view's blocks in the layout Herdr shows.
// It uses no Node or Herdr API, so the `layout` script, the tests and the mods share it.

/**
 * Where each panel opens, and the mod that shows it. A narrow panel gets an aside, which the Priory
 * bar and a key toggle. A wide one opens as a tab in the center, beside `main`. `terminal` is
 * `main`, which is always in the center.
 * @type {Record<string, { place: "aside" | "tab" | "main", mod?: string }>}
 */
export const PANELS = {
  desk: { place: "aside", mod: "priory-desk" },
  asks: { place: "aside", mod: "priory-asks" },
  queue: { place: "aside", mod: "priory-queue" },
  reports: { place: "tab", mod: "priory-bonjour" },
  workflow: { place: "tab", mod: "priory-flows" },
  costs: { place: "tab", mod: "priory-costs" },
  tasks: { place: "tab", mod: "priory-tasks" },
  runs: { place: "tab", mod: "priory-runs" },
  inbox: { place: "tab", mod: "priory-inbox" },
  terminal: { place: "main" },
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

/** The view's panels in tree order, once its nodes are known to be Priory's. @param {string} name @param {any} node @returns {string[]} */
function panels(name, node) {
  const [key, value] = node && typeof node === "object" && !Array.isArray(node) && Object.keys(node).length === 1 ? Object.entries(node)[0] : [];
  if (key === "panel") {
    if (typeof value !== "string" || !Object.hasOwn(PANELS, value)) throw new Error(`view ${name} has a panel Priory does not ship: ${value}`);
    return [value];
  }
  if (!["row", "column", "tabs"].includes(key) || !Array.isArray(value) || !value.length) {
    throw new Error(`view ${name} has a node that is not a row, column, tabs or panel`);
  }
  return value.flatMap((child) => panels(name, child));
}

/**
 * The frame with its `work-body` laid out as the view's panels, in the view's order: a stack in the
 * center of `main` and each wide panel, an aside for each narrow one, and Herdr's own Files and
 * Changes panel last. The view's rows, columns and tabs only order the panels. The first aside
 * starts shown and the others hidden, until Herdr remembers a click.
 * @param {string} name @param {{ layout: any }} view @param {any} frame
 */
export function layout(name, view, frame) {
  const names = panels(name, view.layout);
  if (names.filter((panel) => panel === "terminal").length > 1) {
    throw new Error(`view ${name} has two terminal panels, and Herdr has one main block`);
  }
  const twice = names.find((panel, at) => names.indexOf(panel) !== at);
  if (twice) throw new Error(`view ${name} has the ${twice} panel twice`);
  const root = structuredClone(frame);
  const body = pathTo(root, "work-body")?.at(-1);
  const main = pathTo(root, "main")?.at(-1);
  const herdrPanel = pathTo(root, "panel")?.at(-1);
  if (!body || !main || !herdrPanel) throw new Error("the frame needs a work-body block holding a main block and Herdr's panel");
  const block = (/** @type {string} */ panel) => ({ id: panel, type: "block", content: { mod: PANELS[panel].mod, props: { panel } } });
  const tabs = names.filter((panel) => PANELS[panel].place === "tab").map(block);
  const asides = names.filter((panel) => PANELS[panel].place === "aside").map((panel, at) => (
    { id: `aside-${panel}`, type: "column", size: { px: 360 }, min: 280, max: 700, ...(at ? { hidden: true } : {}), children: [block(panel)] }
  ));
  const center = tabs.length ? { id: "center", type: "stack", active: main.id, children: [main, ...tabs] } : main;
  body.children = [center, ...asides, herdrPanel];
  return root;
}

/**
 * What the Priory bar shows: a button for each aside, then one for each tab in the center. An aside
 * button toggles its `id`, and a tab button selects its block.
 * @param {any} root @returns {{ panel: string, id: string, aside: boolean }[]}
 */
export function buttons(root) {
  const children = pathTo(root, "work-body")?.at(-1)?.children ?? [];
  const asides = children.filter((/** @type {any} */ child) => child.id.startsWith("aside-")).map((/** @type {any} */ child) => ({ panel: child.children[0].content.props.panel, id: child.id, aside: true }));
  const tabs = (children.find((/** @type {any} */ child) => child.id === "center")?.children ?? []).filter((/** @type {any} */ child) => child.content?.props?.panel).map((/** @type {any} */ child) => ({ panel: child.content.props.panel, id: child.id, aside: false }));
  return [...asides, ...tabs];
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

/** What `install` and the Priory bar own: the root id of a layout this adapter wrote, with or without the views' hash. */
export const OWN_ID = /^priory-generated(-[0-9a-f]{12})?$/;

/** Herdr's layout file for a view: `default` keeps `priory.json`, so its layout name and `keys.json` bindings stay. @param {string} name */
export const layoutFile = (name) => (name === "default" ? "priory.json" : `priory-${name}.json`);

/**
 * Every view's layout by file name, each with the root id `priory-generated-HASH`.
 * @param {Record<string, { layout: any }>} views what `priory views --json` prints as `views`
 * @param {any} frame @param {string} hash 12 hex digits of the SHA-256 of that output, trimmed
 */
export function layouts(views, frame, hash) {
  if (!views?.default) throw new Error("the views have no default view");
  return Object.fromEntries(Object.entries(views).map(([name, view]) => [layoutFile(name), { ...layout(name, view, frame), id: `priory-generated-${hash}` }]));
}
