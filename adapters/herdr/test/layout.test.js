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

test("the default view gives a row of main and one stack of its eight panels, Desk first and active", () => {
  const body = find(layout("default", DEFAULT, FRAME), "work-body");

  assert.deepEqual(body.children.map((child) => child.id), ["main", "view", "panel"]);
  assert.deepEqual(body.children[0], find(FRAME, "main"));
  assert.deepEqual(body.children[1], {
    id: "view",
    type: "stack",
    active: "view-0",
    size: { px: 420 },
    min: 280,
    max: 900,
    children: [
      block("view-0", "priory-desk", "desk"),
      block("view-1", "priory-asks", "asks"),
      block("view-2", "priory-elsewhere", "tasks"),
      block("view-3", "priory-elsewhere", "runs"),
      block("view-4", "priory-elsewhere", "inbox"),
      block("view-5", "priory-bonjour", "reports"),
      block("view-6", "priory-flows", "workflow"),
      block("view-7", "priory-costs", "costs"),
    ],
  });
});

test("a tabs whose child is a row gives a stack whose child is that row, with equal weights", () => {
  const view = { layout: { tabs: [{ row: [{ panel: "desk" }, { tabs: [{ panel: "queue" }, { panel: "runs" }] }] }, { panel: "costs" }] } };

  const stack = find(layout("night", view, FRAME), "view");

  assert.equal(stack.type, "stack");
  assert.equal(stack.active, "view-0");
  assert.deepEqual(stack.children, [
    {
      id: "view-0",
      type: "row",
      children: [
        { ...block("view-0-0", "priory-desk", "desk"), size: { weight: 1 } },
        {
          id: "view-0-1",
          type: "stack",
          active: "view-0-1-0",
          size: { weight: 1 },
          children: [block("view-0-1-0", "priory-queue", "queue"), block("view-0-1-1", "priory-elsewhere", "runs")],
        },
      ],
    },
    block("view-1", "priory-costs", "costs"),
  ]);
});

test("the example in views.md gives a row of the Desk and a column of the Queue over the Runs", () => {
  const view = { layout: { row: [{ panel: "desk" }, { column: [{ panel: "queue" }, { panel: "runs" }] }] } };

  const body = find(layout("default", view, FRAME), "work-body");

  assert.deepEqual(body.children.map((child) => child.id), ["main", "view", "panel"]);
  assert.deepEqual(body.children[1], {
    id: "view",
    type: "row",
    size: { px: 420 },
    min: 280,
    max: 900,
    children: [
      { ...block("view-0", "priory-desk", "desk"), size: { weight: 1 } },
      {
        id: "view-1",
        type: "column",
        size: { weight: 1 },
        children: [
          { ...block("view-1-0", "priory-queue", "queue"), size: { weight: 1 } },
          { ...block("view-1-1", "priory-elsewhere", "runs"), size: { weight: 1 } },
        ],
      },
    ],
  });
});

test("a terminal panel takes the main block's place, and the view fills the work body", () => {
  const view = { layout: { row: [{ panel: "terminal" }, { panel: "asks" }] } };

  const body = find(layout("default", view, FRAME), "work-body");

  assert.deepEqual(body.children.slice(0, -1), [
    {
      id: "view",
      type: "row",
      children: [{ ...find(FRAME, "main"), size: { weight: 1 } }, { ...block("view-1", "priory-asks", "asks"), size: { weight: 1 } }],
    },
  ]);
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
    [{ tabs: [{ panel: "kanban" }] }, "view night has a panel Priory does not ship: kanban"],
  ];
  for (const [tree, message] of refused) assert.throws(() => layout("night", { layout: tree }, FRAME), { message }, JSON.stringify(tree));
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

test("Herdr's Files and Changes panels stay in their own hidden stack after the view", () => {
  const views = [DEFAULT, { layout: { panel: "terminal" } }];
  for (const view of views) {
    const body = find(layout("default", view, FRAME), "work-body");
    assert.deepEqual(body.children.at(-1), HERDR_PANEL, JSON.stringify(view));
    assert.equal(body.children.filter((child) => child.id === "panel").length, 1);
  }
});
