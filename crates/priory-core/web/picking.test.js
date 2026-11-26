import { test } from "node:test";
import assert from "node:assert/strict";
import { UNDO_MS, picking } from "./picking.js";

/** Three rows picked one after another, a second apart, within each other's Undo. */
function three(t, between = () => {}) {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const sent = [];
  const shown = [];
  const picks = picking({ send: async (pick) => sent.push(pick.option), changed: (rows) => shown.push([...rows.keys()]) });
  for (const [row, option] of [["a", "In"], ["b", "Out"], ["c", "Later"]]) {
    assert.equal(picks.start({ row, option }), true, row);
    t.mock.timers.tick(1000);
  }
  between(picks, shown);
  t.mock.timers.tick(UNDO_MS);
  return { sent, shown };
}

test("three picks within the Undo window each wait on their own and are all sent", (t) => {
  const { sent } = three(t, (_picks, shown) => assert.deepEqual(shown.at(-1), ["a", "b", "c"]));
  assert.deepEqual(sent, ["In", "Out", "Later"]);
});

test("undo of the second of three picks leaves the first and the third to go", (t) => {
  const { sent } = three(t, (picks) => assert.equal(picks.undo("b"), true));
  assert.deepEqual(sent, ["In", "Later"]);
});

test("a row takes no second pick while its first is pending", (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const picks = picking({ send: async () => {}, changed() {} });
  assert.equal(picks.start({ row: "a", option: "In" }), true);
  assert.equal(picks.start({ row: "a", option: "Out" }), false);
});

test("cancel drops only the waiting picks that fail its test", (t) => {
  const sent = [];
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const picks = picking({ send: async (pick) => sent.push(pick.row), changed() {} });
  picks.start({ row: "a", option: "In" });
  picks.start({ row: "b", option: "In" });
  assert.deepEqual(picks.cancel((pick) => pick.row === "a").map((pick) => pick.row), ["b"]);
  t.mock.timers.tick(UNDO_MS);
  assert.deepEqual(sent, ["a"]);
});

test("a batch waits out one Undo, sends every pick in it, and skips rows already pending", (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const sent = [];
  const picks = picking({ send: async (pick) => sent.push(pick.row), changed() {} });
  picks.start({ row: "a", option: "In" });
  assert.equal(picks.batch("sentry", [{ row: "a", option: "Out" }, { row: "b", option: "Out" }, { row: "c", option: "Out" }]), 2);
  t.mock.timers.tick(UNDO_MS);
  assert.deepEqual(sent, ["a", "b", "c"]);
});

test("undo of one row of a batch undoes the whole batch and leaves a single pick", (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const sent = [];
  const picks = picking({ send: async (pick) => sent.push(pick.row), changed() {} });
  picks.start({ row: "a", option: "In" });
  picks.batch("sentry", [{ row: "b", option: "Out" }, { row: "c", option: "Out" }]);
  assert.equal(picks.undo("c"), true);
  t.mock.timers.tick(UNDO_MS);
  assert.deepEqual(sent, ["a"]);
});
