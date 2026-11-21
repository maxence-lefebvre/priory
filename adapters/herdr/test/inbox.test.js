import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { context, draw, herdr, press, settle, text } from "./kit.js";

process.env.TZ = "UTC";
const { default: Inbox } = await import("../mods/priory-inbox/main.js");

// `priory inbox --unrouted --json` with the groups of crates/priory/tests/serve.rs's unrouted test:
// a dropped, a done and an asleep owner, and one Linear event no task ever owned.
const fixture = JSON.parse(readFileSync(new URL("./fixtures/unrouted.json", import.meta.url), "utf8"));

/** The Inbox panel after its first read. */
async function inbox() {
  herdr.reset();
  herdr.replies.inbox = fixture;
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
  assert.match(shown, /github:owner\/repo#8 1 task 12 dropped github:owner\/repo#8 github\.issue_comment\.created owner\/repo#8 Nov 17, 11:30 Still broken on mobile\./);
  assert.match(shown, /No task ever owned: 1 linear event\./);
});

test("each group routes its next event by its owner's state, with the CLI's verb, and says what it did", async () => {
  const { drawn } = await inbox();
  herdr.replies.subscribe = "linear:ENG-7";
  herdr.replies.revive = "";
  herdr.replies.add = "github:owner/repo#7";

  press(drawn(), "route-linear:ENG-7");
  await settle();
  press(drawn(), "route-github:owner/repo#8");
  await settle();
  press(drawn(), "route-github:owner/repo#7");
  await settle();

  assert.deepEqual(herdr.argv.slice(1).filter((args) => args[0] !== "wait"), [
    ["subscribe", "13", "linear:ENG-7"],
    ["revive", "12"],
    ["add", "--workflow", "implement", "--", "Follow up on https://github.com/owner/repo/issues/7 after event 30"],
  ]);
  const shown = text(drawn());
  assert.match(shown, /Its next event goes to task 13\./);
  assert.match(shown, /Revived task 12\./);
  assert.match(shown, /Requested "?github:owner\/repo#7"?\. It owns/);
});

test("a refused action says why on its group", async () => {
  const { drawn } = await inbox();

  press(drawn(), "route-linear:ENG-7");
  await settle();

  assert.match(text(drawn()), /Could not route it: priory subscribe: no such thing\./);
});

test("pressing a group's owner opens it in the Task panel", async () => {
  const { drawn } = await inbox();

  press(drawn(), "open-github:owner/repo#7");

  assert.equal(/** @type {any} */ (herdr.shared.get("priory.task.open")).task, 11);
});

test("the view model is the web view's inbox.js, copied", () => {
  const source = (path) => readFileSync(new URL(path, import.meta.url), "utf8");
  assert.equal(source("../mods/priory-inbox/view.js"), source("../../../crates/priory-core/web/inbox.js"),
    "cp crates/priory-core/web/inbox.js adapters/herdr/mods/priory-inbox/view.js");
});
