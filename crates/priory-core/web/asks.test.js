import { test } from "node:test";
import assert from "node:assert/strict";
import { askGroups } from "./asks.js";

const now = Date.parse("2026-11-20T12:00:00Z");
const ask = (id, fields) => ({ id, state: "open", snoozed_until: null, ...fields });

test("an ask snoozed past now leaves Open and its count, and an expired snooze does not", () => {
  const asks = [
    ask(1),
    ask(2, { snoozed_until: "2026-11-20T13:00:00Z" }),
    ask(3, { snoozed_until: "2026-11-20T11:00:00Z" }),
    ask(4, { state: "answered", answered_at: "2026-11-20T10:00:00Z" }),
    ask(5, { state: "withdrawn" }),
  ];

  const groups = askGroups(asks, now);

  assert.deepEqual(groups.open.map((each) => each.id), [1, 3]);
  assert.deepEqual(groups.snoozed.map((each) => each.id), [2]);
  assert.deepEqual(groups.answered.map((each) => each.id), [4]);
  assert.deepEqual(groups.withdrawn.map((each) => each.id), [5]);
});
