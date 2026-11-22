import { test } from "node:test";
import assert from "node:assert/strict";
import { context, draw, find, herdr, press, settle, text } from "./kit.js";

process.env.TZ = "UTC";

const { default: Usage } = await import("../mods/priory-usage/main.js");

// What `priory costs --json` prints on Tuesday 2026-11-17 at 12:00 in Paris, its week from Monday.
const COSTS = {
  by: "workflow",
  current: { since: "2026-11-15T23:00:00Z", since_day: "2026-11-16", until: "2026-11-17T11:00:00Z", cost_usd: 171.2 },
  days: [{ day: "2026-11-16", cost_usd: 158.8 }, { day: "2026-11-17", cost_usd: 12.4 }],
  split: [{ workflow: "implement", cost_usd: 128.9 }, { workflow: "review", cost_usd: 31.05 }, { workflow: "plan-batch", cost_usd: null }],
  caps: [
    { scope: "factory", period: "day", cap_usd: 40, spent_usd: 12.4, task: null },
    { scope: "factory", period: "week", cap_usd: 200, spent_usd: 171.2, task: null },
    { scope: "lane core/intake", period: "day", cap_usd: 10, spent_usd: 10.6, task: null },
    { scope: "factory", period: "task", cap_usd: 5, spent_usd: 4.1, task: 41 },
  ],
  held: [
    { task: 38, key: "PRI-38", scope: "factory", period: "task", cap_usd: 5, spent_usd: 5.2 },
    { task: 44, key: "PRI-44", scope: "lane core/intake", period: "task", cap_usd: 10, spent_usd: 10.6 },
  ],
};
const CLAUDE = "/home/me/.cache/over9k/usage/claude.json";

/** Draws the bar, or its detail with `props`, after its first read, at `width`. */
async function drawn({ costs = COSTS, files = {}, width = 1280, props = {} } = {}) {
  herdr.reset();
  herdr.replies = { costs };
  herdr.files = files;
  herdr.props = props;
  const view = new Usage();
  const cx = context();
  view.init(props, cx);
  await settle();
  view.width = width;
  return { view, cx, tree: draw(view, cx) };
}

test("the bar says today's and this week's spend against the factory's caps, the held tasks and Claude's week", async () => {
  const claude = JSON.stringify({ seven_day: { used_percentage: 43, resets_at: Date.now() / 1000 + 3600 } });
  const { tree } = await drawn({ files: { [CLAUDE]: claude } });
  assert.equal(text(find(tree, "usage-line")), "Priory $12.40 today of $40 · $171.20 this week of $200 · 2 held at a cap · Claude 7d 43%");
});

test("at 320 px the line keeps whole numbers, leaves Claude to the detail, and warns of a cap at 80% or more", async () => {
  const claude = JSON.stringify({ seven_day: { used_percentage: 91 } });
  const { view, cx } = await drawn({ files: { [CLAUDE]: claude } });
  view.width = 320;
  const tree = draw(view, cx);
  assert.equal(text(find(tree, "usage-line")), "Priory $12 of $40 today · $171 of $200 wk · 2 held");
  const warn = "#8A5A00";
  assert.equal(find(tree, "usage-week").calls.text_color[0], warn);
  assert.notEqual(find(tree, "usage-today").calls.text_color[0], warn);
  view.width = 1280;
  assert.equal(find(draw(view, cx), "usage-claude").calls.text_color[0], warn);
});

test("a click on the line opens the detail as a popup of this mod", async () => {
  const { tree } = await drawn();
  press(tree, "usage-line");
  assert.equal(herdr.overlays.length, 1);
  assert.equal(herdr.overlays[0].mod, "priory-usage");
  assert.equal(herdr.overlays[0].kind, "popup");
});

test("the detail shows each cap with a bar, this week by workflow, the held tasks and Claude's week", async () => {
  const claude = JSON.stringify({ seven_day: { used_percentage: 43, resets_at: Date.now() / 1000 + 3 * 86400 + 4 * 3600 + 60 } });
  const { tree } = await drawn({ files: { [CLAUDE]: claude }, props: { overlay: "overlay-1" } });
  assert.equal(find(tree, "usage-line"), null);
  assert.equal(text(find(tree, "usage-caps")), [
    "Spend caps",
    "Factory, day $12.40 of $40.00",
    "Factory, week $171.20 of $200.00",
    "Lane core/intake, day $10.60 of $10.00 Reached: new runs wait for the next day",
    "Factory, task task 41 $4.10 of $5.00 The active task under it that spent most",
  ].join(" "));
  assert.equal(find(tree, "cap-fill-1").calls.w[0], "86%");
  assert.equal(find(tree, "cap-fill-2").calls.w[0], "100%");
  assert.equal(text(find(tree, "usage-workflows")), "This week by workflow $171.20 implement $128.90 review $31.05 plan-batch -");
  assert.equal(text(find(tree, "usage-held")), [
    "Held at a cap 2",
    "PRI-38 spent $5.20 of the factory's task cap of $5.00",
    "PRI-44 spent $10.60 of lane core/intake's task cap of $10.00",
  ].join(" "));
  assert.equal(text(find(tree, "usage-claude")), "Claude subscription 7-day limit 43%, resets in 3d 4h");
});

test("a held task opens in the Task panel and closes the popup", async () => {
  const { tree } = await drawn({ props: { overlay: "overlay-1" } });
  herdr.overlays = [{ mod: "priory-usage" }];
  press(tree, "held-44");
  assert.equal(herdr.shared.get("priory.task.open").task, 44);
  assert.equal(herdr.overlays.length, 0);
});

test("when whole parts cannot fit, the line leaves the last ones out rather than cut a number", async () => {
  const caps = [
    { scope: "factory", period: "day", cap_usd: 2000, spent_usd: 1234.5, task: null },
    { scope: "factory", period: "week", cap_usd: 20000, spent_usd: 12345, task: null },
  ];
  const held = Array.from({ length: 12 }, (_, at) => ({ task: at, key: `PRI-${at}`, scope: "factory", period: "task", cap_usd: 5, spent_usd: 6 }));
  const { view, cx } = await drawn({ costs: { ...COSTS, caps, held } });
  view.width = 320;
  assert.equal(text(find(draw(view, cx), "usage-line")), "Priory $1235 of $2000 today · $12345 of $20000 wk");
});

test("after a failed read the line says so within its width, and the detail warns over the last read", async () => {
  const { view, cx } = await drawn();
  herdr.replies = {};
  await view.read();
  view.width = 320;
  assert.equal(text(find(draw(view, cx), "usage-line")), "Priory Could not read costs $12 of $40 today");
  view.overlay = "overlay-1";
  assert.match(text(find(draw(view, cx), "usage-stale")), /^Could not read costs: priory costs: no such thing\. This is the last read\.$/);
});

test("an unpriced day reads -, never $0, and an idle task cap keeps its limit", async () => {
  const days = [{ day: "2026-11-17", cost_usd: null }];
  const { tree } = await drawn({ costs: { ...COSTS, days, caps: [{ scope: "factory", period: "task", cap_usd: 5, spent_usd: null, task: null }] } });
  assert.match(text(find(tree, "usage-line")), /^Priory - today · /);
  const detail = (await drawn({ costs: { ...COSTS, caps: [{ scope: "factory", period: "task", cap_usd: 5, spent_usd: null, task: null }] }, props: { overlay: "overlay-1" } })).tree;
  assert.equal(text(find(detail, "usage-caps")), "Spend caps Factory, task no active task, cap $5.00");
});

test("with no cap and no Over9k file, the line shows the spend alone, today's from the days", async () => {
  const { tree } = await drawn({ costs: { ...COSTS, caps: [], held: [] } });
  assert.equal(text(find(tree, "usage-line")), "Priory $12.40 today · $171.20 this week");
});
