import { test } from "node:test";
import assert from "node:assert/strict";
import { ASK_VIEWS, answeredBy, askGroups, routedMark, routedNote, runnerOf, submitted, seenAgo, sentNote, shownGroups, snoozeShortcut, stillArmed, takesText, openAsksOf, writeNote, shrunkHead, answeredAsks, confidenceNote } from "./asks.js";

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

test("Open and Snoozed list their asks oldest first, as the Desk does", () => {
  const asks = [
    ask(1, { since: "2026-11-20T11:20:00Z" }),
    ask(2, { since: "2026-11-20T10:00:00+01:00" }),
    ask(3, { since: "2026-11-20T11:47:00Z" }),
    ask(4, { since: "2026-11-20T11:00:00Z", snoozed_until: "2026-11-20T13:00:00Z" }),
    ask(5, { since: "2026-11-20T09:00:00Z", snoozed_until: "2026-11-20T14:00:00Z" }),
  ];

  const groups = askGroups(asks, now);

  assert.deepEqual(groups.open.map((each) => each.id), [2, 1, 3]);
  assert.deepEqual(groups.snoozed.map((each) => each.id), [5, 4]);
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

test("answeredBy names a person for the CLI and the server, the lead as agent:<lead>, auto, a session by its name, and nobody when unrecorded", () => {
  assert.deepEqual(
    ["cli", "serve", "agent:lead", "auto", "session:t9 8f3c2a10", null].map(answeredBy),
    ["a person", "a person", "agent:lead", "auto", "session t9 8f3c2a10", null],
  );
});

test("an armed command option runs on the second tap only at the revision and within the window it was armed in", () => {
  const asked = { id: 12, revision: 3 };
  const armed = { id: 12, revision: 3, text: "approve", at: now };

  assert.equal(stillArmed(armed, asked, "approve", now + 1000), true);
  assert.equal(stillArmed(armed, { ...asked, revision: 4 }, "approve", now + 1000), false);
  assert.equal(stillArmed(armed, asked, "approve", now + 4000), false);
  assert.equal(stillArmed(armed, asked, "reject", now + 1000), false);
  assert.equal(stillArmed(null, asked, "approve", now), false);
});

test("each saved view lists its own groups: Open leaves snoozed asks out and Snoozed lists only them", () => {
  assert.deepEqual(ASK_VIEWS, ["open", "snoozed", "answered", "all"]);
  assert.deepEqual(shownGroups("open"), ["open"]);
  assert.deepEqual(shownGroups("snoozed"), ["snoozed"]);
  assert.deepEqual(shownGroups("answered"), ["answered"]);
  assert.deepEqual(shownGroups("all"), ["open", "snoozed", "answered", "withdrawn"]);
});

test("the S key snoozes an open ask of a task for 1 h, and nothing else", () => {
  const open = ask(1, { task: 7 });
  assert.deepEqual(snoozeShortcut(open, now), { kind: "ask", id: 1, for: "1h" });
  assert.equal(snoozeShortcut(ask(2, { task: null }), now), null);
  assert.equal(snoozeShortcut(ask(3, { task: 7, snoozed_until: "2026-11-20T13:00:00Z" }), now), null);
  assert.deepEqual(snoozeShortcut(ask(4, { task: 7, snoozed_until: "2026-11-20T11:00:00Z" }), now), { kind: "ask", id: 4, for: "1h" });
  assert.equal(snoozeShortcut(ask(5, { task: 7, state: "answered" }), now), null);
  assert.equal(snoozeShortcut(undefined, now), null);
});

test("a refused ask write says why, and a 504 or a dropped connection says to check the ask before sending again", () => {
  const ago = () => "5 min";
  const fresh = (fields) => ({ ask: { id: 1, revision: 3, ...fields } });
  assert.equal(writeNote(409, fresh({ state: "open" }), ago), "The question changed. Read revision 3 and answer again.");
  assert.equal(writeNote(409, fresh({ state: "answered", answer: "left", answered_at: "x" }), ago), "Already answered 5 min ago: left.");
  assert.equal(writeNote(409, fresh({ state: "withdrawn", withdrawn_why: "done" }), ago), "Withdrawn: done.");
  assert.match(writeNote(503, {}, ago), /no priory daemon runs beside this server/);
  for (const status of [504, 0]) assert.equal(writeNote(status, {}, ago), "The daemon may have applied this but did not confirm it. Check the ask's state before you send again.");
  assert.equal(writeNote(422, { error: "no such option" }, ago), "Could not send: no such option.");
  assert.equal(writeNote(500, {}, ago), "Could not send: the server answered 500.");
});

test("a step's ask takes only its options, so it offers no text and no dismissal; an agent's ask does", () => {
  assert.equal(takesText(ask(1, { free_text: false })), false);
  assert.equal(takesText(ask(2, { free_text: true })), true);
  assert.equal(takesText(ask(3)), true);
});

test("another runner's ask names its runner, and this runner's ask names none", () => {
  assert.equal(runnerOf(ask("desktop:12", { runner: "desktop", last_seen: null, submissions: [] })), "desktop");
  assert.equal(runnerOf(ask(7, { runner: "workstation" })), null);
  assert.equal(runnerOf(ask(8)), null);
});

test("another runner's ask says when its runner was last seen once that is over 3 minutes ago", () => {
  const ago = (iso) => `${Math.round((now - Date.parse(iso)) / 60000)} min`;
  const remote = (last_seen) => ask("desktop:12", { runner: "desktop", last_seen, submissions: [] });
  assert.equal(seenAgo(remote("2026-11-20T11:20:00Z"), now, ago), "last seen 40 min ago");
  assert.equal(seenAgo(remote("2026-11-20T11:58:00Z"), now, ago), null);
  assert.equal(seenAgo(remote(null), now, ago), "never seen");
  assert.equal(seenAgo(ask(7, { runner: "workstation" }), now, ago), null);
});

test("another runner's ask says what became of its latest submission until its runner applies it", () => {
  const sent = (submission) => ask("laptop:3", { runner: "laptop", last_seen: null, submissions: [{ verb: "answer", text: "rejected", outcome: "refused", why: "stale" }, submission] });
  const pending = { verb: "answer", text: "approved", outcome: null, why: null };
  assert.deepEqual(sentNote(sent(pending)), { text: "Sent to laptop: approved", refused: false });
  assert.deepEqual(sentNote(sent({ ...pending, verb: "dismiss", text: null })), { text: "Dismiss sent to laptop", refused: false });
  assert.deepEqual(sentNote(sent({ ...pending, outcome: "refused", why: "ask 3 is answered" })), { text: "laptop refused approved: ask 3 is answered", refused: true });
  assert.deepEqual(sentNote(sent({ ...pending, verb: "dismiss", text: null, outcome: "refused", why: "ask 3 is answered" })), { text: "laptop refused the dismiss: ask 3 is answered", refused: true });
  assert.equal(sentNote(sent({ ...pending, outcome: "answered" })), null);
  assert.equal(sentNote(ask("laptop:4", { runner: "laptop", submissions: [] })), null);
  assert.equal(sentNote(ask(7)), null);
});

test("a write to another runner's ask that could not reach the log says nothing was sent, in the server's words", () => {
  const remote = ask("desktop:12", { runner: "desktop", submissions: [] });
  const body = { error: "the log is unreachable: connection refused. Nothing was sent: try again" };
  assert.equal(writeNote(503, body, () => "", remote), "Could not send: the log is unreachable: connection refused. Nothing was sent: try again.");
  assert.match(writeNote(503, body, () => "", ask(7)), /no priory daemon runs beside this server/);
});

test("the S key snoozes no ask of another runner, whose task is a task of that runner", () => {
  assert.equal(snoozeShortcut(ask("desktop:12", { task: 4, runner: "desktop", submissions: [] }), now), null);
});

test("each answer and dismiss sent to another runner's ask is a history row with its runner, its outcome and when it was sent", () => {
  const remote = ask("laptop:3", { runner: "laptop", submissions: [
    { submission: "s1", verb: "answer", revision: 1, text: "rejected", sent: "2026-11-20T11:00:00Z", outcome: "refused", why: "ask 3 is at revision 2", applied: "2026-11-20T11:01:00Z" },
    { submission: "s2", verb: "answer", revision: 2, text: "approved", sent: "2026-11-20T11:10:00Z", outcome: "answered", why: null, applied: "2026-11-20T11:11:00Z" },
    { submission: "s3", verb: "dismiss", revision: 2, text: null, sent: "2026-11-20T11:20:00Z", outcome: "dismissed", why: null, applied: "2026-11-20T11:21:00Z" },
    { submission: "s4", verb: "answer", revision: 2, text: "merge", sent: "2026-11-20T11:30:00Z", outcome: null, why: null, applied: null },
  ] });
  assert.deepEqual(submitted(remote), [
    { tile: "refused", text: "Sent rejected to laptop for revision 1. laptop refused it: ask 3 is at revision 2", at: "2026-11-20T11:00:00Z" },
    { tile: "answered", text: "Sent approved to laptop for revision 2. laptop applied it", at: "2026-11-20T11:10:00Z" },
    { tile: "withdrawn", text: "Sent the dismiss to laptop for revision 2. laptop applied it", at: "2026-11-20T11:20:00Z" },
    { tile: "queued", text: "Sent merge to laptop for revision 2. Not applied yet", at: "2026-11-20T11:30:00Z" },
  ]);
  assert.deepEqual(submitted(ask(7)), []);
});

test("a task page lists its task's open asks, snoozed ones too, oldest first", () => {
  const asks = [
    { id: 4, task: 7, state: "open", since: "2026-11-20T11:00:00Z" },
    { id: 3, task: 7, state: "open", since: "2026-11-20T09:00:00Z", snoozed_until: "2026-11-21T09:00:00Z" },
    { id: 2, task: 7, state: "answered", since: "2026-11-20T08:00:00Z" },
    { id: 1, task: 8, state: "open", since: "2026-11-20T07:00:00Z" },
  ];
  assert.deepEqual(openAsksOf(asks, { id: 7 }).map((ask) => ask.id), [3, 4]);
  assert.deepEqual(openAsksOf(asks, { id: 9 }), []);
});

test("a task page lists the open ask its task's held result shares with another task", () => {
  const asks = [{ id: "gate-1-1", task: 1, state: "open", since: "2026-11-20T09:00:00Z" }];
  assert.deepEqual(openAsksOf(asks, { id: 2, asks: [{ id: "gate-1-1" }] }).map((ask) => ask.id), ["gate-1-1"]);
});

test("a task page leaves out another runner's ask on a task of the same id", () => {
  const asks = [
    { id: 5, task: 4, runner: "desk", state: "open", since: "2026-11-20T10:00:00Z" },
    { id: "laptop:2", task: 4, runner: "laptop", state: "open", since: "2026-11-20T09:00:00Z", submissions: [] },
  ];
  assert.deepEqual(openAsksOf(asks, { id: 4 }).map((ask) => ask.id), [5]);
});

test("the shrunk header names the question over the task key, step and revision", () => {
  assert.deepEqual(shrunkHead({ key: "linear:ENG-212", step: "approve-pr", revision: 2, title: "Open the pull request", question: "Open the pull request for ENG-212?" }), { question: "Open the pull request for ENG-212?", place: "ENG-212, approve-pr, revision 2" });
  assert.deepEqual(shrunkHead({ key: null, step: "deploy", revision: 1, title: "Deploy", question: null }), { question: "Deploy", place: "deploy, revision 1" });
});

// Three answered asks of task 4 as `/api/tasks/4` lists them, oldest first, and one of task 5.
const answered = [
  { id: 41, task: 4, state: "answered", title: "Keep the full response for clients without a cursor", question: "When a client sends no cursor, should GET /orders return every order, as today, or the first page?",
    answer: "every order, as today", recommendation: "every order, as today", answered_by: "auto", answered_at: "2026-11-20T11:22:00Z", confidence: "high",
    rationale: "The brief asks for it in so many words.", tradeoff: "Large merchants keep timing out." },
  { id: 42, task: 4, state: "answered", title: "Name the cursor parameter", question: "after or cursor?", answer: "after", recommendation: "after",
    answered_by: "session:7f3c2a", answered_at: "2026-11-20T11:00:00Z", confidence: "medium", rationale: "The invoices endpoint already uses after.", tradeoff: null },
  { id: 43, task: 4, state: "answered", title: null, question: "Pick the default page size", answer: "40", recommendation: "50",
    answered_by: "serve", answered_at: "2026-11-20T10:00:00Z", confidence: null, rationale: null, tradeoff: null },
  { id: 44, task: 4, state: "open", question: "Choose the pagination cursor", since: "2026-11-20T11:20:00Z" },
  { id: 45, task: 5, state: "answered", question: "Other task", answer: "yes", answered_by: "cli", answered_at: "2026-11-20T11:50:00Z" },
];

test("a task page lists its task's answered asks, newest answer first, and leaves out open ones and other tasks'", () => {
  assert.deepEqual(answeredAsks([...answered].reverse(), 4).map((row) => row.id), [41, 42, 43]);
  assert.deepEqual(answeredAsks(answered, 9), []);
  assert.deepEqual(answeredAsks([{ ...answered[0], runner: "laptop", submissions: [] }], 4), []);
});

test("each answered ask says who answered: auto, a session by its name, an agent, or a person", () => {
  const rows = answeredAsks([...answered, { ...answered[2], id: 46, answered_by: "agent:lead", answered_at: "2026-11-20T09:00:00Z" },
    { ...answered[2], id: 47, answered_by: null, answered_at: "2026-11-20T08:00:00Z" }], 4);
  assert.deepEqual(rows.map((row) => [row.id, row.answerer, row.by]), [
    [41, "auto", "auto"], [42, "session", "session 7f3c2a"], [43, "person", "a person"], [46, "agent", "agent:lead"], [47, null, null],
  ]);
});

test("an answered ask carries its answer, confidence as bars, rationale, trade-off, question, and the recommendation only when another option won", () => {
  const [auto, session, person] = answeredAsks(answered, 4);
  assert.deepEqual(auto, {
    id: 41, title: "Keep the full response for clients without a cursor", question: answered[0].question, answer: "every order, as today",
    at: "2026-11-20T11:22:00Z", answerer: "auto", by: "auto", confidence: "high", bars: 3,
    rationale: "The brief asks for it in so many words.", tradeoff: "Large merchants keep timing out.", recommended: null,
  });
  assert.deepEqual([session.title, session.bars, session.rationale], ["Name the cursor parameter", 2, "The invoices endpoint already uses after."]);
  assert.deepEqual([person.title, person.confidence, person.bars, person.recommended], ["Pick the default page size", null, 0, "50"]);
});

test("an auto answer's confidence says it is the agent's own, and the level the task answers at", () => {
  assert.equal(confidenceNote("high"), "confidence, the agent's own. The task answers at high and above.");
  assert.equal(confidenceNote("medium"), "confidence, the agent's own. The task answers at medium and above.");
  assert.equal(confidenceNote(null), "confidence, the agent's own.");
});
