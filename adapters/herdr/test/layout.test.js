import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { layout } from "../lib/layout.js";

const FRAME = JSON.parse(readFileSync(new URL("../layouts/frame.json", import.meta.url), "utf8"));

// The view Priory ships as `default`, as `priory views --json` prints it.
const DEFAULT = {
  layout: {
    tabs: ["desk", "asks", "tasks", "runs", "inbox", "reports", "workflow", "costs"].map((panel) => ({ panel })),
  },
};

/** @param {any} node @param {string} id @returns {any} */
function find(node, id) {
  if (node?.id === id) return node;
  for (const child of node?.children ?? []) {
    const found = find(child, id);
    if (found) return found;
  }
  return null;
}

const block = (id, mod, panel) => ({ id, type: "block", content: { mod, props: { panel } } });
const aside = (panel, mod, hidden) => ({
  id: `aside-${panel}`,
  type: "column",
  size: { px: 360 },
  min: 280,
  max: 700,
  ...(hidden ? { hidden: true } : {}),
  children: [block(panel, mod, panel)],
});

test("the default view gives narrow panels an aside each and wide ones a tab beside main", () => {
  const body = find(layout("default", DEFAULT, FRAME), "work-body");

  assert.deepEqual(body.children.map((child) => child.id), ["center", "aside-desk", "aside-asks", "panel"]);
  assert.deepEqual(body.children[0], {
    id: "center",
    type: "stack",
    active: "main",
    children: [
      find(FRAME, "main"),
      block("tasks", "priory-elsewhere", "tasks"),
      block("runs", "priory-elsewhere", "runs"),
      block("inbox", "priory-inbox", "inbox"),
      block("reports", "priory-bonjour", "reports"),
      block("workflow", "priory-flows", "workflow"),
      block("costs", "priory-costs", "costs"),
    ],
  });
  assert.deepEqual(body.children[1], aside("desk", "priory-desk", false));
  assert.deepEqual(body.children[2], aside("asks", "priory-asks", true));
});

test("every narrow panel is its own aside, so several can be open side by side", () => {
  const view = { layout: { row: [{ panel: "queue" }, { column: [{ panel: "desk" }, { panel: "asks" }] }] } };

  const body = find(layout("default", view, FRAME), "work-body");

  assert.deepEqual(body.children.map((child) => child.id), ["main", "aside-queue", "aside-desk", "aside-asks", "panel"]);
  assert.deepEqual(body.children.slice(1, 4), [aside("queue", "priory-queue", false), aside("desk", "priory-desk", true), aside("asks", "priory-asks", true)]);
});

test("the view's rows, columns and tabs only order the panels", () => {
  const flat = { layout: { tabs: [{ panel: "desk" }, { panel: "costs" }, { panel: "queue" }] } };
  const nested = { layout: { row: [{ panel: "desk" }, { column: [{ panel: "costs" }, { tabs: [{ panel: "queue" }] }] }] } };

  assert.deepEqual(layout("default", nested, FRAME), layout("default", flat, FRAME));
});

test("a terminal panel changes nothing, since main always stays in the center", () => {
  const without = { layout: { tabs: [{ panel: "desk" }, { panel: "costs" }] } };
  const withTerminal = { layout: { tabs: [{ panel: "terminal" }, { panel: "desk" }, { panel: "costs" }] } };

  assert.deepEqual(layout("default", withTerminal, FRAME), layout("default", without, FRAME));
});

test("a view with only terminal panels leaves main alone beside Herdr's panel", () => {
  const body = find(layout("default", { layout: { panel: "terminal" } }, FRAME), "work-body");

  assert.deepEqual(body.children.map((child) => child.id), ["main", "panel"]);
});

test("a panel placed twice is refused with its view's name", () => {
  const view = { layout: { row: [{ panel: "desk" }, { tabs: [{ panel: "desk" }] }] } };

  assert.throws(() => layout("night", view, FRAME), { message: "view night has the desk panel twice" });
});

// Herdr's own panel, as Herdr's shipped `main` layout has it: hidden until the tab bar opens it.
const HERDR_PANEL = {
  id: "panel",
  type: "column",
  hidden: true,
  size: { px: 368 },
  min: 280,
  max: 900,
  children: [
    {
      id: "panel-body",
      type: "stack",
      active: "files",
      children: [
        { id: "files", type: "block", content: { mod: "files" } },
        { id: "changes", type: "block", content: { mod: "changes" } },
      ],
    },
  ],
};

test("Herdr's Files and Changes panels stay in their own hidden stack after the asides", () => {
  for (const view of [DEFAULT, { layout: { panel: "terminal" } }]) {
    const body = find(layout("default", view, FRAME), "work-body");
    assert.deepEqual(body.children.at(-1), HERDR_PANEL, JSON.stringify(view));
    assert.equal(body.children.filter((child) => child.id === "panel").length, 1);
  }
});

test("every layout keeps the frame's chrome, and a second run gives the same layout", () => {
  for (const view of [DEFAULT, { layout: { panel: "terminal" } }, { layout: { column: [{ panel: "desk" }, { panel: "inbox" }] } }]) {
    const generated = layout("default", view, FRAME);
    for (const id of ["sidebar", "priory", "tabs", "main", "status"]) assert.ok(find(generated, id), `${id} in ${JSON.stringify(view)}`);
    assert.deepEqual(layout("default", view, FRAME), generated);
  }
});

test("a view with two terminal panels is refused with its name", () => {
  const view = { layout: { row: [{ panel: "terminal" }, { tabs: [{ panel: "terminal" }] }] } };

  assert.throws(() => layout("night", view, FRAME), { message: "view night has two terminal panels, and Herdr has one main block" });
});

test("a node or panel Priory does not ship is refused with the view's name", () => {
  const refused = [
    [{ grid: [{ panel: "desk" }] }, "view night has a node that is not a row, column, tabs or panel"],
    [{ row: [] }, "view night has a node that is not a row, column, tabs or panel"],
    [{ panel: "__proto__" }, "view night has a panel Priory does not ship: __proto__"],
    [{ panel: ["desk"] }, "view night has a panel Priory does not ship: desk"],
    [{ tabs: [{ panel: ["costs"] }] }, "view night has a panel Priory does not ship: costs"],
    [{ tabs: [{ panel: "kanban" }] }, "view night has a panel Priory does not ship: kanban"],
  ];
  for (const [tree, message] of refused) assert.throws(() => layout("night", { layout: tree }, FRAME), { message }, JSON.stringify(tree));
});
