import { test } from "node:test";
import assert from "node:assert/strict";
import { editBody, laneLayout, momentWords, stages, successors } from "./batches.js";

const entry = (id, ...after) => ({ entry: id, after: after.map(([other, moment = "merged"]) => ({ entry: other, moment })) });
// Board 2's batch: three roots, then entries that wait on them.
const BOARD = [
  entry("e1"), entry("e2"), entry("e3"),
  entry("e4", ["e1", "waiting"]), entry("e5", ["e2", "waiting"]),
  entry("e6", ["e4"]), entry("e7", ["e4"]), entry("e8", ["e6"], ["e7"]),
];

test("an entry continues the lane of the predecessor it is the last to wait on, else takes the lowest free lane", () => {
  const layout = laneLayout(BOARD);
  assert.deepEqual(layout.rows.map((row) => row.lane), [0, 1, 2, 0, 1, 1, 0, 1]);
  assert.equal(layout.lanes, 3);
  assert.deepEqual(layout.edges, [
    { from: 0, to: 3, moment: "waiting" }, { from: 1, to: 4, moment: "waiting" },
    { from: 3, to: 5, moment: "merged" }, { from: 3, to: 6, moment: "merged" },
    { from: 5, to: 7, moment: "merged" }, { from: 6, to: 7, moment: "merged" },
  ]);
});

test("on a phone, entries past the third lane share the last one", () => {
  const roots = ["e1", "e2", "e3", "e4", "e5"].map((id) => entry(id));
  const waits = [entry("e6", ["e1"], ["e2"], ["e3"], ["e4"], ["e5"])];
  const layout = laneLayout([...roots, ...waits], 3);
  assert.deepEqual(layout.rows.map((row) => row.lane), [0, 1, 2, 2, 2, 0]);
  assert.equal(layout.lanes, 3);
});

test("the stage bar shows each stage with entries, in the batch's order", () => {
  const counts = { planned: 3, queued: 0, blocked: 0, started: 3, waiting: 1, merged: 1, failed: 0, dropped: 0 };
  assert.deepEqual(stages(counts), [["merged", 1], ["started", 3], ["waiting", 1], ["planned", 3]]);
});

test("each row says its moments in words, so no reader depends on the line style", () => {
  const plan = [{ ...entry("e1"), issue: "ENG-40", part: null }, { ...entry("e2", ["e1", "waiting"]), issue: "ENG-41", part: 1 },
    { ...entry("e3", ["e1"], ["e2"]), issue: "ENG-41", part: 2 }];
  assert.equal(momentWords(plan[0], plan), "starts at the go");
  assert.equal(momentWords(plan[1], plan), "stacks on #1 ENG-40");
  assert.equal(momentWords(plan[2], plan), "starts once #1 ENG-40 and #2 ENG-41.1 merge");
});

test("the edit menu moves an entry past its neighbour and drops it with every entry after it", () => {
  assert.deepEqual(editBody(2, BOARD, "e3", "up"), { revision: 2, move: "e3", before: "e2" });
  assert.deepEqual(editBody(2, BOARD, "e3", "down"), { revision: 2, move: "e3", after: "e4" });
  assert.equal(editBody(2, BOARD, "e1", "up"), null);
  assert.equal(editBody(2, BOARD, "e8", "down"), null);
  assert.deepEqual(editBody(2, BOARD, "e3", "drop"), { revision: 2, drop: "e3" });
  assert.deepEqual(successors(BOARD, "e4"), ["e6", "e7", "e8"]);
  assert.deepEqual(editBody(2, BOARD, "e4", "drop"), { revision: 2, drop: "e4", with_successors: true });
});
