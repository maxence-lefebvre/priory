import { test } from "node:test";
import assert from "node:assert/strict";
import { backTo, pageTitle, taskRead } from "./pages.js";

test("a detail page's back link names the list the person came from", () => {
  assert.deepEqual(backTo("/"), ["/", "Desk"]);
  assert.deepEqual(backTo("/?view=queue"), ["/?view=queue", "Desk"]);
  assert.deepEqual(backTo("/queue?view=held"), ["/queue?view=held", "Queue"]);
  assert.deepEqual(backTo("/tasks?view=blocked"), ["/tasks?view=blocked", "Tasks"]);
  assert.deepEqual(backTo("/inbox"), ["/inbox", "Inbox"]);
  assert.deepEqual(backTo("/index.html"), ["/index.html", "Desk"]);
});

test("a detail page opened first goes back to Tasks", () => {
  assert.deepEqual(backTo(null), ["/tasks", "Tasks"]);
  assert.deepEqual(backTo("/more"), ["/tasks", "Tasks"]);
});

test("each page has its own title, a task by its key", () => {
  const data = { tasks: [{ id: 7, key: "ENG-224" }, { id: 9, key: "linear:ENG-245" }], asks: [{ id: 3 }] };
  assert.equal(pageTitle("/", data), "Desk · Priory");
  assert.equal(pageTitle("/index.html", data), "Desk · Priory");
  assert.equal(pageTitle("/queue", data), "Queue · Priory");
  assert.equal(pageTitle("/workflow", data), "Flows · Priory");
  assert.equal(pageTitle("/tasks/7", data), "ENG-224 · Priory");
  assert.equal(pageTitle("/tasks/9", data), "ENG-245 · Priory");
  assert.equal(pageTitle("/tasks/8", data), "Task 8 · Priory");
  assert.equal(pageTitle("/asks/3", data), "Ask #3 · Priory");
  assert.equal(pageTitle("/batches/2", data), "Batch 2 · Priory");
  assert.equal(pageTitle("/memories", data), "Memory candidates · Priory");
  assert.equal(pageTitle("/settings/appearance", data), "Appearance · Priory");
  assert.equal(pageTitle("/settings/notifications", data), "Notifications · Priory");
  assert.equal(pageTitle("/more", data), "More · Priory");
  assert.equal(pageTitle("/terminals", data), "Page not found · Priory");
});

test("a title names the page the address renders", () => {
  const data = { tasks: [] };
  assert.equal(pageTitle("/tasks/nope", data), "Tasks · Priory");
  assert.equal(pageTitle("/batches/nope", data), "Page not found · Priory");
  assert.equal(pageTitle("/settings", data), "Appearance · Priory");
  assert.equal(pageTitle("/settings/other", data), "Appearance · Priory");
});

test("a refresh keeps the open ask's task drawn while it is read again, and the new body replaces it", () => {
  const first = taskRead(null, 16);
  assert.deepEqual(first, { task: { id: 16, body: null, error: null, stale: false }, read: true });
  const body = { id: 16, brief: "Brief", inbox: [], trace: [] };
  const kept = { ...first.task, body };
  assert.deepEqual(taskRead(kept, 16), { task: kept, read: false }, "an unchanged snapshot reads nothing");
  // A refresh with a changed snapshot marks the read stale.
  const again = taskRead({ ...kept, stale: true }, 16);
  assert.equal(again.read, true);
  assert.equal(again.task.body, body, "the drawer keeps its Brief, Inbox and Trace until the new body lands");
  assert.equal(again.task.stale, false, "the next repaint reads nothing more");
});

test("a task page keeps its body, so its Say box stays drawn through a refresh", () => {
  const body = { id: 2269, key: "FND-2269", state: "asleep" };
  const { task } = taskRead({ id: 2269, body, stale: true, error: null }, 2269);
  assert.ok(task.body, "the page never falls back to Loading task while it re-reads");
});

test("another ask or task page starts empty", () => {
  const { task, read } = taskRead({ id: 16, body: { id: 16 }, stale: false }, 17);
  assert.equal(read, true);
  assert.equal(task.body, null);
});

test("an ask's title survives an address that does not decode", () => {
  assert.equal(pageTitle("/asks/%E0%A4%A", { tasks: [] }), "Ask #%E0%A4%A · Priory");
});
