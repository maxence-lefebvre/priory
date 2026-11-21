import { test } from "node:test";
import assert from "node:assert/strict";
import { answeredBy, askGroups, routedMark, routedNote } from "./asks.js";

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

test("an open ask routed to a lead is marked with the lead, and an unrouted or answered ask is not", () => {
  assert.equal(routedMark(ask(1, { routed_to: "lead" })), "with lead");
  assert.equal(routedMark(ask(2, { routed_to: null })), null);
  assert.equal(routedMark(ask(3, { routed_to: "lead", state: "answered" })), null);
  assert.equal(routedMark({ kind: "ask", routed_to: "lead" }), "with lead");
});

test("the detail says the lead is answering and that the first answer stands", () => {
  assert.equal(routedNote(ask(1, { routed_to: "lead" })), "lead is answering. You can still answer first: the first answer stands.");
  assert.equal(routedNote(ask(2)), null);
});

test("answeredBy names a person for the CLI and the server, the lead as agent:<lead>, and nobody when unrecorded", () => {
  assert.deepEqual(["cli", "serve", "agent:lead", null].map(answeredBy), ["a person", "a person", "agent:lead", null]);
});
