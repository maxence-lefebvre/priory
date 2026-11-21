import { test } from "node:test";
import assert from "node:assert/strict";
import { nextLink, sinceGroups, mergeSince, progressView, sectionInView, dueText, rowAction, actionNote, batchParts } from "./desk.js";

const why = "an alarm: work stopped and waits on a person";

test("the next thing opens the page that answers it", () => {
  assert.equal(nextLink({ kind: "ask", id: 42, task: 7 }), "/asks/42");
  assert.equal(nextLink({ kind: "ask", id: "deploy-check", task: null }), "/asks/deploy-check");
  for (const kind of ["blocked", "failed", "prompt", "command"]) assert.equal(nextLink({ kind, id: 3, task: 7, why }), "/tasks/7");
  assert.equal(nextLink({ kind: "report", id: 12, task: null, what: "intake: github: 502 Bad Gateway", why }), "#intake");
  assert.equal(nextLink({ kind: "report", id: 13, task: null, what: "backup: done" }), null);
  assert.equal(nextLink(null), null);
});

// `since_you_looked` as `/api/desk` sends it: newest first.
const events = [
  { time: "2026-11-21T09:40:00Z", task: 7, key: "github:acme/app#7", what: "factory.step.ended review approved" },
  { time: "2026-11-21T09:30:00Z", task: null, key: null, what: "backup: done" },
  { time: "2026-11-21T09:20:00Z", task: 9, key: "cli:2026-11-21-1", what: "factory.step.ended plan done" },
  { time: "2026-11-21T09:10:00Z", task: 7, key: "github:acme/app#7", what: "factory.step.ended implement done" },
];

test("since you looked groups the events by task, the task with the newest event first", () => {
  assert.deepEqual(sinceGroups(events).map((group) => [group.task, group.key, group.events.map((event) => event.what)]), [
    [7, "github:acme/app#7", ["factory.step.ended review approved", "factory.step.ended implement done"]],
    [null, null, ["backup: done"]],
    [9, "cli:2026-11-21-1", ["factory.step.ended plan done"]],
  ]);
  assert.deepEqual(sinceGroups([]), []);
});

test("since you looked keeps the events it opened on after the visit is recorded, and adds newer ones", () => {
  const opened = events.slice(1);
  const afterSeen = [events[0], events[1]];
  assert.deepEqual(mergeSince(opened, afterSeen), events);
  assert.deepEqual(mergeSince([], events), events);
});

test("a task's progress shows its goal, each criterion as required or optional, its latest change and next action", () => {
  const progress = {
    goal: "Ship the export",
    acceptanceCriteria: [{ id: "AC1", description: "CSV opens in Excel", required: true }, { id: "AC2", description: "A progress bar", required: false }],
    latestChange: "Added the CSV writer",
    nextAction: "Wait for CI",
  };
  assert.deepEqual(progressView(progress), {
    goal: "Ship the export",
    criteria: [{ id: "AC1", description: "CSV opens in Excel", state: "Required" }, { id: "AC2", description: "A progress bar", state: "Optional" }],
    latestChange: "Added the CSV writer",
    nextAction: "Wait for CI",
  });
  assert.deepEqual(progressView({ nextAction: "Wait for CI" }), { goal: null, criteria: [], latestChange: null, nextAction: "Wait for CI" });
  assert.equal(progressView(null), null);
  assert.equal(progressView({}), null);
});

test("the chip in view is the last section whose top passed under the chip bar, or the last one at the end", () => {
  // Section tops in the scroller, a 844 px tall view of 4000 px, and a 48 px chip bar.
  const at = (scrollTop) => sectionInView({ tops: [0, 900, 1500, 2600], scrollTop, height: 844, scrollHeight: 4000, bar: 48 });
  assert.equal(at(0), 0);
  assert.equal(at(851), 0);
  assert.equal(at(852), 1);
  // Scrolled to the third section: the second's tail is still on screen, the third is lit.
  assert.equal(at(1452), 2);
  assert.equal(at(2000), 2);
  assert.equal(at(3156), 3);
  assert.equal(sectionInView({ tops: [0, 300, 500], scrollTop: 200, height: 600, scrollHeight: 800, bar: 48 }), 2);
  assert.equal(sectionInView({ tops: [], scrollTop: 0, height: 600, scrollHeight: 600, bar: 48 }), -1);
});

test("a to-do's due time reads in local time with a relative hint, and other text stays as it is", () => {
  process.env.TZ = "America/New_York";
  const now = Date.parse("2026-11-20T12:00:00Z");
  const due = (iso) => dueText(`Renew the TLS cert, due ${iso}`, now);
  assert.equal(due("2026-11-20T20:00:00Z"), "Renew the TLS cert, due today 15:00");
  assert.equal(due("2026-11-21T14:00:00Z"), "Renew the TLS cert, due tomorrow 09:00");
  assert.equal(due("2026-11-19T14:00:00Z"), "Renew the TLS cert, due yesterday 09:00");
  assert.equal(due("2026-12-01T14:00:00Z"), "Renew the TLS cert, due Dec 1, 09:00 (in 11 days)");
  assert.equal(due("2026-11-17T14:00:00Z"), "Renew the TLS cert, due Nov 17, 09:00 (3 days ago)");
  assert.equal(dueText("Renew the cert", now), "Renew the cert");
  assert.equal(dueText("Fix it, due soon", now), "Fix it, due soon");
  assert.equal(dueText("Fix it, due 2026-13-20T20:00:00Z", now), "Fix it, due 2026-13-20T20:00:00Z");
});

test("a report row reads and a held task's row releases at the revision the page drew, other rows have no action", () => {
  assert.deepEqual(rowAction({ kind: "report", id: 12, task: null, what: "backup: done" }, 40), { label: "Read", path: "/api/inbox/12/read", body: {} });
  assert.deepEqual(rowAction({ kind: "held", id: 3, task: 7, what: "held until 2026-11-21T09:00:00Z" }, 40), { label: "Release", path: "/api/tasks/7/release", body: { revision: 40 } });
  for (const kind of ["blocked", "failed", "ask", "todo", "sweep"]) assert.equal(rowAction({ kind, id: 1, task: 7 }, 40), null);
});

test("a refused row action says why, and a stale revision says to read again", () => {
  assert.equal(actionNote("Release", 409, {}), "The state changed since this page loaded. Read the task again, then tap again.");
  assert.equal(actionNote("Read", 422, { error: "inbox 5 is not a report" }), "Could not read it: inbox 5 is not a report.");
  assert.equal(actionNote("Release", 500, {}), "Could not release it: the server answered 500.");
  assert.match(actionNote("Read", 503, {}), /no priory daemon runs beside this server/);
  assert.match(actionNote("Read", 504, {}), /may have applied this/);
  assert.match(actionNote("Read", 0, {}), /may have applied this/);
});

test("a batch line splits into its counts and what starts next, or keeps a line with no next whole", () => {
  assert.deepEqual(batchParts("Invoice exports: 1 merged, 2 planned. Next: task 36 starts when task 34 merges."), {
    line: "Invoice exports: 1 merged, 2 planned.",
    next: "Next: task 36 starts when task 34 merges.",
  });
  assert.deepEqual(batchParts("Invoice exports: 2 started."), { line: "Invoice exports: 2 started.", next: null });
});
