import { test } from "node:test";
import assert from "node:assert/strict";
import { droppedNote, droppedWhy, emptyDetail, eventWords, handled, requestedWorkflow, sinceWords, unheardActs, unheardNote, unroutedNote, routeAction, neverOwned } from "./inbox.js";

const event = (fields) => ({ seq: 4, type: "github.pull_request.comment", subject: "o/r#7", time: "2026-11-21T09:00:00Z", task: 1, outcome: "kept", prompt: null, wait: null, ...fields });

test("an inbox event says what it did, as priory task prints it", () => {
  assert.equal(handled(event({ prompt: "confirmed" }), 1), "kept, delivered as prompt");
  assert.equal(handled(event({ prompt: "queued" }), 1), "kept, still queued");
  assert.equal(handled(event({}), 1), "kept, for its next run");
  assert.equal(handled(event({ outcome: "woke", prompt: "confirmed", wait: 3 }), 1), "woke, delivered as prompt, fired wait 3");
  assert.equal(handled(event({ outcome: "coalesced", type: "factory.task.requested" }), 1), "coalesced");
  assert.equal(handled(event({ task: 2, outcome: "kept", prompt: null, wait: 5 }), 1), "kept, fired wait 5 (routed to task 2)");
  assert.equal(handled(event({ task: 2 }), 1, (id) => (id === 2 ? "ENG-9" : "")), "kept (routed to ENG-9)");
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

test("Request a task prefills the task's workflow only when the factory lets a person request it", () => {
  assert.equal(requestedWorkflow("implement", ["implement", "review"]), "implement");
  assert.equal(requestedWorkflow("bonjour", ["implement", "review"]), undefined);
  assert.equal(requestedWorkflow("bonjour", []), undefined);
  assert.equal(requestedWorkflow("bonjour", null), "bonjour");
});

test("the desk's unheard note is calm, and empty with none", () => {
  assert.equal(unheardNote(0), "");
  assert.equal(unheardNote(undefined), "");
  assert.equal(unheardNote(1), "1 event kept by a sleeping task reached no agent");
  assert.equal(unheardNote(4), "4 events kept by a sleeping task reached no agent");
});

test("an unheard group's actions follow its task: say to an asleep one, say or resume a blocked one, route an ended one", () => {
  assert.deepEqual(unheardActs({ state: "asleep" }), ["say"]);
  assert.deepEqual(unheardActs({ state: "blocked" }), ["say", "resume"]);
  assert.deepEqual(unheardActs({ state: "done" }), ["request"]);
  assert.deepEqual(unheardActs({ state: "failed" }), ["revive"]);
  assert.deepEqual(unheardActs({ state: "dropped" }), ["revive"]);
  assert.deepEqual(unheardActs({ state: "ready" }), []);
  assert.deepEqual(unheardActs({ state: "queued" }), []);
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
  assert.equal(emptyDetail(["inbox"]), "Open an event's task to read what the event did.");
  assert.equal(emptyDetail(["inbox", "terminal"]), "Open an event's task to read what the event did.");
  assert.equal(emptyDetail(["desk"]), "Select an ask to read it here.");
  assert.equal(emptyDetail(["inbox", "asks"]), "Select an ask to read it here.");
});

test("a Since you looked line names its event type in plain words and keeps its subject", () => {
  assert.equal(sinceWords("factory.run.scored implement"), "run scored implement");
  assert.equal(sinceWords("github.pull_request_review.submitted acme/shop#245: kept"), "GitHub pull request review submitted acme/shop#245: kept");
  assert.equal(sinceWords("linear.issue.updated"), "Linear issue updated");
  assert.equal(sinceWords("ENG-224: Morning report is ready"), "ENG-224: Morning report is ready");
});

test("the desk's calm line counts the requests a pre script dropped, and says nothing with none", () => {
  assert.equal(droppedNote(0), "");
  assert.equal(droppedNote(undefined), "");
  assert.equal(droppedNote(1), "1 request a pre script dropped in the last 24 hours");
  assert.equal(droppedNote(3), "3 requests a pre script dropped in the last 24 hours");
});

test("a dropped request says why, or how its script failed, and what judges it again", () => {
  const again = " A new event of it that a trigger wants runs the script again.";
  assert.equal(droppedWhy({ reason: "no estimate yet", failed: null }), `Dropped: no estimate yet.${again}`);
  assert.equal(droppedWhy({ reason: null, failed: "exit status 2" }), `Dropped: the script failed: exit status 2.${again}`);
  assert.equal(droppedWhy({ reason: null, failed: null }), `Dropped with no reason.${again}`);
});
