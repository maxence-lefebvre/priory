import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { context, draw, find, herdr, press, settle, text } from "./kit.js";

process.env.TZ = "UTC";
const { default: Inbox } = await import("../mods/priory-inbox/main.js");

// `priory inbox --unrouted --json` with the groups of crates/priory/tests/serve.rs's unrouted test:
// a dropped, a done and an asleep owner, and one Linear event no task ever owned. Then a blocked and
// an asleep task's unheard events, as its unheard test serves them.
const fixture = JSON.parse(readFileSync(new URL("./fixtures/unrouted.json", import.meta.url), "utf8"));

/** The Inbox panel after its first read of `read`. */
async function inbox(read = fixture) {
  herdr.reset();
  herdr.replies.inbox = read;
  const view = new Inbox();
  const cx = context();
  view.init({}, cx);
  await settle();
  return { view, cx, drawn: () => draw(view, cx) };
}

test("reads the unrouted groups and shows each with its owner, its events and the line for the rest", async () => {
  const { drawn } = await inbox();

  assert.deepEqual(herdr.argv[0], ["inbox", "--unrouted", "--json"]);
  const shown = text(drawn());
  assert.match(shown, /Unrouted 3/);
  assert.match(shown, /github:owner\/repo#8 1 task 12 dropped github:owner\/repo#8 GitHub issue comment created owner\/repo#8 Nov 17, 11:30 Still broken on mobile\./);
  assert.match(shown, /No task ever owned: 1 linear event\./);
});

/** Presses a group's route button with `verb` answering `reply`, and the other verbs `replies`; returns the panel and the verb's argv. */
async function route(/** @type {string} */ resource, /** @type {string} */ verb, /** @type {string} */ reply, /** @type {Record<string, any>} */ replies = {}) {
  const panel = await inbox();
  Object.assign(herdr.replies, replies, { [verb]: reply });
  press(panel.drawn(), `route-${resource}`);
  await settle();
  return { ...panel, argv: herdr.argv.filter((args) => args[0] === verb) };
}

test("an active owner subscribes on the revision the panel read, and says where the next event goes", async () => {
  const { drawn, argv } = await route("linear:ENG-7", "subscribe", "linear:ENG-7");

  assert.deepEqual(argv, [["subscribe", "13", "linear:ENG-7", "--expected", "41"]]);
  assert.match(text(drawn()), /Its next event goes to task 13\./);
});

test("a dropped owner revives", async () => {
  const { drawn, argv } = await route("github:owner/repo#8", "revive", "");

  assert.deepEqual(argv, [["revive", "12"]]);
  assert.match(text(drawn()), /Revived task 12\./);
});

test("a done owner's resource requests a new task with the group's text and the owner's workflow", async () => {
  const { drawn, argv } = await route("github:owner/repo#7", "add", "github:owner/repo#7");

  assert.deepEqual(argv, [["add", "--workflow", "implement", "--", "Follow up on https://github.com/owner/repo/issues/7 after event 30"]]);
  assert.match(text(drawn()), /Requested "?github:owner\/repo#7"?\. It owns/);
});

test("a done owner whose workflow is not requestable requests the factory's default workflow instead, and says so", async () => {
  const { drawn, argv } = await route("github:owner/repo#7", "add", "github:owner/repo#7", { queue: { workflows: ["review"], waiting: [] } });

  assert.deepEqual(argv, [["add", "--", "Follow up on https://github.com/owner/repo/issues/7 after event 30"]]);
  assert.match(text(drawn()), /Workflow implement is not requestable, so it starts the factory's default\./);
});

test("an older priory's empty workflows keep the done owner's", async () => {
  const { argv } = await route("github:owner/repo#7", "add", "github:owner/repo#7", { queue: { workflows: [], waiting: [] } });

  assert.deepEqual(argv, [["add", "--workflow", "implement", "--", "Follow up on https://github.com/owner/repo/issues/7 after event 30"]]);
});

test("a done owner whose workflow is requestable keeps it", async () => {
  const { argv } = await route("github:owner/repo#7", "add", "github:owner/repo#7", { queue: { workflows: ["implement"], waiting: [] } });

  assert.deepEqual(argv, [["add", "--workflow", "implement", "--", "Follow up on https://github.com/owner/repo/issues/7 after event 30"]]);
});

test("after an action, the route buttons wait for a read that shows its result", async () => {
  const { drawn } = await route("linear:ENG-7", "subscribe", "linear:ENG-7");
  assert.equal(find(drawn(), "route-github:owner/repo#8").calls.disabled[0], true);

  herdr.wake();
  await settle();

  assert.equal(find(drawn(), "route-github:owner/repo#8").calls.disabled[0], false);
});

test("a refused action says why on its group", async () => {
  const { drawn } = await inbox();

  press(drawn(), "route-linear:ENG-7");
  await settle();

  assert.match(text(drawn()), /Could not route it: priory subscribe: no such thing\./);
  assert.equal(find(drawn(), "route-linear:ENG-7").calls.disabled[0], false);
});

test("pressing a group's owner opens it in the Task panel", async () => {
  const { drawn } = await inbox();

  press(drawn(), "open-github:owner/repo#7");

  assert.equal(/** @type {any} */ (herdr.shared.get("priory.task.open")).task, 11);
});

test("lists the unheard groups first, an asleep task's with the command that tells it to read them", async () => {
  const { drawn } = await inbox();

  const shown = text(drawn());
  assert.match(shown, /Unheard no agent read them linear:ENG-24 1 task 15 blocked/);
  assert.ok(shown.indexOf("github:owner/repo#9") < shown.indexOf("github:owner/repo#8"), shown);
  assert.match(shown, /task 14 asleep github:owner\/repo#9 GitHub pull request comment owner\/repo#9 Nov 17, 11:40 Cap the backoff at 10 minutes\. priory say 14 'Read the events that came while you slept\.'/);
  assert.match(shown, /Unrouted reached no task github:owner\/repo#8/);
});

test("with no owned group, the unheard groups still show above the empty line", async () => {
  const { drawn } = await inbox({ ...fixture, unrouted: { ...fixture.unrouted, owned: [] } });

  assert.match(text(drawn()), /Unheard no agent read them linear:ENG-24 .* Unrouted reached no task No event of a task's resource went unrouted\./);
});

test("a blocked task's unheard group resumes it, since it reads them when it resumes", async () => {
  const panel = await inbox();
  herdr.replies.resume = "";

  assert.match(text(panel.drawn()), /Resume task 15 It reads them when it resumes\./);
  press(panel.drawn(), "unheard-15");
  await settle();

  assert.deepEqual(herdr.argv.filter((args) => args[0] === "resume"), [["resume", "15"]]);
  assert.match(text(panel.drawn()), /Resumed task 15\./);
});

/** A request `scripts/has-estimate` dropped, as `unrouted.dropped` lists it. */
const drop = {
  seq: 40, time: "2026-11-17T12:00:01Z", trigger: "linear-view", script: "scripts/has-estimate", reason: "no estimate yet",
  failed: null, key: "linear:ENG-12",
  event: { seq: 39, type: "linear.issue.entered", subject: "ENG-12", time: "2026-11-17T12:00:00Z", line: "Speed up the export", url: null },
};

test("lists the requests a pre script dropped between the unheard and the unrouted groups, with why", async () => {
  const { drawn } = await inbox({ ...fixture, unrouted: { ...fixture.unrouted, dropped: [drop] } });

  const shown = text(drawn());
  assert.match(shown, /Dropped a script judged each once linear:ENG-12 1 trigger linear-view, scripts\/has-estimate Linear issue entered ENG-12 Nov 17, 12:00 Speed up the export Dropped: no estimate yet\. A new event of it that a trigger wants runs the script again\. Unrouted reached no task/);
  assert.ok(shown.indexOf("Unheard") < shown.indexOf("Dropped a script"), shown);
});

test("with no owned and no unheard group, a dropped request still shows above the empty line", async () => {
  const { drawn } = await inbox({ ...fixture, unrouted: { ...fixture.unrouted, owned: [], unheard: [], dropped: [drop] } });

  assert.match(text(drawn()), /^Inbox Unrouted 0 Dropped a script judged each once linear:ENG-12 .* Unrouted reached no task No event of a task's resource went unrouted\./);
});

test("the view model is the web view's inbox.js, copied", () => {
  const source = (path) => readFileSync(new URL(path, import.meta.url), "utf8");
  assert.equal(source("../mods/priory-inbox/view.js"), source("../../../crates/priory-core/web/inbox.js"),
    "cp crates/priory-core/web/inbox.js adapters/herdr/mods/priory-inbox/view.js");
});
