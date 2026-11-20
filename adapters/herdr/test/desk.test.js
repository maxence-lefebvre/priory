import { test } from "node:test";
import assert from "node:assert/strict";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { deskModel, mergeSince } from "../mods/priory-desk/view.js";

process.env.TZ = "UTC";

// Captured from `priory desk --all --json`, `asks --all --json`, `tasks --json` and `queue DIR --json`
// on a scratch state file seeded by SQL, with PRIORY_NOW=2026-11-20T12:00:00Z.
const fixture = (/** @type {string} */ name) => JSON.parse(readFileSync(new URL(`./fixtures/${name}.json`, import.meta.url), "utf8"));
const data = { desk: fixture("desk"), asks: fixture("desk-asks"), tasks: fixture("desk-tasks"), queue: fixture("desk-queue") };
const now = Date.parse("2026-11-20T12:00:00Z");

test("counts the open asks that are not snoozed, the oldest wait, the stopped and the snoozed", () => {
  assert.deepEqual(deskModel(data, { now }).count, {
    number: 3,
    label: "asks need you",
    line: "Oldest waits 42 min. 1 stopped. 2 snoozed.",
  });
});

test("the next thing is the oldest open ask, with its recommendation and its task's progress", () => {
  assert.deepEqual(deskModel(data, { now }).next, {
    ask: 1,
    revision: 1,
    task: 1,
    key: "cli:PRI-61",
    step: "approve",
    age: "42 min",
    question: "Merge #212 into main? Checks are green at a1f3c9e.",
    recommendation: "merge",
    tradeoff: "The diff stays inside priory-core and the docs.",
    progress: { latestChange: "Checks are green at a1f3c9e", nextAction: "Squash-merge #212 once a person says Merge" },
  });
});

test("an empty desk has no next thing and says what appears here", () => {
  const model = deskModel({ desk: fixture("desk-empty"), asks: { asks: [] }, tasks: { tasks: [] }, queue: { up_next: [] } }, { now });
  assert.equal(model.next, null);
  assert.deepEqual(model.count, { number: 0, label: "asks need you", line: "" });
  assert.equal(model.empty, "No open asks. Agents ask here when they need you.");
});

test("Needs me lists the asks oldest first, then the stopped tasks, then what needs no action", () => {
  const model = deskModel(data, { now });
  assert.deepEqual(model.views.map(({ id, label, count }) => [id, label, count]), [
    ["needs", "Needs me", 4], ["running", "Running", 1], ["asleep", "Asleep", 4], ["snoozed", "Snoozed", 2], ["all", "All tasks", 9],
  ]);
  const sections = model.sections("needs");
  assert.deepEqual(sections.map(({ title, note }) => [title, note]), [["Asks", "3, oldest first"], ["Stopped", "1"], ["Later", "no action needed"]]);
  assert.deepEqual(sections[0].rows.map((row) => [row.kind, row.key, row.age, row.text, row.recommends, row.note]), [
    ["ask", "cli:PRI-61", "42 min", "Merge #212 into main? Checks are green at a1f3c9e.", "merge", null],
    ["ask", "rayon-desk", "20 min", "Ship the <b>release</b> notes?", "Ship", null],
    ["ask", "cli:PRI-62", "6 min", "Use the backoff crate for retries, or keep the 20-line loop?", "Use backoff", "Back early: new revision 2"],
  ]);
  assert.deepEqual(sections[1].rows.map((row) => [row.kind, row.key, row.text, row.recommends, row.snooze]), [
    ["blocked", "cli:PRI-55", "Permission prompt denied: write to ~/.ssh/config, outside the worktree.", "Resume", { kind: "blocked", id: 4 }],
  ]);
  assert.deepEqual(sections[2].rows.map((row) => [row.kind, row.text]), [
    ["asleep", "4 tasks asleep on waits"],
    ["report", "ci: 1 report to read: merges this week"],
    ["held", "held: waits for the release on Monday"],
  ]);
});

test("the room banner, Since you looked and Up next come from the desk and the queue", () => {
  const model = deskModel(data, { now });
  assert.deepEqual(model.banners, ["stalled since 2026-11-20T11:02:00Z: short of disk by 4.1 GB; task 7 waits for room"]);
  assert.equal(model.since.from, "09:12");
  assert.deepEqual(model.since.rows.slice(0, 2), [
    { key: "cli:PRI-66", task: 6, what: "factory.run.started triage", time: "11:02" },
    { key: "cli:PRI-63", task: 3, what: "factory.step.ended approve: merged #210", time: "10:41" },
  ]);
  assert.deepEqual(model.upNext, [
    { when: "now", key: "cli:PRI-72", task: 7, why: "first in the queue" },
    { when: "Nov 21, 02:00", key: "schedule:nightly", task: null, why: "schedule nightly fires" },
  ]);
});

test("an empty desk says nothing is queued or scheduled to run next", () => {
  const model = deskModel({ desk: fixture("desk-empty"), asks: { asks: [] }, tasks: { tasks: [] }, queue: { up_next: [] } }, { now });
  assert.deepEqual(model.banners, []);
  assert.equal(model.since.from, null);
  assert.deepEqual(model.upNext, []);
  assert.equal(model.upNextEmpty, "Nothing is queued or scheduled to run next.");
});

test("Snoozed lists each snoozed item with when it comes back, and Running shows each task's progress", () => {
  const model = deskModel(data, { now });
  assert.deepEqual(model.sections("snoozed")[0].rows.map((row) => [row.kind, row.key, row.note, row.snooze, row.snoozed]), [
    ["failed", "cli:PRI-70", "Snoozed until Nov 21, 09:00", { kind: "failed", id: 5 }, true],
    ["ask", "cli:PRI-63", "Snoozed until Nov 21, 12:05", { kind: "ask", id: 3 }, true],
  ]);
  assert.equal(model.sections("needs")[0].rows[2].snoozed, false);
  assert.deepEqual(model.sections("running")[0].rows.map((row) => [row.key, row.text, row.progress]), [
    ["cli:PRI-66", "Retry with backoff", ["Goal: Retry failed fetches", "Latest: Added a backoff test", "Next: Wire the crate into the fetcher"]],
  ]);
});

// gpui-kit has no light-dark(), so each mod carries the theme pairs resolved; this keeps them in step.
test("every mod's tokens.js matches design/tokens/themes.css", async () => {
  const css = readFileSync(new URL("../../../design/tokens/themes.css", import.meta.url), "utf8");
  /** @type {Record<string, Record<string, string[]>>} */
  const themes = {};
  for (const [, selector, body] of css.matchAll(/^([^\n{]*\[data-theme="?[a-z]+"?\][^\n{]*)\{([^}]*)\}/gm)) {
    const name = /** @type {string} */ (selector.match(/data-theme="([a-z]+)"/)?.[1]);
    themes[name] = Object.fromEntries([...body.matchAll(/--([a-z0-9-]+):\s*light-dark\((.+?),\s*((?:rgb\([^)]*\))|[^)]+)\);/g)]
      .map(([, token, light, dark]) => [token, [light.trim(), dark.trim()]]));
  }
  assert.deepEqual(Object.keys(themes), ["indigo", "ink", "parchment", "abbey"]);
  const mods = new URL("../mods/", import.meta.url);
  for (const mod of readdirSync(mods)) {
    if (!existsSync(new URL(`${mod}/tokens.js`, mods))) continue;
    const { THEMES } = await import(new URL(`${mod}/tokens.js`, mods).href);
    assert.deepEqual(THEMES, themes, `${mod}/tokens.js differs from design/tokens/themes.css`);
  }
  assert.ok(existsSync(new URL("priory-desk/tokens.js", mods)));
});

test("the Desk's structure tokens match design/tokens/structure.css", async () => {
  const css = readFileSync(new URL("../../../design/tokens/structure.css", import.meta.url), "utf8");
  const { STRUCTURE } = await import("../mods/priory-desk/tokens.js");
  for (const [token, value] of Object.entries(STRUCTURE)) {
    const declared = css.match(new RegExp(`--${token}:\\s*([^;]+);`))?.[1];
    const expected = Array.isArray(value) ? `${value[0]}px/${value[1] === 1 ? "1" : `${value[1]}px`}` : `${value}px`;
    assert.equal(declared, expected, `--${token}`);
  }
});

test("Since you looked keeps the events the panel opened on after its own priory seen", () => {
  const opened = fixture("desk").since_you_looked;
  const later = [{ time: "2026-11-20T12:10:00Z", task: 1, key: "cli:PRI-61", what: "factory.step.ended approve: merge" }];
  const kept = mergeSince(mergeSince([], opened), later);
  assert.equal(kept.length, opened.length + 1);
  assert.deepEqual(kept[0], later[0]);
  assert.deepEqual(mergeSince(kept, later), kept);
});
