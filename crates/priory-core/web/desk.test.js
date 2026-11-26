import { test } from "node:test";
import assert from "node:assert/strict";
import { nextLink, onRoom, sinceGroups, mergeSince, progressView, sectionInView, dueText, rowAction, actionNote, batchParts, countWords, memoryNote, memoryTag, capsuleWords, countHead, deskBanners, autopilotView, arrivalAsk, plural, taskRepo } from "./desk.js";
import { linked } from "./links.js";

const why = "work stopped and waits on a person";

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
  assert.deepEqual(batchParts("Ship Next: exports: 1 merged. Next: task 36 starts when task 34 merges."), {
    line: "Ship Next: exports: 1 merged.",
    next: "Next: task 36 starts when task 34 merges.",
  });
});

test("Up next says a queued task starts on room only while it waits for its place, not for its own reason", () => {
  for (const why of ["first in the queue", "a band above it", "queue position 4"]) assert.equal(onRoom({ kind: "queued", why }), true, why);
  assert.equal(onRoom({ kind: "queued", why: "waits for its claim to reach the log" }), false);
  assert.equal(onRoom({ kind: "held", why: "held until 2026-11-22T10:00:00Z" }), false);
});

// A `memories` desk item's candidates as `/api/desk` sends them.
const remember = { id: "c-1", name: "release-day", forget: false, text: "Wednesdays.", replaces: true, current_text: "Tuesdays." };
const fresh = { id: "c-2", name: "search-staging", forget: false, text: "Use staging-b.", replaces: false, current_text: null };
const tombstone = { id: "c-3", name: "kit-fact", forget: true, text: null, replaces: true, current_text: "In git." };

test("a memory candidate's tag says whether it replaces or forgets a memory runs get", () => {
  assert.deepEqual(
    [remember, fresh, tombstone, { ...tombstone, replaces: false }, { ...fresh, replaces: null }].map(memoryTag),
    [
      { label: "replaces a memory runs get", warn: true },
      { label: "new", warn: false },
      { label: "forgets a memory runs get", warn: true },
      { label: "forget", warn: false },
      // Unknown where the kit is not checked out: never "new".
      { label: "may replace a memory runs get", warn: true },
    ],
  );
});

test("an accept or a drop says what happened, or why it was not sent", () => {
  const ok = { ok: "release-day accepted: every runner applies it as it reads the log" };
  assert.equal(memoryNote("accept", 200, ok), "Release-day accepted: every runner applies it as it reads the log, then it leaves this list.");
  assert.equal(memoryNote("drop", 409, { error: "candidate c-1 is accepted" }), "Not sent: candidate c-1 is accepted. The list reads it again.");
  assert.equal(memoryNote("drop", 404, { error: "no memory candidate c-9" }), "Not sent: no memory candidate c-9. The list reads it again.");
  assert.equal(memoryNote("accept", 503, { error: "the log is unreachable since 10:00: try again when it is back. Nothing was sent: try again" }), "Could not send: the log is unreachable since 10:00: try again when it is back. Nothing was sent: try again.");
  assert.equal(memoryNote("accept", 0, { error: "the connection dropped" }), "The server may have sent this but did not confirm it. Check the list before you send again.");
});

test("the Desk's count line names every item Needs me counts besides the asks", () => {
  const none = { stopped: 0, todos: 0, reports: 0, other: 0 };
  assert.equal(countWords({ ...none, oldest: "5 min" }), "Oldest waits 5 min.");
  assert.equal(countWords({ ...none, oldest: "3 h", stopped: 1, other: 2 }), "Oldest waits 3 h. Also 1 stopped and 2 waiting.");
  assert.equal(countWords({ oldest: null, stopped: 0, todos: 2, reports: 1, other: 0 }), "Also 2 to-dos and 1 report.");
  assert.equal(countWords({ oldest: null, stopped: 2, todos: 1, reports: 3, other: 1 }), "Also 2 stopped, 1 to-do, 3 reports and 1 waiting.");
  assert.equal(countWords({ ...none, oldest: null }), "");
});

test("the count shows a number only when an ask needs you, so an empty desk stays quiet", () => {
  assert.deepEqual(countHead(0), { n: null, words: "No asks need you" });
  assert.deepEqual(countHead(1), { n: 1, words: "ask needs you" });
  assert.deepEqual(countHead(7), { n: 7, words: "asks need you" });
});

test("the capsule answers while live, and only reads the asks while offline", () => {
  assert.deepEqual(capsuleWords(7, false), { text: "Answer 7 asks", icon: "ask", off: false });
  assert.deepEqual(capsuleWords(1, false), { text: "Answer 1 ask", icon: "ask", off: false });
  assert.deepEqual(capsuleWords(7, true), { text: "Read 7 asks, offline", icon: "unplug", off: true });
});

test("a wide Desk opens its oldest ask on arrival, and stays empty once that drawer closes", () => {
  const asks = [{ id: 7 }, { id: 3 }];
  assert.equal(arrivalAsk({ from: null, list: "desk", view: "needs", asks }), 7);
  assert.equal(arrivalAsk({ from: "/tasks/4", list: "desk", view: "needs", asks }), 7);
  assert.equal(arrivalAsk({ from: "/asks/9", list: "asks", view: "needs", asks }), 7);
  assert.equal(arrivalAsk({ from: "/asks/7", list: "desk", view: "needs", asks }), null);
  assert.equal(arrivalAsk({ from: "/", list: "desk", view: "needs", asks }), null);
  assert.equal(arrivalAsk({ from: "/?view=running", list: "desk", view: "needs", asks }), null);
  assert.equal(arrivalAsk({ from: "/tasks", list: "desk", view: "running", asks }), null);
  assert.equal(arrivalAsk({ from: null, list: "desk", view: "needs", asks: [] }), null);
});

test("the desk's banners say why runs wait: the pause first, then room, intake, reclaim and each cap reached", () => {
  const cap = "spend cap reached: factory spent $50.10 of its day cap of $50.00; its new runs wait for the next day";
  const lane = "spend cap reached: lane core/intake spent $12.40 of its day cap of $10.00; its new runs wait for the next day";
  const reclaim = "reclaimed 14.2 GB of build output from 2 folders in the last 24 hours, last at 2026-11-23T19:00:00Z";
  const ago = () => "1 h ago";
  assert.deepEqual(deskBanners({ paused: "paused since 2026-11-22T21:40:00Z: freeze", banner: "waits for room", intake: "the intake fails", reclaim, caps: [cap, lane] }, ago), [
    { text: "paused 1 h ago: freeze", icon: "pause", intake: false },
    { text: "waits for room", icon: "hard-drive", intake: false },
    { text: "the intake fails", icon: "unplug", intake: true },
    { text: reclaim, icon: "hard-drive", intake: false },
    { text: cap, icon: "hourglass", intake: false },
    { text: lane, icon: "hourglass", intake: false },
  ]);
  assert.deepEqual(deskBanners({ paused: null, banner: null, intake: null, caps: [] }, ago), []);
  // A desk cached before the fields existed has neither.
  assert.deepEqual(deskBanners({ banner: "waits for room" }, ago), [{ text: "waits for room", icon: "hard-drive", intake: false }]);
});

test("the stall and pause banners say when in the page's relative words, not a raw time", () => {
  const ago = (/** @type {string} */ iso) => ({ "2026-11-22T14:34:03Z": "12 min ago", "2026-11-20T08:00:00Z": "on Nov 20, 08:00" })[iso];
  const desk = {
    paused: "paused since 2026-11-20T08:00:00Z: freeze since 2026-11-19T00:00:00Z",
    banner: "stalled since 2026-11-22T14:34:03Z: short of memory by 2 GB; task 4 waits for room",
  };
  assert.deepEqual(deskBanners(desk, ago).map((banner) => banner.text), [
    "paused on Nov 20, 08:00: freeze since 2026-11-19T00:00:00Z",
    "stalled 12 min ago: short of memory by 2 GB; task 4 waits for room",
  ]);
  // A pause without a reason ends at its time.
  assert.equal(deskBanners({ paused: "paused since 2026-11-22T14:34:03Z" }, ago)[0].text, "paused 12 min ago");
});

test("a count names its noun in the singular for one, else in the plural", () => {
  assert.equal(plural(1, "task"), "1 task");
  assert.equal(plural(0, "task"), "0 tasks");
  assert.equal(plural(2, "entry", "entries"), "2 entries");
});

test("a stopped row's #n links in its task's repository, and a row with no task links no #n", () => {
  const tasks = [{ id: 4, repo: "acme/shop" }, { id: 5, repo: null }];
  const rule = [{ pattern: String.raw`\B#(\d+)\b`, url: "https://github.com/{repo}/issues/$1" }];

  assert.equal(taskRepo(tasks, 4), "acme/shop");
  assert.equal(taskRepo(tasks, 5), null);
  assert.equal(taskRepo(tasks, null), null);
  assert.equal(taskRepo(undefined, 4), null);
  assert.equal(
    linked("checks red on #12", rule, taskRepo(tasks, 4)),
    `checks red on <a class="autolink" href="https://github.com/acme/shop/issues/12" rel="noopener noreferrer" target="_blank">#12</a>`,
  );
  assert.equal(linked("checks red on #12", rule, taskRepo(tasks, 5)), "checks red on #12");
});

test("the Desk's model sorts its items, and an open ask the asks listing lacks keeps the desk's words", async () => {
  const { deskModel } = await import("./desk.js");
  const desk = {
    items: [
      { kind: "ask", id: 1, what: "Open it?" }, { kind: "ask", id: 2, what: "Ship it?" }, { kind: "blocked", id: 3, task: 3 },
      { kind: "command", id: 4 }, { kind: "todo", id: 5 }, { kind: "report", id: 6 }, { kind: "held", id: 7 }, { kind: "queue", id: 8 },
    ],
    ranked: [{ kind: "blocked", id: 3 }, { kind: "ask", id: 2 }], next: { kind: "ask", id: 2 },
    counts: { asleep: 2 }, paused: "paused since 2026-11-25T10:00:00Z", intake: null,
  };
  const queue = { asleep: [{ id: 9 }], running: [], waiting: [{ id: 10 }], up_next: [{ kind: "queued" }] };
  const model = deskModel({ desk, asks: [{ id: 2, question: "Ship it now?", options: ["yes"] }], queue, tasks: [] }, () => "3 min ago");
  assert.deepEqual(model.open, [{ kind: "ask", id: 1, what: "Open it?", question: "Open it?", options: [] }, { id: 2, question: "Ship it now?", options: ["yes"] }]);
  assert.equal(model.first.id, 2);
  assert.deepEqual([model.stopped, model.todos, model.reports, model.other].map((items) => items.map((item) => item.id)), [[3, 4], [5], [6], [7]]);
  assert.equal(model.needs, 7);
  assert.deepEqual(model.banners, [{ text: "paused 3 min ago", icon: "pause", intake: false }]);
  assert.deepEqual([model.asleep, model.asleepTasks, model.queue, model.upNext], [2, queue.asleep, queue.waiting, queue.up_next]);
});

const PILOT = { window: 4, since: "2026-11-25T22:00:00Z", until: "2026-11-26T08:00:00Z", level: "high", lanes: [], answered: 3, left: 1 };
const TIMES = { clock: () => "08:00", ago: () => "4 min ago" };
const IDLE = { window: null, sending: false, untold: null };

test("the autopilot banner says until when, at what level and lanes, and what it did, with Turn off", () => {
  assert.deepEqual(autopilotView(PILOT, IDLE, null, TIMES), {
    words: "Autopilot is on until 08:00 (high, every lane). It answered 3 and left 1.", label: "Turn off", busy: false, error: false,
  });
  assert.equal(autopilotView({ ...PILOT, lanes: ["core/intake", "web"] }, IDLE, null, TIMES).words,
    "Autopilot is on until 08:00 (high, core/intake, web). It answered 3 and left 1.");
  assert.equal(autopilotView(null, IDLE, null, TIMES), null, "no banner while autopilot is off");
});

test("the autopilot banner waits while its off is in flight, then goes once the runners were told", () => {
  assert.deepEqual(autopilotView(PILOT, { window: null, sending: true, untold: null }, null, TIMES),
    { words: "Autopilot is on until 08:00 (high, every lane). It answered 3 and left 1.", label: "Turning off", busy: true, error: false });
  const told = { window: 4, sending: false, untold: null };
  assert.equal(autopilotView(PILOT, told, null, TIMES), null, "the window it ended hides before the desk is read again");
  assert.equal(autopilotView(null, told, null, TIMES), null);
  assert.equal(autopilotView({ ...PILOT, window: 5 }, told, null, TIMES).label, "Turn off", "a later window shows again");
});

test("an off that told no other runner keeps the banner, as an error, with Tell them again", () => {
  const untold = { window: 4, sending: false, untold: "the log is unreachable." };
  const view = { words: "Autopilot is off here, and no other runner was told: the log is unreachable.", label: "Tell them again", busy: false, error: true };
  assert.deepEqual(autopilotView(PILOT, untold, null, TIMES), view);
  assert.deepEqual(autopilotView(null, untold, null, TIMES), view);
  assert.equal(autopilotView({ ...PILOT, window: 5 }, untold, null, TIMES).label, "Turn off", "a later window replaces the error");
});

test("offline, the autopilot banner says what the last read showed and how old it is, and cannot turn it off", () => {
  assert.deepEqual(autopilotView(PILOT, IDLE, "2026-11-26T07:00:00Z", TIMES), {
    words: "Autopilot was on 4 min ago, until 08:00 (high, every lane).", label: "Turn off", busy: true, error: false,
  });
});
