import { test } from "node:test";
import assert from "node:assert/strict";
import { emptyDetail, eventWords, handled, unroutedNote, routeAction, neverOwned } from "./inbox.js";

const event = (fields) => ({ seq: 4, type: "github.pull_request.comment", subject: "o/r#7", time: "2026-11-21T09:00:00Z", task: 1, outcome: "kept", prompt: null, wait: null, ...fields });

test("an inbox event says what it did, as priory task prints it", () => {
  assert.equal(handled(event({ prompt: "confirmed" }), 1), "kept, delivered as prompt");
  assert.equal(handled(event({ prompt: "queued" }), 1), "kept, still queued");
  assert.equal(handled(event({}), 1), "kept, for its next run");
  assert.equal(handled(event({ outcome: "woke", prompt: "confirmed", wait: 3 }), 1), "woke, delivered as prompt, fired wait 3");
  assert.equal(handled(event({ outcome: "coalesced", type: "factory.task.requested" }), 1), "coalesced");
  assert.equal(handled(event({ task: 2, outcome: "kept", prompt: null, wait: 5 }), 1), "kept, fired wait 5 (routed to task 2)");
  assert.equal(handled(event({ prompt: "withdrawn" }), 1), "kept, prompt withdrawn");
});

test("the desk's unrouted note is calm, and empty with none", () => {
  assert.equal(unroutedNote(0), "");
  assert.equal(unroutedNote(1), "1 event reached no task in the last 24 hours");
  assert.equal(unroutedNote(3), "3 events reached no task in the last 24 hours");
});

test("an owned group's action follows its owner, as priory inbox --unrouted prints it", () => {
  assert.equal(routeAction({ state: "dropped" }), "revive");
  assert.equal(routeAction({ state: "failed" }), "revive");
  assert.equal(routeAction({ state: "done" }), "request");
  assert.equal(routeAction({ state: "asleep" }), "subscribe");
});

test("events no task ever owned read as one count per source", () => {
  assert.equal(neverOwned([]), "");
  assert.equal(neverOwned([{ source: "github", events: 2 }, { source: "linear", events: 1 }]), "No task ever owned: 2 github events, 1 linear event.");
});

test("an event's type reads in plain words", () => {
  assert.equal(eventWords("linear.issue.updated"), "Linear issue updated");
  assert.equal(eventWords("github.pull_request_review.submitted"), "GitHub pull request review submitted");
  assert.equal(eventWords("slack.message.posted"), "Slack message posted");
  assert.equal(eventWords("factory.task.requested"), "task requested");
  assert.equal(eventWords("custom.thing_done"), "custom thing done");
});

test("the empty detail pane beside the Inbox names an event, not an ask", () => {
  assert.equal(emptyDetail("inbox"), "Open an event's task to read what the event did.");
  assert.equal(emptyDetail("desk"), "Select an ask to read it here.");
  assert.equal(emptyDetail("asks"), "Select an ask to read it here.");
});
