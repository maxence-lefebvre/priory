import { test } from "node:test";
import assert from "node:assert/strict";
import { backTo, pageTitle } from "./pages.js";

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
  const data = { tasks: [{ id: 7, key: "ENG-224" }], asks: [{ id: 3 }] };
  assert.equal(pageTitle("/", data), "Desk · Priory");
  assert.equal(pageTitle("/index.html", data), "Desk · Priory");
  assert.equal(pageTitle("/queue", data), "Queue · Priory");
  assert.equal(pageTitle("/workflow", data), "Flows · Priory");
  assert.equal(pageTitle("/tasks/7", data), "ENG-224 · Priory");
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
