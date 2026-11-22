import { test } from "node:test";
import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import { all, nodeOf } from "./kit.js";
import { palette } from "../lib/tokens.js";

// ui.js imports gpui-kit, which kit.js maps only once it has run.
const { stateTile } = await import("../lib/ui.js");

const t = palette("graphite", "light");

test("a state tile is a stroke icon in its role's text color, with no fill", () => {
  const tile = nodeOf(stateTile("blocked", "icons/hand.svg", "tile-warn", t));
  assert.equal(tile.calls.bg, undefined);
  assert.equal(tile.calls.accessibility_label[0], "blocked");
  const [icon] = all(tile, "svg");
  assert.deepEqual([icon.path, icon.calls.size[0], icon.calls.text_color[0]], ["icons/hand.svg", 17, t["warn-text"]]);
  // An ask takes the amber of what waits on a person; a small tile a 15 px icon.
  const ask = all(stateTile("ask", "icons/ask.svg", "wait", t, "small"), "svg")[0];
  assert.deepEqual([ask.calls.size[0], ask.calls.text_color[0]], [15, "#A84D06"]);
});

test("a running tile holds a live dot in the live role's text color", () => {
  const tile = nodeOf(stateTile("running", null, "tile-live", t));
  assert.equal(tile.calls.bg, undefined);
  assert.equal(nodeOf(tile.kids[0]).calls.bg[0], t["action-text"]);
});

// DESIGN.md, State tile: no Herdr panel draws a filled, rounded tile any more.
test("every panel draws its state tiles with stateTile", () => {
  const mods = new URL("../mods/", import.meta.url);
  for (const mod of readdirSync(mods).filter((name) => name.startsWith("priory-"))) {
    const source = readFileSync(new URL(`${mod}/main.js`, mods), "utf8");
    assert.equal(source.match(/radius-tile|RADIUS\.tile/g), null, `${mod} fills a tile: draw it with stateTile`);
  }
});

// DESIGN.md, Radii: a full radius only for dots, never for a button, a chip or a bar.
test("no panel draws a pill", () => {
  const mods = new URL("../mods/", import.meta.url);
  for (const mod of readdirSync(mods).filter((name) => name.startsWith("priory-") && name !== "priory-tabs")) {
    const pills = readFileSync(new URL(`${mod}/main.js`, mods), "utf8").split("\n")
      .filter((line) => /rounded\((999|S\["radius-full"\])\)/.test(line) && !/\.size\(\d+\)[^)]*\.rounded/.test(line));
    assert.deepEqual(pills, [], `${mod} draws a pill`);
  }
});
