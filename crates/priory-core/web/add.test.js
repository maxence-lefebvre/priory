import { test } from "node:test";
import assert from "node:assert/strict";
import { addBody, addOutcome } from "./add.js";

test("the request sends the trimmed text and only the chips that hold a value", () => {
  assert.deepEqual(addBody({ text: "  Fix the docs \n", workflow: "implement", band: "", lane: null }), { text: "Fix the docs", workflow: "implement" });
  assert.equal(addBody({ text: "   ", workflow: "implement" }), null);
});

test("a reply becomes the sheet's state: the new task, a refusal, or an unsure send that kept the draft", () => {
  assert.deepEqual(addOutcome(200, { key: "cli:2026-11-21-1", id: 7 }), { state: "added", key: "cli:2026-11-21-1", id: 7 });
  assert.deepEqual(addOutcome(422, { error: "unknown workflow `nope`" }), { state: "refused", error: "unknown workflow `nope`" });
  assert.deepEqual(addOutcome(503, { error: "no daemon" }), { state: "refused", error: "no priory daemon runs beside this server. Start priory daemon, then add again" });
  assert.deepEqual(addOutcome(500, {}), { state: "refused", error: "the server answered 500" });
  assert.deepEqual(addOutcome(0, {}), { state: "unsure" });
  assert.deepEqual(addOutcome(504, {}), { state: "unsure" });
  assert.deepEqual(addOutcome(401, {}), { state: "signed-out" });
});
